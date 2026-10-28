use super::{AppState, MAX_AUTO_TOLERANCE, MAX_BLINK_OBSERVATIONS, MAX_FRAME, MIN_BLINK_INTERVALS};
use crate::application::state_types::Mode;
use crate::domain::rng::{ObservationCache, ObservationMatch};
use crate::domain::timing::{
    blink_ticks_to_seconds, game_frames_to_seconds, seconds_to_game_frames,
};
use std::time::{Duration, Instant};

impl AppState {
    // --- Observation lifecycle and blink scheduling ----------------------

    pub fn apply_observation_cache(&mut self, cache: ObservationCache) {
        self.target_dirty = false;
        self.production_preparation_pending = false;
        self.encounter_start_requested = false;
        self.observation_cache = Some(cache);
        self.clear_guidance_reachability();
        self.production = None;
        self.clear_candidates();
        self.selected = 0;
        self.observed_rotom_talk = None;
        self.timer_start = None;
        self.next_blink = None;
        self.next_blink_index = None;
        self.table_follow_position = None;
        self.fast_close_circle_frames.clear();
        self.correction_source_frame = None;
        self.correction_source_index = 0;
        self.correction_target_frame = None;
        self.correction_source_offset = None;
        self.clear_article_timer();
    }

    /// 計算中でも観測を先に開始できる入口。計算完了後に観測列を照合する。
    pub fn start_observation_pending(&mut self) {
        self.target_dirty = false;
        self.production_preparation_pending = false;
        self.encounter_start_requested = false;
        self.observation_cache = None;
        self.clear_guidance_reachability();
        self.production = None;
        self.enter_observing();
        self.observed.clear();
        self.intervals.clear();
        self.match_tolerance = self.tolerance_value();
        self.search_exhausted = false;
        self.clear_candidates();
        self.selected = 0;
        self.observed_rotom_talk = None;
        self.next_blink = None;
        self.next_blink_index = None;
        self.table_follow_position = None;
        self.fast_close_circle_frames.clear();
        self.correction_source_frame = None;
        self.correction_source_index = 0;
        self.correction_target_frame = None;
        self.correction_source_offset = None;
        self.clear_article_timer();
    }

    pub fn cancel_observation(&mut self) {
        self.target_dirty = false;
        self.production_preparation_pending = false;
        self.encounter_start_requested = false;
        self.observation_cache = None;
        self.enter_idle();
        self.clear_guidance_reachability();
        self.production = None;
        self.timer_start = None;
        self.timer_seconds = 0.0;
        self.observed.clear();
        self.intervals.clear();
        self.search_exhausted = false;
        self.match_tolerance = self.tolerance_value();
        self.clear_candidates();
        self.selected = 0;
        self.observed_rotom_talk = None;
        self.next_blink = None;
        self.next_blink_index = None;
        self.table_follow_position = None;
        self.fast_close_circle_frames.clear();
        self.correction_source_frame = None;
        self.correction_source_index = 0;
        self.correction_target_frame = None;
        self.correction_source_offset = None;
        self.clear_article_timer();
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn generate(&mut self) {
        let Ok(seed) = u32::from_str_radix(self.seed.trim().trim_start_matches("0x"), 16) else {
            return;
        };
        let Ok(a) = self.range_start.trim().parse::<i64>() else {
            return;
        };
        let Ok(b) = self.range_end.trim().parse::<i64>() else {
            return;
        };
        let Ok(target) = self.target.trim().parse::<i64>() else {
            return;
        };
        if a < 0 || b < a || target < 0 || b > MAX_FRAME || target > MAX_FRAME {
            return;
        }
        // 観測段階はNPCなし。NPC数は孵化演出終了後の本番計算でだけ使う。
        match crate::domain::rng::generate_observation_cache(seed, a..=b, target, self.fps_value())
        {
            Ok(cache) => self.observation_cache = Some(cache),
            Err(_) => {
                return;
            }
        }
        self.clear_guidance_reachability();
        self.production = None;
        self.clear_candidates();
        self.selected = 0;
        self.observed_rotom_talk = None;
        self.enter_idle();
        self.timer_start = None;
        self.next_blink = None;
        self.next_blink_index = None;
        self.table_follow_position = None;
        self.fast_close_circle_frames.clear();
        self.correction_source_frame = None;
        self.correction_source_index = 0;
        self.correction_target_frame = None;
        self.correction_source_offset = None;
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn start_observation(&mut self) {
        if self.observation_cache.is_none() {
            return;
        }
        self.enter_observing();
        self.clear_guidance_reachability();
        self.production = None;
        self.observed.clear();
        self.intervals.clear();
        self.clear_candidates();
        self.selected = 0;
        self.observed_rotom_talk = None;
        self.next_blink = None;
        self.next_blink_index = None;
        self.table_follow_position = None;
        self.fast_close_circle_frames.clear();
        self.correction_source_frame = None;
        self.correction_source_index = 0;
        self.correction_target_frame = None;
        self.correction_source_offset = None;
    }

    pub fn observe(&mut self) {
        if self.mode != Mode::Observing {
            return;
        }
        if self.observed.len() >= MAX_BLINK_OBSERVATIONS {
            return;
        }
        let now = Instant::now();
        self.observed.push(now);
        if let Some(previous) = self.observed.iter().rev().nth(1) {
            // 観測間隔はFieldTimelineと同じ表示Frame単位で保持する。
            // Timeline内部の1/30秒tick（=2F）への変換は検索側で行う。
            self.intervals.push(
                seconds_to_game_frames(
                    now.duration_since(*previous).as_secs_f64(),
                    self.fps_value(),
                )
                .round() as i64,
            );
        }

        // 最初のSpaceは基準点のみ。最初の瞬きまでの待ち時間は検索に含めない。
        if self.intervals.is_empty() {
            return;
        }
        if self.intervals.len() < MIN_BLINK_INTERVALS {
            self.clear_candidates();
            self.search_exhausted = false;
            return;
        }
        if self.observation_cache.is_none() {
            self.clear_candidates();
            self.search_exhausted = false;
            return;
        }

        self.rematch_candidates();
        self.selected = 0;
        self.refresh_production();
        match self.candidates.len() {
            0 => {}
            1 => {
                self.enter_ready();
            }
            _ => {}
        }
        self.schedule_blink();
    }

    /// バックグラウンド計算が終わった後、すでに入力済みの瞬き列を再照合する。
    pub fn rematch_observations(&mut self) {
        if self.mode != Mode::Observing {
            return;
        }
        if self.intervals.len() < MIN_BLINK_INTERVALS {
            self.clear_candidates();
            self.search_exhausted = false;
            return;
        }
        if self.observation_cache.is_none() {
            self.clear_candidates();
            self.search_exhausted = false;
            return;
        }
        self.rematch_candidates();
        self.selected = 0;
        self.refresh_production();
        if self.candidates.len() == 1 {
            self.enter_ready();
        }
        self.schedule_blink();
    }

    pub(super) fn rematch_candidates(&mut self) {
        let mut tolerance = self
            .match_tolerance
            .max(self.tolerance_value())
            .min(MAX_AUTO_TOLERANCE);
        loop {
            let matches = self.find_matches(tolerance);
            if !matches.is_empty() || tolerance >= MAX_AUTO_TOLERANCE {
                self.replace_candidates(
                    matches
                        .into_iter()
                        .map(|matched| self.make_candidate(matched))
                        .collect(),
                );
                self.match_tolerance = tolerance;
                self.search_exhausted = self.candidates.is_empty();
                return;
            }
            tolerance += 1;
        }
    }

    pub(super) fn find_matches(&self, tolerance: i64) -> Vec<ObservationMatch> {
        let Some(cache) = self.observation_cache.as_ref() else {
            return Vec::new();
        };
        match cache.find_matches(&self.intervals, tolerance) {
            Ok(matches) => matches,
            Err(error) => {
                self.defer_cache_io_error(error);
                Vec::new()
            }
        }
    }

    pub(super) fn schedule_blink(&mut self) {
        // 候補が複数ある段階では、選択中の先頭候補を現在地として扱わない。
        // 一意に確定するまでタイマー・beep・青い現在行をすべて停止する。
        if self.candidates.len() != 1 {
            self.next_blink = None;
            self.next_blink_index = None;
            return;
        }
        // Shiftを押した実時刻が同期の基準である。最初のShiftから動かしている
        // 実測時計の経過分をそのまま使い、最後のShiftから先だけを乱数で予測する。
        // 過去の観測区間まで予測値で置き換えると、照合許容差が区間ごとに累積し、
        // 候補自体は正しくても最初のbeepが実機からずれる。
        let Some(first) = self.observed.first().copied() else {
            self.next_blink = None;
            return;
        };
        let last = self.observed.last().copied().unwrap_or(first);
        let observed_elapsed = last.saturating_duration_since(first);
        self.schedule_blink_from(first + observed_elapsed);
    }

    pub(super) fn next_blink_seconds_at(&self, position: i64, fallback_ticks: i64) -> Option<f64> {
        match self
            .observation_cache
            .as_ref()
            .map(|cache| cache.interval_seconds_at(position))
        {
            Some(Ok(Some(seconds))) => Some(seconds),
            Some(Err(error)) if error.is_cache_io() => {
                self.defer_cache_io_error(error);
                None
            }
            _ => Some(blink_ticks_to_seconds(fallback_ticks, self.fps_value())),
        }
    }

    pub(super) fn schedule_blink_from(&mut self, anchor: Instant) {
        let Some(candidate) = self.candidates.get(self.selected).copied() else {
            self.next_blink = None;
            return;
        };
        let Some(interval_ticks) = candidate.next_blink_ticks else {
            self.next_blink = None;
            return;
        };
        let Some(interval_seconds) =
            self.next_blink_seconds_at(candidate.sfmt_position, interval_ticks)
        else {
            self.next_blink = None;
            self.next_blink_index = None;
            return;
        };
        let deadline = anchor + Duration::from_secs_f64(interval_seconds);
        self.next_blink = Some(self.shift_deadline_by_adjust(deadline));
        self.next_blink_index = Some(candidate.sfmt_position);
    }

    /// UIの「補正」は実機との位相を合わせる実時間Frame補正であり、
    /// SFMT消費位置を変更しない。
    pub(super) fn shift_deadline_by_adjust(&self, deadline: Instant) -> Instant {
        let shift = Duration::from_secs_f64(game_frames_to_seconds(
            self.adjust.unsigned_abs().min(MAX_FRAME as u64) as f64,
            self.fps_value(),
        ));
        // +補正は「音が遅いので速める」、-補正は「音が早いので遅くする」。
        if self.adjust.is_positive() {
            deadline.checked_sub(shift).unwrap_or_else(Instant::now)
        } else {
            deadline.checked_add(shift).unwrap_or(deadline)
        }
    }

    /// 予測beepを次の瞬きへ進める。実機の瞬きを入力した場合はobserveが再同期する。
    pub fn advance_predicted_blink(&mut self) {
        let Some(_candidate) = self.candidates.get(self.selected).copied() else {
            self.next_blink = None;
            self.next_blink_index = None;
            return;
        };
        // repaintは最大50ms間隔なので、beep処理が少し遅れても次の予定時刻を
        // now()基準にしない。now()を使うと、遅延が瞬きごとに累積してしまう。
        let previous_deadline = self.next_blink.unwrap_or_else(Instant::now);
        let Some(current_index) = self.next_blink_index else {
            self.schedule_blink();
            return;
        };
        // `current_index`の乱数値は、いま鳴らした瞬きまでの区間ですでに使用済み。
        // 次回は必ず1つ進めた位置の乱数値を使う。
        let next_index = current_index.saturating_add(1);
        let interval_ticks = match self
            .observation_cache
            .as_ref()
            .map(|cache| cache.interval_ticks_at(next_index))
        {
            Some(Ok(Some(ticks))) => ticks,
            Some(Err(error)) if error.is_cache_io() => {
                self.defer_cache_io_error(error);
                self.next_blink = None;
                self.next_blink_index = None;
                return;
            }
            _ => {
                self.next_blink = None;
                self.next_blink_index = None;
                return;
            }
        };
        let Some(interval_seconds) = self.next_blink_seconds_at(next_index, interval_ticks) else {
            self.next_blink = None;
            self.next_blink_index = None;
            return;
        };
        self.next_blink = Some(previous_deadline + Duration::from_secs_f64(interval_seconds));
        self.next_blink_index = Some(next_index);
        // 現在位置が進んだ場合も本番Timelineは更新するが、絶対Frameごとの
        // 到達判定結果は変わらないため、次の○の先読みを消さずに保持する。
        self.refresh_production_preserving_guidance();
    }

    /// 現在選択中の候補について、次に消費されるSFMT位置を返す。
    /// 予測瞬きが発生した後は、候補の初期位置ではなく次の位置へ進む。
    pub fn timeline_display_position(&self) -> Option<i64> {
        if self.candidates.len() != 1 {
            return None;
        }
        let candidate = self.candidates.get(self.selected)?;
        let next_position = self.next_blink_index.unwrap_or(candidate.sfmt_position);
        Some(next_position)
    }

    pub fn adjust_by(&mut self, delta: i64) {
        self.adjust = self.adjust.saturating_add(delta);
        // 補正はbeepの位相だけを動かす。SFMT位置、到達可否、ロトム判定は
        // 変えないため、本番タイムラインを再計算しない。
        let shift = Duration::from_secs_f64(game_frames_to_seconds(
            delta.unsigned_abs().min(MAX_FRAME as u64) as f64,
            self.fps_value(),
        ));
        if let Some(deadline) = self.next_blink.as_mut() {
            if delta.is_positive() {
                *deadline = deadline.checked_sub(shift).unwrap_or_else(Instant::now);
            } else {
                *deadline = deadline.checked_add(shift).unwrap_or(*deadline);
            }
        }
    }

    pub fn set_adjust(&mut self, value: i64) {
        self.adjust_by(value.saturating_sub(self.adjust));
    }
}
