use super::*;

impl FieldState {
    pub(super) fn timeline_limit_reached(&self) -> bool {
        if let Some(shared) = &self.resolved_timeline_cache {
            if let Ok(cache) = shared.lock() {
                if cache.cursor.consumption >= crate::domain::rng::MAX_SFMT_FRAME
                    || cache.cursor.tick >= MAX_FUTURE_TIMELINE_TICKS
                {
                    return true;
                }
            }
        }
        self.results
            .get(self.selected_result)
            .and_then(|result| result.timeline_cursor.as_ref())
            .is_some_and(|cursor| {
                cursor.consumption >= crate::domain::rng::MAX_SFMT_FRAME
                    || cursor.tick >= MAX_FUTURE_TIMELINE_TICKS
            })
    }

    /// 検索上限まで到達し、次の瞬きがないと確定したときにタイマーを停止する。
    pub(crate) fn stop_blink_timer_out_of_range(&mut self) {
        self.timer_start = None;
        self.timer_seconds = 0.0;
        self.timer_finished = false;
        self.next_blink_out_of_range = true;
        self.clear_notice();
    }

    /// Targetへ到達できないと確定した場合も選択状態を保ち、停止表示にする。
    pub(crate) fn stop_target_timer_unavailable(&mut self) {
        self.timer_start = None;
        self.timer_seconds = 0.0;
        self.timer_finished = true;
    }

    /// 明示的なずれ検証操作でのみ呼び出す。押下時の現在位置を保存して
    /// Timeline表を停止し、実測Frame・idx入力を解放する。
    pub fn enter_correction_mode(&mut self, fps: f64) {
        let remaining = if self.timer_kind == FieldTimerKind::Target && self.target_is_passed(fps) {
            Some(0.0)
        } else {
            self.preview_remaining(self.timer_kind, fps)
        };
        self.correction_timeline_frame = self.current_timeline_frame(fps);
        self.mode = FieldMode::Correction;
        self.target_input_locked = true;
        self.correction_calculated = false;
        self.correction_suggested_offset = None;
        self.inferred_indices.clear();
        self.timer_start = None;
        self.timer_finished = false;
        if let Some(remaining) = remaining {
            self.timer_seconds = remaining.max(0.0);
        }
        self.clear_notice();
    }

    pub(super) fn rebuild_blink_timer_queue(&mut self, fps: f64) {
        let result = self.results.get(self.selected_result).cloned();

        let Some(result) = result else {
            self.blink_wait_queue.clear();
            return;
        };
        let current = self
            .current_timeline_frame(fps)
            .unwrap_or(result.last_wait_frame);
        let mut queue = self.cached_blink_waits_after(
            current,
            result.last_wait_frame,
            BLINK_TIMER_PREFETCH_COUNT,
        );
        if queue.is_empty()
            && result.timeline_cursor.is_none()
            && result.next_blink_wait_frame > 0
            && result
                .last_wait_frame
                .saturating_add(result.next_blink_wait_frame)
                > current
        {
            queue.push(result.next_blink_wait_frame);
        }
        self.blink_wait_queue = queue.into();
    }

    /// 先読みキューが空になる前に、既に生成済みのTimeline行を補充する。
    /// Timeline延長ワーカーが完了した次のUI更新でも同じ処理が走るため、
    /// 末尾到達を待ってから全キューを作り直すことはない。
    pub fn maintain_blink_timer_queue(&mut self, fps: f64) {
        if self.results.len() != 1 || self.mode == FieldMode::Correction {
            return;
        }
        // Targetタイマー中も次の瞬き表示を途切れさせないよう、消化済みの
        // 区間を捨てて先読みキューを補充する。NextBlink中は完了処理側が
        // 先頭を1件だけ取り除くため、ここでは追加だけを行う。
        if self.timer_kind == FieldTimerKind::Target {
            while self
                .blink_wait_queue
                .front()
                .is_some_and(|wait| self.remaining_from_observation(*wait, fps).is_err())
            {
                self.blink_wait_queue.pop_front();
            }
        }
        if self.blink_wait_queue.is_empty() {
            self.rebuild_blink_timer_queue(fps);
            return;
        }
        if self.blink_wait_queue.len() > BLINK_TIMER_REFILL_THRESHOLD {
            return;
        }
        let Some(result) = self.results.get(self.selected_result) else {
            return;
        };
        let last_queued_absolute = result
            .last_wait_frame
            .saturating_add(self.blink_wait_queue.back().copied().unwrap_or_default());
        let remaining = BLINK_TIMER_PREFETCH_COUNT.saturating_sub(self.blink_wait_queue.len());
        let additions =
            self.cached_blink_waits_after(last_queued_absolute, result.last_wait_frame, remaining);
        self.blink_wait_queue.extend(additions);
    }

    pub(super) fn cached_blink_waits_after(
        &self,
        absolute_frame: i64,
        anchor_frame: i64,
        limit: usize,
    ) -> Vec<i64> {
        if limit == 0 {
            return Vec::new();
        }
        let Some(shared) = &self.resolved_timeline_cache else {
            let Some(result) = self.results.get(self.selected_result) else {
                return Vec::new();
            };
            if result.timeline_cursor.is_some() {
                return Vec::new();
            }
            return result
                .timeline
                .iter()
                .filter(|row| row.timeline_frame > absolute_frame)
                .take(limit)
                .map(|row| row.timeline_frame - anchor_frame)
                .collect();
        };
        let Ok(cache) = shared.lock() else {
            return Vec::new();
        };
        let first = cache
            .blinks
            .partition_point(|event| i64::from(event.tick) * 2 <= absolute_frame);
        cache.blinks[first..]
            .iter()
            .take(limit)
            .map(|event| i64::from(event.tick) * 2 - anchor_frame)
            .collect()
    }

    /// Targetタイマー右上に表示する次の瞬きまでの残り時間。
    ///
    /// Target区間では瞬きタイマー自体を進めないため、表示専用の先読み
    /// キューから「まだ未来にある最初の区間」を選ぶ。区間の境界で古い
    /// 行が0秒になっても、次の行へ直ちに切り替わるので空表示にならない。
    pub fn preview_next_blink_remaining(&self, fps: f64) -> Option<f64> {
        for wait_frame in &self.blink_wait_queue {
            if let Ok(seconds) = self.remaining_from_observation(*wait_frame, fps) {
                return Some(seconds);
            }
        }
        let result = self.results.get(self.selected_result)?;
        let current = self.current_timeline_frame(fps)?;
        let wait_frame = self
            .cached_blink_waits_after(current, result.last_wait_frame, 1)
            .into_iter()
            .next()?;
        self.remaining_from_observation(wait_frame, fps).ok()
    }

    pub(super) fn start_blink_timer_from_queue(&mut self, fps: f64) -> AppResult<()> {
        let wait_frame = self
            .blink_wait_queue
            .front()
            .copied()
            .ok_or(AppError::InvalidState(StateIssue::BlinkQueuePending))?;
        self.timer_seconds = self.remaining_from_observation(wait_frame, fps)?;
        self.timer_kind = FieldTimerKind::NextBlink;
        self.timer_start = Some(Instant::now());
        self.timer_finished = false;
        Ok(())
    }

    /// 現在のタイマー区間が終わった後、先読み済みの次の区間へ切り替える。
    /// Timelineがまだ延長中なら、キューを再構築してから一度だけ再試行する。
    pub fn advance_blink_timer(&mut self, fps: f64) -> AppResult<()> {
        if self.timer_kind != FieldTimerKind::NextBlink || self.results.len() != 1 {
            return Err(AppError::InvalidState(StateIssue::BlinkTimerCannotContinue));
        }
        self.blink_wait_queue.pop_front();
        self.maintain_blink_timer_queue(fps);
        match self.start_blink_timer_from_queue(fps) {
            Ok(()) => Ok(()),
            Err(AppError::InvalidState(StateIssue::BlinkQueuePending)) => {
                self.rebuild_blink_timer_queue(fps);
                self.start_blink_timer_from_queue(fps)
            }
            Err(error) => Err(error),
        }
    }

    pub fn start_target_timer(&mut self, fps: f64) -> AppResult<()> {
        if self.mode == FieldMode::Correction {
            return Err(AppError::InvalidState(StateIssue::CorrectionModeLocksTimer));
        }
        if self.timeleap_enabled && !self.timeleap_timeline_adopted() {
            if self.timeleap_frame1_preview_is_pending()
                && self.timeleap_frame1_timer_available(fps)
                && self.timeleap_frame1_deadline.is_none()
            {
                self.timer_kind = FieldTimerKind::Target;
                self.timer_start = None;
                self.timer_finished = false;
                return Ok(());
            }
            let deadline = self
                .timeleap_frame1_deadline
                .filter(|_| self.timeleap_frame1_timer_available(fps))
                .ok_or(AppError::InvalidState(StateIssue::TargetTimerUnavailable))?;
            self.timer_kind = FieldTimerKind::Target;
            self.start_timeleap_frame1_timer(Instant::now(), fps);
            if deadline <= Instant::now() {
                self.timer_finished = true;
                self.timer_start = None;
                self.timer_seconds = 0.0;
            }
            return Ok(());
        }
        if self.results.len() != 1 {
            return Err(AppError::InvalidState(StateIssue::CandidatesNotUnique));
        }
        let result = self
            .results
            .get(self.selected_result)
            .cloned()
            .ok_or(AppError::InvalidState(StateIssue::CandidateUnavailable))?;
        if result.timeline_cursor.is_some() || result.target_cursor.is_some() {
            self.prepare_shared_timeline_cache()?;
        }
        let wait_frame = self
            .target_timer_wait_frame(&result)
            .ok_or(AppError::InvalidState(StateIssue::TargetUnreachable))?;
        self.timer_kind = FieldTimerKind::Target;
        match self.remaining_from_observation(wait_frame, fps) {
            Ok(seconds) => {
                self.next_blink_out_of_range = false;
                self.timer_seconds = seconds;
                self.timer_start = Some(Instant::now());
                self.timer_finished = false;
            }
            Err(_) => {
                // Target情報は残っているためTargetモードへは切り替えられる。
                // ただし既に通過しているので、0秒で固定して通知音は鳴らさない。
                self.timer_seconds = 0.0;
                self.timer_start = None;
                self.timer_finished = true;
            }
        }
        Ok(())
    }

    /// 表示オフセット更新後、実行中のTargetタイマーの基準時刻を取り直す。
    /// 瞬きタイマーと保持中のTimelineは変更しない。
    pub fn refresh_target_timer_for_offset(&mut self, fps: f64) {
        if self.timer_kind != FieldTimerKind::Target || self.timer_start.is_none() {
            return;
        }
        let Some(result) = self.results.get(self.selected_result) else {
            return;
        };
        let Some(wait_frame) = self.target_timer_wait_frame(result) else {
            return;
        };
        match self.remaining_from_observation(wait_frame, fps) {
            Ok(seconds) => {
                self.timer_seconds = seconds;
                self.timer_start = Some(Instant::now());
                self.timer_finished = false;
            }
            Err(_) => {
                self.timer_seconds = 0.0;
                self.timer_start = None;
                self.timer_finished = true;
            }
        }
    }

    pub fn toggle_timer_kind(&mut self, fps: f64) -> AppResult<()> {
        if self.mode == FieldMode::Correction {
            return Err(AppError::InvalidState(StateIssue::CorrectionModeLocksTimer));
        }
        if self.timeleap_waiting_for_blink_observation() {
            return Err(AppError::InvalidState(StateIssue::TargetTimerUnavailable));
        }
        let previous = self.timer_kind;
        let next = match previous {
            FieldTimerKind::NextBlink => FieldTimerKind::Target,
            FieldTimerKind::Target => FieldTimerKind::NextBlink,
        };
        if next == FieldTimerKind::Target && !self.target_timer_available(fps) {
            return Err(AppError::InvalidState(StateIssue::TargetTimerUnavailable));
        }
        if next == FieldTimerKind::Target
            && self.timeleap_enabled
            && !self.timeleap_timeline_adopted()
            && self.timeleap_frame1_preview_is_pending()
        {
            self.timer_kind = next;
            self.timer_start = None;
            self.timer_finished = false;
            return Ok(());
        }
        self.timer_kind = next;
        if self.timer_start.is_some() {
            let result = match self.timer_kind {
                FieldTimerKind::NextBlink => self.start_blink_timer(fps),
                FieldTimerKind::Target => self.start_target_timer(fps),
            };
            if result.is_err() {
                self.timer_kind = previous;
            }
            result
        } else {
            // 完了したTargetタイマーから戻る場合は表示だけでなく瞬きカウントダウンを
            // 再開する。表示だけを切り替えると`timer_start`がなく通知音を予約できない。
            self.timer_finished = false;
            if self.timer_kind == FieldTimerKind::NextBlink {
                match self.start_blink_timer(fps) {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        self.timer_kind = previous;
                        Err(error)
                    }
                }
            } else {
                Ok(())
            }
        }
    }

    pub fn target_timer_available(&mut self, fps: f64) -> bool {
        if self.timeleap_enabled && !self.timeleap_timeline_adopted() {
            return self.timeleap_frame1_timer_available(fps);
        }
        if self.results.len() != 1 {
            return false;
        }
        self.results
            .get(self.selected_result)
            .is_some_and(|result| self.target_timer_wait_frame(result).is_some())
    }

    /// Targetまでの表示Frameへ、フィールド専用オフセットを適用する。
    /// 候補のTarget到達判定・瞬き列・観測アンカーは変更しない。
    pub fn target_wait_frame_with_offset(&self, result: &FieldResult) -> Option<i64> {
        let wait = result.target_wait_frame?;
        if !self.use_offset {
            return Some(wait);
        }
        let trimmed = self.encounter_offset.trim();
        let offset = if trimmed.is_empty() {
            0
        } else {
            trimmed.parse::<i64>().ok()?
        };
        if !(-MAX_SFMT_FRAME..=MAX_SFMT_FRAME).contains(&offset) || offset % 2 != 0 {
            return None;
        }
        Some(wait.saturating_add(offset))
    }

    pub(super) fn target_timer_wait_frame(&self, result: &FieldResult) -> Option<i64> {
        self.target_wait_frame_with_offset(result)
    }

    pub fn target_is_passed(&self, fps: f64) -> bool {
        let Some(result) = self.results.get(self.selected_result) else {
            return false;
        };
        let Some(wait_frame) = self.target_timer_wait_frame(result) else {
            return result.target_exact == Some(false)
                && result.target_frame_before.is_some()
                && result.target_frame_after.is_some();
        };
        self.remaining_from_observation(wait_frame, fps).is_err()
    }

    /// Target情報がない場合は通常は瞬きへ戻す。直接エンカウントの採用後は
    /// 非同期計算待ちや到達不能の間も、明示的に選ばれたTarget表示を維持する。
    pub fn normalize_timer_kind(&mut self, fps: f64) {
        if self.timer_kind == FieldTimerKind::Target
            && !(self.timeleap_enabled
                && self.timeleap_direct_encounter
                && self.timeleap_timeline_adopted())
            && !(self.timeleap_enabled
                && !self.timeleap_timeline_adopted()
                && self.timeleap_frame1_preview_is_pending())
            && !self.target_timer_available(fps)
        {
            self.timer_kind = FieldTimerKind::NextBlink;
            if self.timer_start.is_none() && self.results.len() == 1 {
                let _ = self.start_blink_timer(fps);
            }
        }
    }

    pub fn timer_remaining(&self) -> Option<f64> {
        self.timer_remaining_at(Instant::now())
    }

    pub(crate) fn timer_remaining_at(&self, now: Instant) -> Option<f64> {
        self.timer_start.map(|start| {
            (self.timer_seconds - now.saturating_duration_since(start).as_secs_f64()).max(0.0)
        })
    }

    pub fn adjust_by(&mut self, delta: i64, fps: f64) {
        self.adjust = self.adjust.saturating_add(delta);
        let shift = game_frames_to_seconds(delta.unsigned_abs() as f64, fps);
        if let Some(deadline) = self.timeleap_frame1_deadline {
            let shift = Duration::from_secs_f64(shift);
            self.timeleap_frame1_deadline = Some(if delta.is_positive() {
                deadline.checked_sub(shift).unwrap_or(deadline)
            } else {
                deadline.checked_add(shift).unwrap_or(deadline)
            });
        }
        if let Some(start) = self.timer_start {
            let shifted = if delta.is_positive() {
                start.checked_sub(std::time::Duration::from_secs_f64(shift))
            } else {
                start.checked_add(std::time::Duration::from_secs_f64(shift))
            };
            self.timer_start = shifted.or(Some(start));
        }
    }

    pub fn preview_remaining(&self, kind: FieldTimerKind, fps: f64) -> Option<f64> {
        if self.timeleap_waiting_for_blink_observation() && kind == self.timer_kind {
            return Some(self.timer_seconds);
        }
        if self.timeleap_enabled && kind == FieldTimerKind::Target {
            if self.timeleap_timeline_adopted() {
                let result = self.results.get(self.selected_result)?;
                let wait_frame = self.target_timer_wait_frame(result)?;
                return self.remaining_from_observation(wait_frame, fps).ok();
            }
            if kind == self.timer_kind {
                if let Some(remaining) = self.timer_remaining() {
                    return Some(remaining);
                }
            }
            return self.timeleap_frame1_remaining_at(Instant::now(), fps);
        }
        if kind == self.timer_kind {
            if let Some(remaining) = self.timer_remaining() {
                return Some(remaining);
            }
            if self.timer_finished && kind == FieldTimerKind::Target {
                return Some(0.0);
            }
        }
        let result = self.results.get(self.selected_result)?;
        let wait_frame = match kind {
            FieldTimerKind::NextBlink => {
                let current = self.current_timeline_frame(fps)?;
                result
                    .timeline
                    .iter()
                    .find(|row| row.timeline_frame > current)
                    .map(|row| row.timeline_frame - result.last_wait_frame)?
            }
            FieldTimerKind::Target => self.target_timer_wait_frame(result)?,
        };
        self.remaining_from_observation(wait_frame, fps).ok()
    }

    pub(super) fn remaining_from_observation(&self, wait_frame: i64, fps: f64) -> AppResult<f64> {
        self.remaining_from_observation_at(wait_frame, fps, Instant::now())
    }

    pub(super) fn remaining_from_observation_at(
        &self,
        wait_frame: i64,
        fps: f64,
        now: Instant,
    ) -> AppResult<f64> {
        let anchor = self
            .timeleap_timeline_anchor
            .or_else(|| self.observed.last().copied())
            .ok_or(AppError::InvalidState(StateIssue::ObservationAnchorMissing))?;
        let unapplied_adjustment = self
            .timeleap_frame1_adjustment_consumed
            .map(|consumed| self.adjust.saturating_sub(consumed))
            .unwrap_or(self.adjust);
        let wait_frame = wait_frame.saturating_add(self.timeleap_assumed_transition_frames);
        let total = game_frames_to_seconds(wait_frame as f64, fps)
            - game_frames_to_seconds(unapplied_adjustment as f64, fps);
        let remaining = total - now.saturating_duration_since(anchor).as_secs_f64();
        if remaining <= 0.0 {
            return Err(AppError::InvalidState(StateIssue::TimerTargetPassed));
        }
        Ok(remaining)
    }

    pub fn current_timeline_frame(&self, fps: f64) -> Option<i64> {
        self.current_timeline_frame_at(fps, Instant::now())
    }

    pub(super) fn current_timeline_frame_at(&self, fps: f64, now: Instant) -> Option<i64> {
        let result = self.results.get(self.selected_result)?;
        if self.timeleap_waiting_for_blink_observation() {
            if let Some(frame) = self.timeleap_recheck_pause_frame {
                return Some(frame);
            }
        }
        if self.mode == FieldMode::Correction {
            if let Some(frame) = self.correction_timeline_frame {
                return Some(frame);
            }
        }
        let anchor = self
            .timeleap_timeline_anchor
            .or_else(|| self.observed.last().copied())?;
        Some(
            result.last_wait_frame
                + seconds_to_game_frames(now.saturating_duration_since(anchor).as_secs_f64(), fps)
                    .floor() as i64,
        )
    }

    pub fn current_timeline_row(&self, fps: f64) -> Option<usize> {
        let result = self.results.get(self.selected_result)?;
        let current = self.current_timeline_frame(fps)?;
        Some(
            result
                .timeline
                .partition_point(|row| row.timeline_frame <= current)
                .saturating_sub(1),
        )
    }
}
