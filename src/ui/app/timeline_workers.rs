use super::UiApp;
use crate::errors::{AppError, StateIssue};
use crate::ui::app_types::{CorrectionDialog, FieldTargetKey};
use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

impl UiApp {
    pub(super) fn poll_field_pool_generation(&mut self, _ctx: &egui::Context) {
        let Some((cancelled, result)) = self.jobs.field_pool_generation.take_finished() else {
            return;
        };
        if cancelled {
            return;
        }
        match result {
            Ok(Ok(pool)) => {
                if self.field.is_observing() {
                    self.jobs.field_pool = Some(pool);
                }
            }
            Ok(Err(message)) => {
                self.present_app_error(message);
            }
            Err(_) => {}
        }
    }

    /// SFMTプール完成後のTimeline索引を別ワーカーで先行生成する。
    ///
    /// この処理は観測入力や検索ワーカーと同じJoinHandleへ入れない。観測中に
    /// 完了すれば検索で再利用でき、間に合わない場合でも検索自体は詳細照合へ
    /// 進められる。リセット／検索開始時にはキャンセルして古い索引を破棄する。
    #[allow(dead_code)]
    pub(super) fn start_field_timeline_generation(&mut self, ctx: &egui::Context) {
        if self.jobs.field_timeline_generation.is_running() {
            return;
        }
        let Some(pool) = self.jobs.field_pool.as_ref() else {
            return;
        };
        let Ok(configs) = self.field.configs() else {
            return;
        };
        let cache = pool.timeline_cache();
        let repaint = ctx.clone();
        self.jobs
            .field_timeline_generation
            .spawn_cancellable(move |cancel| {
                let result = crate::application::field::prepare_timeline_warmup_cancelable(
                    &cache,
                    &configs,
                    Some(cancel.as_ref()),
                );
                repaint.request_repaint();
                result
            });
    }

    pub(super) fn poll_field_timeline_generation(&mut self) {
        let Some((cancelled, result)) = self.jobs.field_timeline_generation.take_finished() else {
            return;
        };
        if cancelled {
            return;
        }
        match result {
            Ok(Ok(index)) => {
                if self.field.is_observing() {
                    if let Some(pool) = self.jobs.field_pool.as_mut() {
                        pool.install_timeline(index);
                    }
                }
            }
            Ok(Err(message)) => {
                // Timeline索引は先行最適化なので、失敗しても詳細検索は継続する。
                self.present_app_error(message);
            }
            Err(_) => {}
        }
    }

    /// 一意候補のTimeline延長はModelStatusとSFMTを進めるため、UIスレッドで
    /// 実行しない。表示履歴には上限があり、ワーカーのスナップショットが履歴全体と
    /// ともに肥大化しないようにする。
    pub(super) fn start_field_timeline_extension(&mut self, ctx: &egui::Context) {
        if self.jobs.field_timeline_extension.is_running()
            || self.jobs.field_timeline_extension_blocked
            || !self.field.timeline_extension_needed(self.state.fps_value())
        {
            return;
        }
        let Some(request) = self.field.timeline_extension_request() else {
            return;
        };
        self.jobs.field_timeline_extension_generation = self
            .jobs
            .field_timeline_extension_generation
            .wrapping_add(1);
        let generation = self.jobs.field_timeline_extension_generation;
        let fps = self.state.fps_value();
        let repaint = ctx.clone();
        self.jobs.field_timeline_extension_key = Some(generation);
        self.jobs
            .field_timeline_extension
            .spawn_cancellable(move |cancel| {
                let result = request.extend(fps, Some(cancel.as_ref()));
                repaint.request_repaint();
                result
            });
    }

    pub(super) fn poll_field_timeline_extension(&mut self) {
        let Some((cancelled, result)) = self.jobs.field_timeline_extension.take_finished() else {
            return;
        };
        let key = self.jobs.field_timeline_extension_key.take();
        if cancelled || key != Some(self.jobs.field_timeline_extension_generation) {
            return;
        }
        let Ok(result) = result else {
            self.jobs.field_timeline_extension_blocked = true;
            let error = Arc::new(AppError::WorkerPanicked);
            self.jobs.field_timeline_extension_error = Some(error.clone());
            self.present_app_error_ref(&error);
            return;
        };
        if !self.field.is_correction_mode() {
            match result {
                Ok(extension) => {
                    let Some(can_continue) = self.field.apply_timeline_extension(extension) else {
                        return;
                    };
                    self.jobs.field_timeline_extension_blocked = !can_continue;
                    self.jobs.field_timeline_extension_error = None;
                }
                Err(message) => {
                    if matches!(&message, AppError::Cancelled) {
                        return;
                    }
                    self.jobs.field_timeline_extension_blocked = true;
                    let message = Arc::new(message);
                    self.jobs.field_timeline_extension_error = Some(message.clone());
                    self.present_app_error_ref(&message);
                }
            }
        }
    }

    pub(super) fn poll_field_idx_inference(&mut self, _ctx: &egui::Context) {
        let Some((cancelled, result)) = self.jobs.field_idx_inference.take_finished() else {
            return;
        };
        if cancelled {
            return;
        }
        match result {
            Ok(Ok(inference)) => {
                if self.field.intervals.len() < crate::application::state::MIN_BLINK_INTERVALS {
                    return;
                }
                self.field.inferred_indices = inference.indices.clone();
                self.field.correction_calculated = true;
                self.field.correction_suggested_offset = inference.suggested_offset;
                let unknown = inference.indices.is_empty() || inference.indices.len() > 5;
                self.correction_dialog = Some(CorrectionDialog::ConfirmFieldCorrection {
                    indices: if unknown {
                        Vec::new()
                    } else {
                        inference.indices
                    },
                    unknown,
                    suggested_offset: inference.suggested_offset,
                });
                self.field.clear_notice();
            }
            Ok(Err(message)) => {
                self.present_app_error(message);
            }
            Err(_) => {}
        }
    }

    pub(super) fn cancel_field_idx_inference(&mut self) {
        self.jobs.cancel_field_idx_inference();
    }

    pub(super) fn start_field_pool_generation(&mut self, ctx: &egui::Context) {
        if self.jobs.field_pool_generation.is_running() {
            return;
        }
        let configs = match self.field.configs() {
            Ok(configs) => configs,
            Err(message) => {
                self.field.set_search_range_notice(message.to_string());
                return;
            }
        };
        let repaint = ctx.clone();
        self.jobs
            .field_pool_generation
            .spawn_cancellable(move |cancel| {
                let result = crate::application::field::prepare_search_pool_with_configs_cancelable(
                    &configs,
                    Some(cancel.as_ref()),
                );
                repaint.request_repaint();
                result
            });
    }

    pub(super) fn start_field_idx_inference(&mut self, ctx: &egui::Context) {
        if self.jobs.field_idx_inference.is_running() || self.field.timer_is_running() {
            return;
        }
        let Ok(configs) = self.field.configs() else {
            return;
        };
        let Ok(actual_frame) = self.field.actual_frame.trim().parse::<i64>() else {
            return;
        };
        let Some(selected) = self.field.results.get(self.field.selected_result).cloned() else {
            return;
        };
        let Some(seed) = configs.first().map(|config| config.seed) else {
            return;
        };
        let tolerance = configs.first().map(|config| config.tolerance).unwrap_or(0);
        let target = configs.first().and_then(|config| config.target_consumption);
        let current_offset = if self.field.use_offset {
            self.field
                .encounter_offset
                .trim()
                .parse::<i64>()
                .unwrap_or_default()
        } else {
            0
        };
        if actual_frame < 0 || self.field.results.len() != 1 {
            return;
        }
        if self.field.correction_calculated {
            let indices = self.field.inferred_indices.clone();
            let unknown = indices.is_empty() || indices.len() > 5;
            self.correction_dialog = Some(CorrectionDialog::ConfirmFieldCorrection {
                indices: if unknown { Vec::new() } else { indices },
                unknown,
                suggested_offset: self.field.correction_suggested_offset,
            });
            return;
        }
        self.field.idx_fallback_indices.clear();
        let repaint = ctx.clone();
        let observation_cache = self
            .jobs
            .field_pool
            .as_ref()
            .map(|pool| pool.timeline_cache());
        self.jobs
            .field_idx_inference
            .spawn_cancellable(move |cancel| {
                let result = crate::application::field::infer_idx_from_result_with_cache_cancelable(
                    seed,
                    &selected,
                    tolerance,
                    actual_frame,
                    observation_cache.as_ref(),
                    Some(cancel.as_ref()),
                );
                let Ok(mut inference) = result else {
                    repaint.request_repaint();
                    return result;
                };
                inference.suggested_offset = target.and_then(|target| {
                crate::application::field::suggest_field_offset_from_result_with_cache_cancelable(
                    seed,
                    &selected,
                    target,
                    actual_frame,
                    current_offset,
                    observation_cache.as_ref(),
                    Some(cancel.as_ref()),
                )
                .ok()
                .flatten()
            });
                repaint.request_repaint();
                Ok(inference)
            });
    }

    pub(super) fn start_field_search(&mut self, ctx: &egui::Context) {
        if self.field.target_research_pending && self.field.results.len() == 1 {
            self.start_field_target_research(ctx);
            return;
        }
        if self.jobs.field_generation.is_running() {
            return;
        }
        let configs = match self.field.configs() {
            Ok(configs) => configs,
            Err(message) => {
                self.field.target_research_pending = false;
                self.field.set_search_range_notice(message.to_string());
                return;
            }
        };
        let initial_branch_scan_span = match self.field.initial_branch_scan_span_value() {
            Ok(value) => value,
            Err(message) => {
                self.field.target_research_pending = false;
                self.field.set_search_range_notice(message.to_string());
                return;
            }
        };
        // 先行Timelineは観測入力に間に合わなかった場合、検索とCPUを奪い合わない
        // ように止める。完了済みならpoll側でプールへ取り込まれている。
        self.jobs.field_timeline_generation.cancel_and_discard();
        self.jobs.cancel_field_timeline_extension();
        let target_mode = self.field.target_mode;
        let cached_pool = self
            .jobs
            .field_pool
            .take()
            .filter(|pool| configs.iter().all(|config| pool.covers(config)));
        let (pending_pool, pending_pool_cancel) = self
            .jobs
            .field_pool_generation
            .take()
            .map_or((None, None), |(handle, cancel)| (Some(handle), cancel));
        let cancel = pending_pool_cancel.unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
        let repaint = ctx.clone();
        self.jobs
            .field_generation
            .spawn_with_token(cancel.clone(), move || {
                if cancel.load(Ordering::Acquire) {
                    return Err(AppError::Cancelled);
                }
                let pool = if let Some(pool) = cached_pool {
                    Ok(pool)
                } else if let Some(handle) = pending_pool {
                    let pool = handle.join().map_err(|_| AppError::CacheWorkerJoin)??;
                    if configs.iter().all(|config| pool.covers(config)) {
                        Ok(pool)
                    } else {
                        crate::application::field::prepare_search_pool_with_configs_cancelable(
                            &configs,
                            Some(cancel.as_ref()),
                        )
                    }
                } else {
                    crate::application::field::prepare_search_pool_with_configs_cancelable(
                        &configs,
                        Some(cancel.as_ref()),
                    )
                }?;
                let result =
                    crate::application::field::search_with_npc_range_with_pool_cancelable_and_span(
                        &configs,
                        &pool,
                        target_mode,
                        initial_branch_scan_span,
                        Some(cancel.as_ref()),
                    )?;
                repaint.request_repaint();
                Ok((result, pool))
            });
    }

    pub(super) fn start_field_target_research(&mut self, ctx: &egui::Context) {
        if self.field.timeleap_direct_encounter
            && self.field.timer_kind() == crate::application::field::FieldTimerKind::Target
        {
            // Every Target refresh (not just the initial Timeleap adoption)
            // must resume the selected clock from its original observation
            // anchor once this asynchronous calculation finishes.
            self.field_timer_retry = true;
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        if self.jobs.field_target_generation.is_running() {
            return;
        }
        let Some(target) = self.field.target_consumption.trim().parse::<i64>().ok() else {
            self.field.target_research_pending = false;
            return;
        };
        let Some(seed) =
            u32::from_str_radix(self.field.seed.trim().trim_start_matches("0x"), 16).ok()
        else {
            self.field.target_research_pending = false;
            return;
        };
        if let Err(error) = self.field.prepare_shared_timeline_cache() {
            self.field.target_research_pending = false;
            self.present_app_error(error);
            return;
        }
        let Some(candidate) = self.field.results.get(self.field.selected_result).cloned() else {
            self.field.target_research_pending = false;
            return;
        };
        if self.field.observation_cache_for_seed(seed).is_none() {
            self.field.target_research_pending = false;
            self.present_app_error(AppError::InvalidState(StateIssue::CacheUnavailable));
            return;
        }
        let key = FieldTargetKey::for_result(seed, target, &candidate);
        let repaint = ctx.clone();
        let mut snapshot = self.field.clone();
        self.jobs
            .field_target_generation
            .spawn_cancellable(move |cancel| {
                let result =
                    snapshot.retarget_confirmed_result_from_cache(target, Some(cancel.as_ref()));
                repaint.request_repaint();
                (key, result)
            });
    }
}
