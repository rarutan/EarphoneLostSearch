use super::UiApp;
use crate::application::field::FieldTimerKind;
use crate::domain::timing::beep_offsets_for_segment;
use crate::ui::beep_scheduler::BeepScheduler;
use eframe::egui;
use std::time::{Duration, Instant};

impl UiApp {
    pub(super) fn handle_timeleap_frame1_transition(&mut self, ctx: &egui::Context, now: Instant) {
        // Frame1の期限に予約された最後の音を、新しいタイマーに切り替える前に処理する。
        self.finish_field_beep_segment_before_timeleap_transition(now);
        if !self.field.adopt_assumed_timeleap_timeline(now) {
            self.jobs.cancel_prepared_timeleap_target();
            return;
        }
        if self.field.timer_kind() == FieldTimerKind::Target
            && self.apply_prepared_timeleap_target()
        {
            self.field_timer_retry = false;
            if self
                .field
                .start_target_timer(self.state.fps_value())
                .is_err()
            {
                self.field.stop_target_timer_unavailable();
            }
        } else {
            self.field.prepare_target_research(self.state.fps_value());
            self.start_field_target_research(ctx);
            if self.field.timer_kind() == FieldTimerKind::Target {
                self.field_timer_retry = true;
                ctx.request_repaint_after(Duration::from_millis(50));
            } else {
                match self.field.start_blink_timer(self.state.fps_value()) {
                    Ok(())
                    | Err(crate::errors::AppError::InvalidState(
                        crate::errors::StateIssue::TimelineLimitReached,
                    )) => {}
                    Err(_) => {
                        self.field_timer_retry = true;
                        ctx.request_repaint_after(Duration::from_millis(50));
                    }
                }
            }
        }
    }

    pub(super) fn toggle_normal_timer(&mut self) {
        if self.state.normal_timer.is_running() {
            self.state.normal_timer.stop();
            self.beeps.normal.clear();
            return;
        }

        if self
            .state
            .normal_timer
            .start(self.state.fps_value())
            .is_ok()
        {
            self.schedule_normal_timer_beeps();
        }
    }

    pub(super) fn retry_field_timer_if_needed(&mut self, ctx: &egui::Context) {
        if !self.field_timer_retry || self.field.timer_is_running() || self.field.results.len() != 1
        {
            return;
        }
        if self.field.timer_kind() == FieldTimerKind::Target {
            if self.field.target_research_pending {
                ctx.request_repaint_after(Duration::from_millis(50));
                return;
            }
            match self.field.start_target_timer(self.state.fps_value()) {
                Ok(()) => self.field_timer_retry = false,
                Err(_) => {
                    // Keep Target selected if the destination is unavailable;
                    // never silently replace the requested clock with blinking.
                    self.field_timer_retry = false;
                    self.field.stop_target_timer_unavailable();
                }
            }
        } else if self.field.start_blink_timer(self.state.fps_value()).is_ok() {
            self.field_timer_retry = false;
        } else if self.jobs.field_timeline_extension_blocked {
            // 上限到達なら範囲外として停止し、計算失敗ならそのエラーを表示する。
            self.field_timer_retry = false;
            if let Some(message) = self.jobs.field_timeline_extension_error.take() {
                self.present_app_error_ref(&message);
            } else {
                self.field.stop_blink_timer_out_of_range();
            }
        } else {
            // Timeline延長のワーカーがまだ終わっていない。短い間隔で
            // 再試行し、タイマーと通知音を停止状態のまま放置しない。
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    pub(super) fn tick_normal_timer(&mut self, ctx: &egui::Context) {
        self.tick_normal_timer_at(ctx, Instant::now());
    }

    pub(super) fn tick_normal_timer_at(&mut self, ctx: &egui::Context, now: Instant) {
        let due = BeepScheduler::drain_due(&mut self.beeps.normal, now);
        for _ in 0..due {
            self.play_beep();
        }

        while self.state.normal_timer.is_running()
            && self.state.normal_timer.remaining_at(now) <= 0.0
        {
            let due = BeepScheduler::drain_due(&mut self.beeps.normal, now);
            for _ in 0..due {
                self.play_beep();
            }
            let advanced = self
                .state
                .normal_timer
                .advance_segment_at(self.state.fps_value(), now);
            self.beeps.normal.clear();
            if !advanced {
                break;
            }
            self.schedule_normal_timer_beeps();
            let due = BeepScheduler::drain_due(&mut self.beeps.normal, now);
            for _ in 0..due {
                self.play_beep();
            }
        }

        if self.state.normal_timer.is_running() || !self.beeps.normal.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    pub(super) fn normal_beep_parameters(&self) -> (u32, f64) {
        let count = self
            .state
            .beep_count
            .trim()
            .parse::<u32>()
            .unwrap_or(1)
            .max(1);
        let interval = self
            .state
            .beep_interval
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(1.0);
        (count, interval)
    }

    pub(super) fn schedule_normal_timer_beeps(&mut self) {
        let Some(start) = self.state.normal_timer.timer_start else {
            self.beeps.normal.clear();
            return;
        };
        self.beeps.normal =
            self.schedule_segment_beeps(Some(start), self.state.normal_timer.timer_seconds);
    }

    pub(super) fn ensure_article_beep_schedule(&mut self) {
        if !self.state.is_countdown() {
            return;
        }
        let Some(start) = self.state.article_timer_start_time() else {
            return;
        };
        if self.beeps.article_start == Some(start) {
            return;
        }
        self.beeps.article_start = Some(start);
        self.beeps.article =
            self.schedule_segment_beeps(Some(start), self.state.article_timer_seconds_value());
    }

    pub(super) fn ensure_field_beep_schedule(&mut self) {
        let Some(start) = self.field.timer_start_time() else {
            return;
        };
        if self.beeps.field_start == Some(start) {
            return;
        }
        self.beeps.field_start = Some(start);
        // フィールド観測のbeepは予測瞬きそのものを示すため常に1回とする。
        // 設定した回数はTargetタイマーなど長いカウントダウンだけで使う。
        let (count, interval) = match self.field.timer_kind() {
            FieldTimerKind::NextBlink => (1, self.normal_beep_parameters().1),
            FieldTimerKind::Target => self.normal_beep_parameters(),
        };
        self.beeps.field = self.schedule_segment_beeps_with_parameters(
            Some(start),
            self.field.timer_seconds_value(),
            count,
            interval,
        );
    }

    pub(super) fn finish_field_beep_segment_before_timeleap_transition(
        &mut self,
        now: Instant,
    ) -> usize {
        let due = BeepScheduler::drain_due(&mut self.beeps.field, now);
        for _ in 0..due {
            self.play_beep();
        }
        self.beeps.field.clear();
        self.beeps.field_start = None;
        self.field_timer_retry = false;
        due
    }

    pub(super) fn schedule_segment_beeps(
        &self,
        start: Option<Instant>,
        duration_seconds: f64,
    ) -> Vec<Instant> {
        let (count, interval) = self.normal_beep_parameters();
        self.schedule_segment_beeps_with_parameters(start, duration_seconds, count, interval)
    }

    pub(super) fn schedule_segment_beeps_with_parameters(
        &self,
        start: Option<Instant>,
        duration_seconds: f64,
        count: u32,
        interval: f64,
    ) -> Vec<Instant> {
        let Some(start) = start else {
            return Vec::new();
        };
        let duration_seconds = if duration_seconds.is_finite() {
            duration_seconds.max(0.0)
        } else {
            0.0
        };
        beep_offsets_for_segment(duration_seconds, count, interval)
            .into_iter()
            .map(|offset| start + Duration::from_secs_f64(offset))
            .collect()
    }
}
