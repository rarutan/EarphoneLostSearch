use super::UiApp;
use crate::errors::{AppError, StateIssue};
use crate::infrastructure::audio::AudioOutput;
use eframe::egui;
use std::time::Duration;

impl UiApp {
    pub(super) fn record_hatch_blink(&mut self) {
        if !self.state.is_observing() {
            return;
        }
        let was_exhausted = self.state.search_exhausted;
        self.state.observe();
        if !was_exhausted && self.state.search_exhausted {
            self.play_candidate_not_found();
        }
    }

    pub(super) fn record_field_blink(&mut self, ctx: &egui::Context) {
        if !self.field.is_observation_input_enabled() || self.jobs.field_idx_inference.is_running()
        {
            return;
        }
        if self.field.search_exhausted {
            return;
        }
        // 初回検索で候補が得られた後は、StartingFrameから全Timelineを作り直さず
        // 次の瞬き1回分だけ候補を延長する。初回検索中は再利用可能な状態がないため、
        // 再検索要求として扱う。
        let had_candidates = !self.field.results.is_empty();
        let search_running = self.jobs.field_generation.is_running();
        let previous_interval_count = self.field.intervals.len();
        self.field.observe(self.state.fps_value());
        if had_candidates && !search_running && previous_interval_count >= 2 {
            let expected = self.field.intervals.last().copied().unwrap_or_default();
            let survivors = self.field.advance_candidates_by_interval(expected);
            self.field.search_exhausted = survivors == 0;
            if survivors == 0 {
                self.field
                    .set_candidate_not_found_notice("候補が見つかりません");
                self.play_candidate_not_found();
            } else if survivors == 1 {
                self.field
                    .sync_timeleap_source_inputs_from_internal_timeline();
                self.field.mark_timeline_ready();
                if self.field.can_start_timer() {
                    match self.field.start_blink_timer(self.state.fps_value()) {
                        Ok(()) => {
                            self.play_candidate_confirmed();
                        }
                        Err(_) => {
                            self.field_timer_retry = true;
                            ctx.request_repaint_after(Duration::from_millis(50));
                        }
                    }
                }
            }
        } else {
            if had_candidates {
                self.field.invalidate_search_results();
            }
            if self.field.intervals.len() >= 2 && !search_running {
                self.start_field_search(ctx);
            } else if search_running {
                self.jobs.field_search_needs_restart = true;
            }
        }
    }

    pub(super) fn start_observation_and_generation(&mut self, ctx: &egui::Context) {
        if self.jobs.generation.is_running() {
            return;
        }
        let config = match self.state.search_config() {
            Ok(config) => config,
            Err(_) => {
                return;
            }
        };
        // 起動時の準備に失敗していた環境では、観測開始時に再試行する。
        if self.beep_enabled && self.audio.is_none() {
            self.audio = AudioOutput::open();
        }
        self.state.start_observation_pending();
        let repaint = ctx.clone();
        self.jobs.generation.spawn_cancellable(move |cancel| {
            let cache = crate::domain::rng::generate_observation_cache_cancelable(
                config.seed,
                config.range_start..=config.range_end,
                config.target,
                config.fps,
                Some(cancel.as_ref()),
            );
            repaint.request_repaint();
            cache
        });
    }

    pub(super) fn cancel_observation_generation(&self) {
        self.jobs.generation.cancel();
    }

    pub(super) fn poll_generation(&mut self) {
        let Some((_, result)) = self.jobs.generation.take_finished() else {
            return;
        };
        match result {
            Ok(Ok(cache)) => {
                if !self.state.is_observing() {
                    // クリックでキャンセルした検索が次のフレームで完了することがある。
                    // その結果を待機画面へ再適用しない。
                    return;
                }
                self.state.apply_observation_cache(cache);
                self.state.rematch_observations();
                if self.state.search_exhausted {
                    self.play_candidate_not_found();
                }
            }
            Ok(Err(message)) => {
                if self.state.is_observing() {
                    self.present_app_error(message);
                }
            }
            Err(_) => {}
        }
    }

    pub(super) fn poll_field_generation(&mut self, ctx: &egui::Context) {
        let Some((cancelled, result)) = self.jobs.field_generation.take_finished() else {
            return;
        };
        if cancelled {
            return;
        }
        match result {
            Ok(Ok((outcome, pool))) => {
                if self.jobs.field_search_needs_restart {
                    self.jobs.field_search_needs_restart = false;
                    self.field.set_observation_cache(pool.timeline_cache());
                    self.jobs.field_pool = Some(pool);
                    if self.field.is_observing()
                        && self.field.intervals.len()
                            >= crate::application::state::MIN_BLINK_INTERVALS
                    {
                        self.start_field_search(ctx);
                    }
                    return;
                }
                if !self.field.accepts_search_result() {
                    // キャンセルはクリック操作だけで行う。実行中の検索結果を
                    // キャンセル済みセッションへ再適用しない。
                    // Targetだけの再評価は、既存の瞬きタイマーを動かしたまま別経路で受け付ける。
                    return;
                }
                self.field.set_observation_cache(pool.timeline_cache());
                self.jobs.field_pool = Some(pool);
                // Targetだけの再評価で観測候補を置き換えない。Timeline、開始位置、remain状態、
                // 観測アンカーはTargetから独立しており、到達可否とTargetまでの時間だけを更新する。
                if self.field.target_research_pending && self.field.results.len() == 1 {
                    let selected_key =
                        self.field
                            .results
                            .get(self.field.selected_result)
                            .map(|result| {
                                (
                                    result.start_consumption,
                                    result.models,
                                    result.observed_model,
                                )
                            });
                    if let (Some(current), Some(refined)) = (
                        self.field.results.get_mut(self.field.selected_result),
                        outcome.results.iter().find(|result| {
                            selected_key.is_some_and(|key| {
                                key == (
                                    result.start_consumption,
                                    result.models,
                                    result.observed_model,
                                )
                            })
                        }),
                    ) {
                        current.target_wait_frame = refined.target_wait_frame;
                        current.target_exact = refined.target_exact;
                        current.target_frame_before = refined.target_frame_before;
                        current.target_frame_after = refined.target_frame_after;
                        current.target_blink_count = refined.target_blink_count;
                    }
                    self.field.target_research_pending = false;
                    self.field.search_exhausted = false;
                    self.field.clear_notice();
                    // 観測済みTimelineは変わらないため、瞬きタイマーと候補確定音を再開しない。
                    return;
                }
                self.field.target_research_pending = false;
                self.field.results = self.field.normalize_search_results(outcome.results);
                self.field.search_exhausted = self.field.results.is_empty();
                self.jobs.field_timeline_extension_blocked = false;
                self.jobs.field_timeline_extension_error = None;
                self.field.idx_fallback_indices.clear();
                self.field.table_last_followed_row = None;
                self.field.refresh_nearest_timeline_frame();
                if self.field.results.is_empty() {
                    self.play_candidate_not_found();
                }
                if self.field.results.is_empty() {
                    self.field
                        .set_candidate_not_found_notice("候補が見つかりません");
                } else if self.field.results.len() == 1 {
                    self.field
                        .sync_timeleap_source_inputs_from_internal_timeline();
                    self.field.mark_timeline_ready();
                }
                if self.field.results.len() == 1 {
                    // Enter/ずれ検証後は、遅れて届いた検索結果でも
                    // タイマーや候補確定音を再開しない。リセットでのみ
                    // 新しい観測セッションへ戻る。
                    if self.field.is_correction_mode() {
                        return;
                    }
                    // Timelineが一意になったら瞬き合わせのカウントダウンを開始する。
                    // 先読みキューが空などの理由でタイマーを開始できない場合は、
                    // 確定音だけが先に鳴らないよう、タイマー開始後に通知する。
                    if self.field.can_start_timer() {
                        match self.field.start_blink_timer(self.state.fps_value()) {
                            Ok(()) => {
                                self.play_candidate_confirmed();
                            }
                            Err(AppError::InvalidState(StateIssue::TimelineLimitReached)) => {
                                self.field_timer_retry = false;
                            }
                            Err(_) => {
                                // 先読み末尾へ到達した場合は、次のチャンクをバックグラウンド
                                // で追加してから同じ候補のタイマー開始を再試行する。
                                self.field_timer_retry = true;
                                ctx.request_repaint_after(Duration::from_millis(50));
                            }
                        }
                    }
                }
            }
            Ok(Err(message)) => {
                self.field.target_research_pending = false;
                self.present_app_error(message);
            }
            Err(_) => {
                self.field.target_research_pending = false;
            }
        }
    }

    pub(super) fn poll_field_target_generation(&mut self, ctx: &egui::Context) {
        let Some((cancelled, joined)) = self.jobs.field_target_generation.take_finished() else {
            return;
        };
        if cancelled {
            return;
        }
        let Ok((key, result)) = joined else {
            self.field.target_research_pending = false;
            return;
        };
        let current_target = self.field.target_consumption.trim().parse::<i64>().ok();
        let current_seed =
            u32::from_str_radix(self.field.seed.trim().trim_start_matches("0x"), 16).ok();
        let candidate_matches = self
            .field
            .results
            .get(self.field.selected_result)
            .is_some_and(|candidate| {
                current_seed.is_some_and(|seed| {
                    current_target.is_some_and(|target| key.matches_result(seed, target, candidate))
                })
            });

        if !candidate_matches {
            if self.field.target_research_pending {
                self.start_field_target_research(ctx);
            }
            return;
        }

        match result {
            Ok(refined) => {
                if let Some(current) = self.field.results.get_mut(self.field.selected_result) {
                    current.target_wait_frame = refined.target_wait_frame;
                    current.target_exact = refined.target_exact;
                    current.target_frame_before = refined.target_frame_before;
                    current.target_frame_after = refined.target_frame_after;
                    current.target_blink_count = refined.target_blink_count;
                }
                self.field.target_research_pending = false;
                self.field.search_exhausted = false;
                self.field.clear_notice();
            }
            Err(AppError::Cancelled) => {}
            Err(error) => {
                self.field.target_research_pending = false;
                self.present_app_error(error);
            }
        }
    }
}
