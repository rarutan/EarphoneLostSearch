use super::UiApp;
use crate::application::state::AppState;
use crate::errors::{AppError, StateIssue};
use eframe::egui;

impl UiApp {
    pub(super) fn start_guidance_generation(&mut self, ctx: &egui::Context) {
        let Some(request) = self.state.guidance_search_request() else {
            return;
        };
        let Some(cache) = self.state.observation_cache.clone() else {
            return;
        };
        let repaint = ctx.clone();
        self.jobs.guidance_generation.spawn(move || {
            let fastest_extra = request
                .fastest_ticks
                .unwrap_or(0)
                .max(0)
                .saturating_mul(request.models as i64)
                .saturating_add(request.models as i64 + 8);
            let furthest_start = request
                .observed_end
                .saturating_add(request.pre_rotom_consumption)
                .saturating_add(request.npc_initial_load)
                .saturating_add(request.offset.unsigned_abs().min(i64::MAX as u64) as i64 / 2)
                .saturating_add(fastest_extra);
            let required_end = furthest_start.max(request.observed_end);
            let results = cache
                .ensure_through(required_end.min(crate::domain::rng::MAX_SFMT_FRAME), None)
                .and_then(|()| AppState::calculate_guidance_batch_from_cache(request, &cache));
            repaint.request_repaint();
            (request, results)
        });
    }

    pub(super) fn poll_guidance_generation(&mut self) {
        let Some((_, Ok((request, results)))) = self.jobs.guidance_generation.take_finished()
        else {
            return;
        };
        // 設定を編集したら到達判定の対応表を消去し、別のSeed・Target・オフセットへ
        // 古い結果を適用しない。
        if self
            .state
            .guidance_search_request()
            .is_some_and(|current| current.same_configuration(request))
        {
            match results {
                Ok(results) => self.state.apply_guidance_batch_results(request, results),
                Err(error) => {
                    self.present_app_error(error);
                }
            }
        }
    }

    pub(super) fn maybe_start_normal_target_wait_fallback(
        &mut self,
        ctx: &egui::Context,
        guidance: Option<crate::application::state_types::HatchGuidance>,
    ) {
        if self.jobs.normal_target_wait.is_running()
            || self.state.target_dirty
            || self.state.fastest_close
            || self.state.production_preparation_pending
        {
            return;
        }
        let Some(guidance) = guidance else {
            return;
        };
        let Some(request) = self.state.normal_target_wait_fallback_request(
            guidance.current_frame,
            guidance.next_circle_frame,
        ) else {
            return;
        };
        if self.jobs.normal_target_wait_key == Some(request.key) {
            return;
        }
        let Some(cache) = self
            .state
            .observation_cache
            .clone()
            .filter(|cache| cache.seed == request.key.seed)
        else {
            return;
        };
        self.jobs.normal_target_wait_key = Some(request.key);
        let repaint = ctx.clone();
        self.jobs.normal_target_wait.spawn(move || {
            let mut profile = crate::application::hot_path_metrics::HotPathTimer::start(
                crate::application::hot_path_metrics::HotPath::TargetFallback,
            );
            let result = AppState::calculate_normal_target_wait_fallback(request, &cache);
            profile.set_items(1);
            repaint.request_repaint();
            (request, result)
        });
    }

    pub(super) fn poll_normal_target_wait_fallback(&mut self) {
        let Some((_, joined)) = self.jobs.normal_target_wait.take_finished() else {
            return;
        };
        let active_key = self.jobs.normal_target_wait_key.take();
        let Ok((request, result)) = joined else {
            self.present_app_error(AppError::WorkerPanicked);
            return;
        };
        if active_key != Some(request.key)
            || !self.state.normal_target_wait_fallback_is_current(request)
        {
            return;
        }
        match result {
            Ok(ticks) => {
                self.state.apply_normal_target_wait_fallback(request, ticks);
            }
            Err(error) => self.present_app_error(error),
        }
    }

    pub(super) fn maybe_start_candidate_reachability(&mut self, ctx: &egui::Context) {
        if self.jobs.candidate_reachability.is_running() {
            return;
        }
        let Some(key) = self.state.candidate_reachability_key() else {
            self.jobs.candidate_reachability_key = None;
            return;
        };
        if self
            .jobs
            .candidate_reachability_key
            .as_ref()
            .is_some_and(|current| *current == key)
        {
            return;
        }
        let Some(request) = self.state.candidate_reachability_request_for_key(key) else {
            return;
        };
        self.jobs.candidate_reachability_key = Some(key);
        let observation_cache = self.state.observation_cache.clone();
        let repaint = ctx.clone();
        self.jobs.candidate_reachability.spawn(move || {
            let results = observation_cache
                .as_ref()
                .ok_or(AppError::InvalidState(StateIssue::CacheUnavailable))
                .and_then(|cache| {
                    let max_candidate = request
                        .candidate_positions
                        .iter()
                        .map(|(_, frame)| *frame)
                        .max()
                        .unwrap_or(request.target);
                    let fastest_extra = request
                        .fastest_ticks
                        .unwrap_or(0)
                        .max(0)
                        .saturating_mul(request.models as i64)
                        .saturating_add(request.models as i64 + 8);
                    let required_end = request
                        .candidate_positions
                        .iter()
                        .map(|(_, frame)| *frame)
                        .max()
                        .unwrap_or(max_candidate)
                        .saturating_add(request.pre_rotom_consumption)
                        .saturating_add(request.npc_initial_load)
                        .saturating_add(
                            request.offset.unsigned_abs().min(i64::MAX as u64) as i64 / 2,
                        )
                        .saturating_add(fastest_extra)
                        .min(crate::domain::rng::MAX_SFMT_FRAME);
                    cache.ensure_through(required_end, None)?;
                    AppState::calculate_candidate_reachability_from_cache(request.clone(), cache)
                });
            repaint.request_repaint();
            (request, results)
        });
    }

    pub(super) fn poll_candidate_reachability(&mut self) {
        let Some((_, Ok((request, results)))) = self.jobs.candidate_reachability.take_finished()
        else {
            return;
        };
        match results {
            Ok(results) => self.state.apply_candidate_reachability(&request, &results),
            Err(error) => {
                self.present_app_error(error);
            }
        }
    }

    pub(super) fn maybe_start_production_preparation(&mut self, ctx: &egui::Context) {
        if !self.state.production_preparation_pending
            || self.jobs.production_preparation.is_running()
        {
            return;
        }
        if self.state.target_dirty {
            return;
        }
        let Some((request, cache)) = self.state.production_preparation_request() else {
            // 範囲外Targetなど、Timeline生成が不要な入力は待機表示にしない。
            self.state.finish_production_preparation_without_result();
            return;
        };
        let repaint = ctx.clone();
        self.jobs.production_preparation.spawn(move || {
            let result = AppState::calculate_production_preparation(request.clone(), cache);
            repaint.request_repaint();
            (request, result)
        });
    }

    pub(super) fn poll_production_preparation(&mut self) {
        let Some((_, worker_result)) = self.jobs.production_preparation.take_finished() else {
            // `None`は「まだ実行中」。ここでpendingを落とすと、Enter要求が
            // 計算中のTimelineを使って開始できず、UI表示も準備完了と誤認する。
            return;
        };
        let Ok((request, result)) = worker_result else {
            self.state.finish_production_preparation_without_result();
            if self.state.observation_cache.is_none() {
                self.clear_unused_sfmt_cache_files();
            }
            return;
        };
        match result {
            Ok(prepared) => {
                let applied = self.state.apply_production_preparation(&request, prepared);
                if !applied && self.state.observation_cache.is_none() {
                    self.clear_unused_sfmt_cache_files();
                }
            }
            Err(error) => {
                let still_current = self
                    .state
                    .production_preparation_request()
                    .is_some_and(|(current, _)| current == request);
                if still_current {
                    self.state.finish_production_preparation_without_result();
                    self.present_app_error(error);
                } else if self.state.observation_cache.is_none() {
                    self.clear_unused_sfmt_cache_files();
                }
            }
        }
    }
}

impl UiApp {
    /// フィールド観測をリセットしたとき、検索ワーカーへキャンセルを通知して破棄する。
    /// 生成・照合中も共有トークンを確認するため、キャンセル後にCPUを使い続けたり、
    /// 長い検索の完了まで入力を無効にしたりしない。実行中のFrame1候補検索も止めるが、
    /// 完了済み候補結果はFieldState内に残す。
    pub(super) fn cancel_field_background_tasks(&mut self) {
        self.jobs.field_timeleap_candidates.cancel_and_discard();
        self.field.cancel_timeleap_candidate_search();
        self.jobs.cancel_field_session();
        self.field.cancel_timeleap_shift_resolution();
        self.beeps.field.clear();
        self.beeps.field_start = None;
    }

    pub(super) fn clear_unused_sfmt_cache_files(&self) {
        crate::infrastructure::cache_store::CacheStore::default().clear_unused_files();
    }
}
