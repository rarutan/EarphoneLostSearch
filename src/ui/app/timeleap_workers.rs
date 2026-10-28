use super::UiApp;
use crate::errors::{AppError, StateIssue};
use crate::ui::i18n::Texts;
use eframe::egui;
use std::time::{Duration, Instant};

impl UiApp {
    pub(super) fn request_field_timeleap_candidate_search(&mut self, ctx: &egui::Context) {
        if !self
            .field
            .timeleap_lifecycle_policy()
            .candidate_search_enabled
        {
            return;
        }
        if self.jobs.field_timeleap_candidates.is_running() {
            return;
        }
        self.launch_field_timeleap_candidate_search(ctx);
    }

    pub(super) fn launch_field_timeleap_candidate_search(&mut self, ctx: &egui::Context) {
        if self.jobs.field_timeleap_candidates.is_running() {
            return;
        }
        if !self
            .field
            .timeleap_lifecycle_policy()
            .candidate_search_enabled
        {
            self.field.clear_timeleap_candidates();
            return;
        }
        if !self
            .field
            .timeleap_candidate_search_inputs_ready(self.state.fps_value())
        {
            self.field.clear_timeleap_candidates();
            return;
        }
        let (key, cached) = match self
            .field
            .prepare_timeleap_candidate_search(self.state.fps_value())
        {
            Ok(request) => request,
            Err(message) => {
                self.field.fail_timeleap_candidate_search();
                self.field.set_search_range_notice(message);
                return;
            }
        };
        self.field.clear_notice();
        self.field.begin_timeleap_candidate_search();
        self.field.set_timeleap_candidate_search_key(key.clone());
        let repaint = ctx.clone();
        let fps = self.state.fps_value();
        self.jobs
            .field_timeleap_candidates
            .spawn_cancellable(move |cancel| {
                let cache_result = cached.map_or_else(
                    || {
                        crate::domain::rng::generate_observation_cache_cancelable(
                            key.config.seed,
                            0..=key.config.start_frame,
                            key.config.start_frame,
                            fps,
                            Some(cancel.as_ref()),
                        )
                    },
                    Ok,
                );
                let result = match cache_result {
                    Ok(cache) => crate::domain::timeleap::run_timeleap_candidate_search(
                        &key,
                        &cache,
                        cancel.as_ref(),
                    ),
                    Err(crate::errors::AppError::Cancelled) => {
                        Err(crate::domain::timeleap::TimeleapSearchFailure::Cancelled)
                    }
                    Err(error) => Err(crate::domain::timeleap::TimeleapSearchFailure::Cache(error)),
                };
                repaint.request_repaint();
                (key, result)
            });
    }

    pub(super) fn poll_field_timeleap_candidates(&mut self, ctx: &egui::Context) {
        if self.jobs.field_timeleap_candidates.is_running() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        let Some((cancelled, joined)) = self.jobs.field_timeleap_candidates.take_finished() else {
            return;
        };
        if cancelled {
            self.field.cancel_timeleap_candidate_search();
            return;
        }
        let Ok((key, result)) = joined else {
            self.field.fail_timeleap_candidate_search();
            self.present_app_error(AppError::WorkerPanicked);
            return;
        };
        if !self.field.timeleap_search_key_matches(&key) {
            self.field.fail_timeleap_candidate_search();
            return;
        }

        match result {
            Ok(groups) => {
                self.field.apply_timeleap_candidate_groups(groups);
                self.field.clear_notice();
            }
            Err(crate::domain::timeleap::TimeleapSearchFailure::Cancelled) => {
                self.field.fail_timeleap_candidate_search();
            }
            Err(crate::domain::timeleap::TimeleapSearchFailure::Frame1Unreachable) => {
                self.field.fail_timeleap_candidate_search();
                self.field
                    .set_search_range_notice("入力したFrame1には通常Timelineで到達できません。");
            }
            Err(crate::domain::timeleap::TimeleapSearchFailure::GenerationFrameUnreachable) => {
                self.field.fail_timeleap_candidate_search();
                self.field
                    .set_search_range_notice("指定した生成開始Frameには到達できません。");
            }
            Err(crate::domain::timeleap::TimeleapSearchFailure::Cache(error)) => {
                self.field.fail_timeleap_candidate_search();
                if error.is_cache_io() {
                    self.present_app_error(error);
                } else {
                    self.field
                        .set_search_range_notice(Texts::new(self.language).app_error(&error));
                }
            }
            Err(crate::domain::timeleap::TimeleapSearchFailure::Calculation(message)) => {
                self.field.fail_timeleap_candidate_search();
                self.field.set_search_range_notice(message);
            }
        }
    }

    pub(super) fn start_field_timeleap_frame1_preparation(&mut self, ctx: &egui::Context) {
        if self.jobs.field_timeleap_frame1.is_running() {
            return;
        }
        let (config, frame1, cache, start) =
            match self.field.prepare_timeleap_frame1(self.state.fps_value()) {
                Ok(request) => request,
                Err(message) => {
                    self.field.set_search_range_notice(message);
                    return;
                }
            };
        let preview = self
            .field
            .take_matching_timeleap_frame1_preview(&config, frame1, &start);
        self.jobs.field_timeleap_frame1_preview.cancel_and_discard();
        self.jobs.field_timeleap_target_check.cancel_and_discard();
        self.field.begin_timeleap_frame1_preparation();
        if let Some(preparation) = preview {
            self.field
                .apply_timeleap_frame1_candidates(preparation, self.state.fps_value());
            self.start_prepared_timeleap_target(ctx);
            return;
        }
        let repaint = ctx.clone();
        self.jobs
            .field_timeleap_frame1
            .spawn_cancellable(move |cancel| {
                let result = crate::domain::timeleap::run_timeleap_frame1_preparation(
                    &config,
                    frame1,
                    &start,
                    &cache,
                    cancel.as_ref(),
                );
                repaint.request_repaint();
                result
            });
    }

    pub(super) fn start_prepared_timeleap_target(&mut self, ctx: &egui::Context) {
        let Some((candidate, mut snapshot)) = self.field.direct_encounter_target_snapshot() else {
            self.jobs.cancel_prepared_timeleap_target();
            return;
        };
        let Some(target) = self.field.target_consumption.trim().parse::<i64>().ok() else {
            self.jobs.cancel_prepared_timeleap_target();
            return;
        };
        let Some(seed) =
            u32::from_str_radix(self.field.seed.trim().trim_start_matches("0x"), 16).ok()
        else {
            self.jobs.cancel_prepared_timeleap_target();
            return;
        };
        let repaint = ctx.clone();
        self.jobs
            .queue_prepared_timeleap_target(Box::new(move |cancel| {
                let result = snapshot
                    .retarget_confirmed_result_from_cache(target, Some(cancel.as_ref()))
                    .map(|result| crate::ui::app_types::PreparedTimeleapTarget {
                        seed,
                        target,
                        candidate,
                        result,
                    });
                repaint.request_repaint();
                result
            }));
    }

    pub(super) fn poll_prepared_timeleap_target(&mut self, ctx: &egui::Context) {
        if self.jobs.poll_prepared_timeleap_target() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    pub(super) fn apply_prepared_timeleap_target(&mut self) -> bool {
        let prepared = self.jobs.prepared_timeleap_target.take();
        self.jobs.cancel_prepared_timeleap_target();
        let Some(prepared) = prepared else {
            return false;
        };
        if u32::from_str_radix(self.field.seed.trim().trim_start_matches("0x"), 16).ok()
            != Some(prepared.seed)
            || self.field.target_consumption.trim().parse::<i64>().ok() != Some(prepared.target)
        {
            return false;
        }
        self.field
            .apply_prepared_direct_target(&prepared.candidate, &prepared.result)
    }

    pub(super) fn start_field_timeleap_frame1_preview(&mut self, ctx: &egui::Context) {
        if !self.field.timeleap_enabled {
            self.jobs.field_timeleap_frame1_preview.cancel();
            self.jobs.field_timeleap_target_check.cancel_and_discard();
            self.field.reset_timeleap_frame1_preview_to_idle();
            return;
        }
        if self.field.results.len() != 1 {
            self.field.reset_timeleap_frame1_preview_to_idle();
            return;
        }
        if !self.field.timeleap_frame1_preview_is_pending() {
            return;
        }
        if self.jobs.field_timeleap_frame1_preview.is_running() {
            self.jobs.field_timeleap_frame1_preview.cancel();
            return;
        }
        let prepared_at = Instant::now();
        let (config, frame1, cache, start) =
            match self.field.prepare_timeleap_frame1(self.state.fps_value()) {
                Ok(request) => request,
                Err(message) => {
                    self.field.mark_timeleap_frame1_preview_failed();
                    self.field.set_search_range_notice(message);
                    return;
                }
            };
        let generation = self.field.timeleap_frame1_preview_generation();
        let repaint = ctx.clone();
        self.jobs
            .field_timeleap_frame1_preview
            .spawn_cancellable(move |cancel| {
                let (generation, completion) =
                    crate::ui::app_types::run_timeleap_frame1_preview(generation, || {
                        crate::domain::timeleap::run_timeleap_frame1_preparation(
                            &config,
                            frame1,
                            &start,
                            &cache,
                            cancel.as_ref(),
                        )
                    });
                repaint.request_repaint();
                (generation, config, frame1, start, prepared_at, completion)
            });
    }

    pub(super) fn ensure_field_timeleap_frame1_preview(&mut self, ctx: &egui::Context) {
        // Target reachability is a separate derived check. Refresh its target
        // without invalidating/restarting the Frame1-only preparation worker.
        self.field.refresh_timeleap_target_check_target();
        if self.field.reconcile_timeleap_frame1_preview()
            == crate::application::field::TimeleapFrame1PreviewReconciliation::Restart
        {
            self.jobs.field_timeleap_target_check.cancel();
        }
        if self.field.timeleap_frame1_preview_is_pending()
            && !self.jobs.field_timeleap_frame1_preview.is_running()
        {
            self.start_field_timeleap_frame1_preview(ctx);
        }
    }

    pub(super) fn poll_field_timeleap_frame1_preview(&mut self, ctx: &egui::Context) {
        let Some((cancelled, joined)) = self.jobs.field_timeleap_frame1_preview.take_finished()
        else {
            if self.field.timeleap_frame1_preview_is_pending()
                && !self.jobs.field_timeleap_frame1_preview.is_running()
            {
                self.start_field_timeleap_frame1_preview(ctx);
            }
            return;
        };
        if !cancelled {
            match joined {
                Ok((
                    generation,
                    config,
                    frame1,
                    start,
                    prepared_at,
                    crate::ui::app_types::TimeleapFrame1PreviewCompletion::Calculated(Ok(
                        preparation,
                    )),
                )) => {
                    self.field.set_timeleap_frame1_preview(
                        generation,
                        config,
                        frame1,
                        self.field.committed_target_frame(),
                        start,
                        prepared_at,
                        preparation.clone(),
                        self.state.fps_value(),
                    );
                    if self.field.timeleap_frame1_preview_generation() == generation
                        && self.field.has_timeleap_frame1_preview()
                    {
                        self.start_field_timeleap_target_check(ctx);
                    }
                }
                Ok((
                    generation,
                    _,
                    _,
                    _,
                    _,
                    crate::ui::app_types::TimeleapFrame1PreviewCompletion::Calculated(Err(error)),
                )) if generation == self.field.timeleap_frame1_preview_generation() => {
                    self.field
                        .clear_timeleap_target_check_for_frame1_generation(generation);
                    match error {
                        crate::domain::timeleap::TimeleapSearchFailure::Cancelled => {
                            self.field.reset_timeleap_frame1_preview_to_idle();
                        }
                        crate::domain::timeleap::TimeleapSearchFailure::Frame1Unreachable => {
                            self.field.set_timeleap_frame1_unreachable();
                        }
                        crate::domain::timeleap::TimeleapSearchFailure::GenerationFrameUnreachable => {
                            self.field.mark_timeleap_frame1_preview_failed();
                            self.field.set_search_range_notice("指定した生成開始Frameには到達できません。");
                        }
                        crate::domain::timeleap::TimeleapSearchFailure::Cache(error) => {
                            self.field.mark_timeleap_frame1_preview_failed();
                            if error.is_cache_io() {
                                self.present_app_error(error);
                            } else {
                                self.field
                                    .set_search_range_notice(Texts::new(self.language).app_error(&error));
                            }
                        }
                        crate::domain::timeleap::TimeleapSearchFailure::Calculation(message) => {
                            self.field.mark_timeleap_frame1_preview_failed();
                            self.field.set_search_range_notice(message);
                        }
                    }
                }
                Ok((
                    generation,
                    _,
                    _,
                    _,
                    _,
                    crate::ui::app_types::TimeleapFrame1PreviewCompletion::Panicked,
                )) if self
                    .field
                    .fail_timeleap_frame1_preview_generation(generation) =>
                {
                    self.present_app_error(AppError::WorkerPanicked);
                }
                Ok(_) => {}
                Err(()) => {
                    // No request identity is available here; do not mutate the
                    // current preview because this join failure may be stale.
                    self.present_app_error(AppError::WorkerPanicked);
                }
            }
        }
        if self.field.timeleap_frame1_preview_is_pending() {
            self.start_field_timeleap_frame1_preview(ctx);
        }
    }

    pub(super) fn start_field_timeleap_target_check(&mut self, ctx: &egui::Context) {
        let Some((generation, frame1, target_frame, preparation, cache)) =
            self.field.prepare_timeleap_target_check_request()
        else {
            return;
        };
        if self.jobs.field_timeleap_target_check.is_running() {
            return;
        }
        let repaint = ctx.clone();
        self.jobs
            .field_timeleap_target_check
            .spawn_cancellable(move |cancel| {
                let output = crate::ui::app_types::run_timeleap_target_check(
                    generation,
                    target_frame,
                    || {
                        crate::domain::timeleap::timeleap_target_reachable_after_frame1(
                            &preparation,
                            frame1,
                            target_frame,
                            &cache,
                            cancel.as_ref(),
                        )
                    },
                );
                repaint.request_repaint();
                output
            });
    }

    pub(super) fn poll_field_timeleap_target_check(&mut self, ctx: &egui::Context) {
        let Some((cancelled, joined)) = self.jobs.field_timeleap_target_check.take_finished()
        else {
            if self.jobs.field_timeleap_target_check.is_running() {
                ctx.request_repaint_after(Duration::from_millis(50));
            } else if self.field.timeleap_target_check_is_pending() {
                self.start_field_timeleap_target_check(ctx);
            }
            return;
        };
        if cancelled {
            if self.field.timeleap_target_check_is_pending() {
                self.start_field_timeleap_target_check(ctx);
            }
            return;
        }
        match joined {
            Ok((
                generation,
                target,
                crate::ui::app_types::TimeleapTargetCheckCompletion::Calculated(Ok(reachable)),
            )) => {
                self.field
                    .apply_timeleap_target_check(generation, target, reachable);
            }
            Ok((
                generation,
                target,
                crate::ui::app_types::TimeleapTargetCheckCompletion::Calculated(Err(error)),
            )) => {
                if !matches!(
                    error,
                    crate::domain::timeleap::TimeleapSearchFailure::Cancelled
                ) && self.field.fail_timeleap_target_check(generation, target)
                {
                    match error {
                        crate::domain::timeleap::TimeleapSearchFailure::Cache(error) => {
                            if error.is_cache_io() {
                                self.present_app_error(error);
                            } else {
                                self.field
                                    .set_search_range_notice(Texts::new(self.language).app_error(&error));
                            }
                        }
                        crate::domain::timeleap::TimeleapSearchFailure::Calculation(message) => {
                            self.field.set_search_range_notice(message);
                        }
                        crate::domain::timeleap::TimeleapSearchFailure::Cancelled
                        | crate::domain::timeleap::TimeleapSearchFailure::Frame1Unreachable
                        | crate::domain::timeleap::TimeleapSearchFailure::GenerationFrameUnreachable => {}
                    }
                }
            }
            Ok((
                generation,
                target,
                crate::ui::app_types::TimeleapTargetCheckCompletion::Panicked,
            )) => {
                if self.field.fail_timeleap_target_check(generation, target) {
                    self.present_app_error(AppError::WorkerPanicked);
                }
            }
            Err(()) => {
                // The request-keyed boundary above catches calculation panics. If the
                // thread still panics outside it, preserve pending state because the
                // joined task no longer carries an identity and may already be stale.
                self.present_app_error(AppError::WorkerPanicked);
            }
        }
        // If an older worker finished after the selected Frame1/Target changed,
        // the generation guard discards its result. Start the current snapshot now.
        if self.field.timeleap_target_check_is_pending() {
            self.start_field_timeleap_target_check(ctx);
        }
    }

    pub(super) fn poll_field_timeleap_frame1(&mut self, ctx: &egui::Context) {
        if self.jobs.field_timeleap_frame1.is_running() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        let Some((cancelled, joined)) = self.jobs.field_timeleap_frame1.take_finished() else {
            return;
        };
        if cancelled {
            self.field.abort_timeleap_frame1_preparation();
            return;
        }
        let Ok(result) = joined else {
            self.field.abort_timeleap_frame1_preparation();
            self.field.mark_timeleap_frame1_preview_failed();
            self.present_app_error(AppError::WorkerPanicked);
            return;
        };
        match result {
            Ok(preparation) => {
                self.field
                    .apply_timeleap_frame1_candidates(preparation, self.state.fps_value());
                self.start_prepared_timeleap_target(ctx);
            }
            Err(crate::domain::timeleap::TimeleapSearchFailure::Cancelled) => {
                self.field.abort_timeleap_frame1_preparation();
                self.field.reset_timeleap_frame1_preview_to_idle();
            }
            Err(crate::domain::timeleap::TimeleapSearchFailure::Frame1Unreachable) => {
                self.field.abort_timeleap_frame1_preparation();
                self.field.set_timeleap_frame1_unreachable();
                self.field
                    .set_search_range_notice("入力したFrame1には通常Timelineで到達できません。");
            }
            Err(crate::domain::timeleap::TimeleapSearchFailure::GenerationFrameUnreachable) => {
                self.field.abort_timeleap_frame1_preparation();
                self.field.mark_timeleap_frame1_preview_failed();
                self.field
                    .set_search_range_notice("指定した生成開始Frameには到達できません。");
            }
            Err(crate::domain::timeleap::TimeleapSearchFailure::Cache(error)) => {
                self.field.abort_timeleap_frame1_preparation();
                self.field.mark_timeleap_frame1_preview_failed();
                if error.is_cache_io() {
                    self.present_app_error(error);
                } else {
                    self.field
                        .set_search_range_notice(Texts::new(self.language).app_error(&error));
                }
            }
            Err(crate::domain::timeleap::TimeleapSearchFailure::Calculation(message)) => {
                self.field.abort_timeleap_frame1_preparation();
                self.field.mark_timeleap_frame1_preview_failed();
                self.field.set_search_range_notice(message);
            }
        }
    }

    pub(super) fn start_field_timeleap_reachability(&mut self, ctx: &egui::Context) {
        self.jobs.field_timeleap_reachability.cancel_and_discard();
        let (generation, request, cache) = match self.field.begin_timeleap_reachability_check() {
            Ok(request) => request,
            Err(message) => {
                self.field.timeleap_reachability_form.status =
                    crate::application::field::TimeleapReachabilityStatus::Failed(message);
                return;
            }
        };
        let repaint = ctx.clone();
        self.jobs
            .field_timeleap_reachability
            .spawn_cancellable(move |cancel| {
                let result = crate::domain::timeleap::run_timeleap_reachability_check(
                    &request,
                    &cache,
                    cancel.as_ref(),
                );
                repaint.request_repaint();
                (generation, result)
            });
    }

    pub(super) fn poll_field_timeleap_reachability(&mut self, ctx: &egui::Context) {
        if self.jobs.field_timeleap_reachability.is_running() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        let Some((cancelled, joined)) = self.jobs.field_timeleap_reachability.take_finished()
        else {
            return;
        };
        if cancelled {
            return;
        }
        let Ok((generation, result)) = joined else {
            self.field.timeleap_reachability_form.status =
                crate::application::field::TimeleapReachabilityStatus::Failed(
                    "到達可否計算を完了できませんでした。".into(),
                );
            self.present_app_error(AppError::WorkerPanicked);
            return;
        };
        if let Err(crate::domain::timeleap::TimeleapSearchFailure::Cache(error)) = &result {
            if error.is_cache_io() {
                self.present_app_error_ref(error);
            }
        }
        self.field
            .apply_timeleap_reachability_result(generation, result);
    }

    pub(super) fn start_field_confirmed_target_search(&mut self, ctx: &egui::Context) {
        if self.jobs.field_confirmed_target_search.is_running() {
            return;
        }
        let (generation, request, cached) = match self.field.begin_confirmed_target_search() {
            Ok(search) => search,
            Err(message) => {
                self.field.confirmed_target_search_form.status =
                    crate::application::field::ConfirmedTargetSearchStatus::Failed(message);
                return;
            }
        };
        let worker_request = request.clone();
        let repaint = ctx.clone();
        let fps = self.state.fps_value();
        self.jobs
            .field_confirmed_target_search
            .spawn_cancellable(move |cancel| {
                let cache_result = cached.map_or_else(
                    || {
                        let start = worker_request
                            .source_config
                            .as_ref()
                            .map(|config| config.start_frame)
                            .unwrap_or(worker_request.start.sfmt_position);
                        let end = worker_request
                            .baseline_target_frame
                            .max(worker_request.generation_start_frame)
                            .max(worker_request.confirmed_frame1)
                            .max(start);
                        crate::domain::rng::generate_observation_cache_cancelable(
                            worker_request.seed,
                            0..=end,
                            end,
                            fps,
                            Some(cancel.as_ref()),
                        )
                    },
                    Ok,
                );
                let result = match cache_result {
                    Ok(cache) => crate::domain::timeleap::run_confirmed_target_search(
                        &worker_request,
                        &cache,
                        cancel.as_ref(),
                    )
                    .map(|rows| {
                        crate::domain::timeleap::ConfirmedTargetSearchResult::from_rows(
                            rows,
                            worker_request.drift_search_generation_start_frame,
                        )
                    }),
                    Err(crate::errors::AppError::Cancelled) => {
                        Err(crate::domain::timeleap::TimeleapSearchFailure::Cancelled)
                    }
                    Err(error) => Err(crate::domain::timeleap::TimeleapSearchFailure::Cache(error)),
                };
                repaint.request_repaint();
                (generation, request, result)
            });
        self.jobs.field_confirmed_target_search_key = Some((
            self.field.confirmed_target_search_form.generation,
            self.field.confirmed_target_timeline_generation,
        ));
    }

    pub(super) fn poll_field_timeleap_shift_resolution(&mut self, ctx: &egui::Context) {
        if self.jobs.field_timeleap_shift_resolution.is_running() {
            ctx.request_repaint_after(Duration::from_millis(25));
        }
        let Some((cancelled, joined)) = self.jobs.field_timeleap_shift_resolution.take_finished()
        else {
            return;
        };
        if cancelled {
            self.field.cancel_timeleap_shift_resolution();
            return;
        }
        let (generation, survivors) = match joined {
            Ok(output) => output,
            Err(()) => {
                self.field.cancel_timeleap_shift_resolution();
                self.present_app_error(AppError::WorkerPanicked);
                return;
            }
        };
        let Some(summary) = survivors else {
            self.field.cancel_timeleap_shift_resolution();
            return;
        };
        let no_candidates_matched = summary.has_no_matches();
        let stage = self
            .field
            .apply_timeleap_shift_resolution(generation, summary);
        match stage {
            Some(crate::application::field::TimeleapStage::Succeeded)
                if self.field.adopt_timeleap_timeline() =>
            {
                self.field.prepare_target_research(self.state.fps_value());
                self.start_field_target_research(ctx);
                self.play_candidate_confirmed();
                if self.field.timer_kind() == crate::application::field::FieldTimerKind::Target {
                    self.field_timer_retry = true;
                    ctx.request_repaint_after(Duration::from_millis(50));
                } else {
                    match self.field.start_blink_timer(self.state.fps_value()) {
                        Ok(()) | Err(AppError::InvalidState(StateIssue::TimelineLimitReached)) => {
                            self.field_timer_retry = false;
                        }
                        Err(_) => {
                            self.field_timer_retry = true;
                            ctx.request_repaint_after(Duration::from_millis(50));
                        }
                    }
                }
            }
            Some(crate::application::field::TimeleapStage::Failed) if no_candidates_matched => {
                self.play_candidate_not_found();
            }
            _ => {}
        }
    }

    pub(super) fn poll_field_confirmed_target_search(&mut self, ctx: &egui::Context) {
        if self.jobs.field_confirmed_target_search.is_running()
            && self.jobs.field_confirmed_target_search_key
                != Some((
                    self.field.confirmed_target_search_form.generation,
                    self.field.confirmed_target_timeline_generation,
                ))
        {
            self.jobs.field_confirmed_target_search.cancel();
        }
        if self.jobs.field_confirmed_target_search.is_running() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        let Some((cancelled, joined)) = self.jobs.field_confirmed_target_search.take_finished()
        else {
            return;
        };
        self.jobs.field_confirmed_target_search_key = None;
        if cancelled {
            self.field.cancel_confirmed_target_search();
            return;
        }
        let Ok((generation, request, result)) = joined else {
            self.field
                .fail_confirmed_target_search("TargetFrame検索を完了できませんでした。".into());
            self.present_app_error(AppError::WorkerPanicked);
            return;
        };
        if let Err(crate::domain::timeleap::TimeleapSearchFailure::Cache(error)) = &result {
            if error.is_cache_io() {
                self.present_app_error_ref(error);
            }
        }
        let target_changed = self
            .field
            .apply_confirmed_target_search_result(generation, &request, result);
        if target_changed {
            self.field.prepare_target_research(self.state.fps_value());
            self.start_field_search(ctx);
        }
    }
}
