use super::*;

impl FieldState {
    pub(super) fn resolved_timeline_identity(seed: u32, result: &FieldResult) -> TimelineIdentity {
        TimelineIdentity {
            seed,
            start_consumption: result.start_consumption,
            first_wait_frame: result.first_wait_frame,
            last_wait_frame: result.last_wait_frame,
            first_consumption: result.first_consumption,
            last_consumption: result.last_consumption,
            models: result.models,
            observed_model: result.observed_model,
            intervals: result.intervals.clone(),
            target_consumptions: result.target_consumptions.clone(),
        }
    }

    pub(super) fn ensure_resolved_timeline_cache(
        &mut self,
    ) -> AppResult<Arc<Mutex<ResolvedTimelineCache>>> {
        self.ensure_resolved_timeline_cache_cancelable(None)
    }

    pub(super) fn ensure_resolved_timeline_cache_cancelable(
        &mut self,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> AppResult<Arc<Mutex<ResolvedTimelineCache>>> {
        if cancel.is_some_and(|token| token.load(std::sync::atomic::Ordering::Acquire)) {
            return Err(AppError::Cancelled);
        }
        let seed = parse_seed(&self.seed).map_err(|_| AppError::InvalidSeed)?;
        let result = self
            .results
            .get(self.selected_result)
            .cloned()
            .ok_or(AppError::InvalidState(StateIssue::CandidateUnavailable))?;
        Self::ensure_resolved_timeline_cache_for(
            seed,
            &result,
            self.observation_cache.as_ref(),
            &mut self.resolved_timeline_cache,
            &mut self.timeline_projection_through_tick,
            cancel,
        )
    }

    pub(super) fn ensure_resolved_timeline_cache_for(
        seed: u32,
        result: &FieldResult,
        observation_cache: Option<&ObservationCache>,
        shared_cache: &mut Option<Arc<Mutex<ResolvedTimelineCache>>>,
        projection_through_tick: &mut i64,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> AppResult<Arc<Mutex<ResolvedTimelineCache>>> {
        if cancel.is_some_and(|token| token.load(std::sync::atomic::Ordering::Acquire)) {
            return Err(AppError::Cancelled);
        }
        let identity = Self::resolved_timeline_identity(seed, result);
        if let Some(shared) = shared_cache {
            let matches = shared
                .lock()
                .map(|cache| cache.identity == identity)
                .unwrap_or(false);
            if matches {
                return Ok(shared.clone());
            }
        }

        let cursor = result
            .target_cursor
            .as_ref()
            .or(result.timeline_cursor.as_ref())
            .cloned()
            .ok_or(AppError::InvalidState(
                StateIssue::TimelineCursorUnavailable,
            ))?;
        let cache = observation_cache
            .filter(|cache| cache.seed == seed)
            .ok_or(AppError::InvalidState(StateIssue::CacheUnavailable))?;
        let sfmt = cache
            .sfmt_at_checked(cursor.stream_cursor)
            .map_err(|error| match error {
                CacheReadError::OutOfRange => AppError::Cache(CacheError::OutOfRange),
                CacheReadError::Io { operation, source } => {
                    AppError::Cache(CacheError::Io { operation, source })
                }
            })?
            .ok_or(AppError::Cache(CacheError::StateUnavailable))?;
        let mut resolved =
            ResolvedTimelineCache::new(identity, cursor, sfmt, result.observed_model);
        let existing_end_tick = result
            .timeline_cursor
            .as_ref()
            .map(|cursor| cursor.tick)
            .unwrap_or(resolved.cursor.tick);
        while resolved.cursor.tick < existing_end_tick {
            if cancel.is_some_and(|token| token.load(std::sync::atomic::Ordering::Acquire)) {
                return Err(AppError::Cancelled);
            }
            let remaining = existing_end_tick.saturating_sub(resolved.cursor.tick) as usize;
            if resolved
                .append_chunk_cancelable(remaining.min(RESOLVED_TIMELINE_CHECKPOINT_TICKS), cancel)
                == 0
            {
                break;
            }
        }
        if cancel.is_some_and(|token| token.load(std::sync::atomic::Ordering::Acquire)) {
            return Err(AppError::Cancelled);
        }
        *projection_through_tick = existing_end_tick;
        let shared = Arc::new(Mutex::new(resolved));
        *shared_cache = Some(shared.clone());
        Ok(shared)
    }

    pub(crate) fn prepare_shared_timeline_cache(&mut self) -> AppResult<()> {
        self.ensure_resolved_timeline_cache().map(|_| ())
    }

    pub(crate) fn retarget_confirmed_result_from_cache(
        &mut self,
        target: i64,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> AppResult<FieldResult> {
        let result = self
            .results
            .get(self.selected_result)
            .cloned()
            .ok_or(AppError::InvalidState(StateIssue::CandidateUnavailable))?;
        if target < result.start_consumption || target < result.last_consumption {
            let mut result = result;
            result.target_wait_frame = None;
            result.target_exact = Some(false);
            result.target_frame_before = Some(target);
            result.target_frame_after = Some(result.last_consumption);
            result.target_blink_count = None;
            return Ok(result);
        }
        let shared = self.ensure_resolved_timeline_cache_cancelable(cancel)?;
        loop {
            if cancel.is_some_and(|token| token.load(std::sync::atomic::Ordering::Acquire)) {
                return Err(AppError::Cancelled);
            }
            let Ok(mut cache) = shared.lock() else {
                return Err(AppError::InvalidState(
                    StateIssue::TimelineCursorUnavailable,
                ));
            };
            if let Some(timing) = cache.target_timing(target) {
                return Ok(crate::application::field::apply_cached_target_timing(
                    result,
                    target,
                    Some(timing),
                ));
            }
            let appended =
                cache.append_chunk_cancelable(RESOLVED_TIMELINE_LOCK_BATCH_TICKS, cancel);
            drop(cache);
            std::thread::yield_now();
            if appended == 0 {
                return Ok(crate::application::field::apply_cached_target_timing(
                    result, target, None,
                ));
            }
        }
    }

    /// 検索始点から指定幅を足して、検索終点を設定する。
    pub fn set_range_from_start(&mut self) {
        let Ok(start) = self.range_start.trim().parse::<i64>() else {
            return;
        };
        let Ok(span) = self.range_after_start.trim().parse::<i64>() else {
            return;
        };
        if !(0..=MAX_SEARCH_FRAME).contains(&start) || !(0..=MAX_SEARCH_SPAN).contains(&span) {
            return;
        }
        let Some(end) = start.checked_add(span) else {
            return;
        };
        if end > MAX_SEARCH_FRAME {
            return;
        }
        self.range_end = end.to_string();
    }

    pub(crate) fn take_timeline_stream_error(&mut self) -> Option<std::sync::Arc<AppError>> {
        self.timeline_stream_error.take()
    }

    /// 共有済みTimelineのtick消費差分から、現在のSFMT消費位置を引く。
    /// UI更新のたびにModelStatusやSFMTを別カーソルで再実行しない。
    pub fn current_sfmt_frame(&mut self, fps: f64) -> Option<i64> {
        self.current_sfmt_frame_at(fps, Instant::now())
    }

    pub(super) fn current_sfmt_frame_at(&mut self, fps: f64, now: Instant) -> Option<i64> {
        let current_frame = self.current_timeline_frame_at(fps, now)?;
        let shared = match self.resolved_timeline_cache.clone() {
            Some(shared) => shared,
            None => match self.ensure_resolved_timeline_cache() {
                Ok(shared) => shared,
                Err(error) => {
                    self.timeline_stream_error = Some(Arc::new(error));
                    return None;
                }
            },
        };
        let current = match shared.try_lock() {
            Ok(cache) => {
                let frame = cache
                    .consumption_at_tick(current_frame.max(cache.anchor_tick * 2) / 2)
                    .min(crate::domain::rng::MAX_SFMT_FRAME);
                self.last_current_sfmt_frame = Some(frame);
                Some(frame)
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                self.last_current_sfmt_frame.or_else(|| {
                    self.results
                        .get(self.selected_result)
                        .map(|result| result.last_consumption)
                })
            }
            Err(std::sync::TryLockError::Poisoned(_)) => None,
        };
        current
    }

    /// UIスレッドでTimeline延長を開始する前の軽量な判定。
    /// 実際のModelStatus/SFMT走査は呼び出し側のワーカーで行う。
    pub(crate) fn timeline_extension_needed(&self, fps: f64) -> bool {
        if self.results.len() != 1 || self.is_correction_mode() {
            return false;
        }
        let Some(current) = self.current_timeline_frame(fps) else {
            return false;
        };
        if self.results.get(self.selected_result).is_none() {
            return false;
        }
        let current_tick = current.div_euclid(2);
        let display_horizon = current_tick.saturating_add(TIMELINE_CHUNK_TICKS * 2);
        let visible_timeline_needs_extension =
            self.timeline_projection_through_tick < display_horizon;
        let frame1_needs_extension = self
            .active_timeleap_frame1_target_consumption()
            .is_some_and(|frame1| {
                self.results[self.selected_result]
                    .frame_boundary_index(frame1)
                    .is_none()
                    && self.timeleap_frame1_table_boundary(frame1).is_none()
            });
        visible_timeline_needs_extension || frame1_needs_extension
    }

    #[cfg(test)]
    pub(crate) fn extend_timeline_if_needed(&mut self, fps: f64) -> AppResult<bool> {
        let Some(request) = self.timeline_extension_request() else {
            return Ok(false);
        };
        let extension = request.extend(fps, None)?;
        Ok(self.apply_timeline_extension(extension).unwrap_or(false))
    }

    #[cfg(test)]
    pub(crate) fn timeline_extension_result_for_test(
        &self,
        safe_frame: Option<i64>,
    ) -> TimelineExtensionResult {
        let result = self
            .results
            .get(self.selected_result)
            .expect("selected result exists");
        let seed = parse_seed(&self.seed).expect("valid seed");
        TimelineExtensionResult {
            identity: Self::resolved_timeline_identity(seed, result),
            timeleap_mode: self.timeleap_mode,
            result: result.timeline_extension_snapshot(),
            resolved_timeline_cache: self.resolved_timeline_cache.clone(),
            projection_through_tick: self.timeline_projection_through_tick,
            safe_frame,
            frame1_table_boundary: None,
            can_continue: true,
        }
    }

    /// Build an immutable, narrowly-scoped timeline extension request.
    pub(crate) fn timeline_extension_request(&self) -> Option<TimelineExtensionRequest> {
        if self.results.len() != 1 || self.is_correction_mode() {
            return None;
        }
        let seed = parse_seed(&self.seed).ok()?;
        let selected = self.results.get(self.selected_result)?;
        Some(TimelineExtensionRequest {
            identity: Self::resolved_timeline_identity(seed, selected),
            timeleap_mode: self.timeleap_mode,
            result: selected.timeline_extension_snapshot(),
            observation_cache: self.observation_cache.clone(),
            resolved_timeline_cache: self.resolved_timeline_cache.clone(),
            projection_through_tick: self.timeline_projection_through_tick,
            timeleap_stage: self.timeleap_stage,
            timeleap_timeline_anchor: self.timeleap_timeline_anchor,
            timeleap_recheck_pause_frame: self.timeleap_recheck_pause_frame,
            observation_anchor: self.observed.last().copied(),
            frame1_target_consumption: self.active_timeleap_frame1_target_consumption(),
        })
    }

    /// バックグラウンドで延長した状態のうち、Timeline関連だけを反映する。
    /// 入力欄・観測列・タイマー状態はUI側の最新値を保持する。
    pub(crate) fn apply_timeline_extension(
        &mut self,
        extension: TimelineExtensionResult,
    ) -> Option<bool> {
        if self.results.len() != 1
            || self.selected_result != 0
            || self.timeleap_mode != extension.timeleap_mode
        {
            return None;
        }
        let Ok(seed) = parse_seed(&self.seed) else {
            return None;
        };
        let Some(current_result) = self.results.get(self.selected_result) else {
            return None;
        };
        if Self::resolved_timeline_identity(seed, current_result) != extension.identity {
            return None;
        }
        let shared_cursor = extension
            .resolved_timeline_cache
            .as_ref()
            .and_then(|shared| shared.lock().ok().map(|cache| cache.cursor.clone()));
        let previous_safe_frame = self.timeleap_safe_frame_display;
        let Some(current) = self.results.get_mut(self.selected_result) else {
            return None;
        };
        current.timeline = extension.result.timeline;
        current.timeline_cursor = shared_cursor.or(extension.result.timeline_cursor);
        current.next_blink_wait_frame = extension.result.next_blink_wait_frame;
        if let Some(shared) = extension.resolved_timeline_cache {
            self.resolved_timeline_cache = Some(shared);
        }
        self.timeline_projection_through_tick = extension.projection_through_tick;
        self.timeleap_safe_frame_display = extension.safe_frame;
        self.timeleap_frame1_table_boundary = extension.frame1_table_boundary;
        if previous_safe_frame.is_none()
            && self.timeleap_safe_frame_display.is_some()
            && self.timeleap_timeline_adopted()
        {
            self.sync_timeleap_source_inputs_from_internal_timeline();
        }
        Some(extension.can_continue)
    }

    pub fn observe(&mut self, fps: f64) {
        if !self.is_observation_input_enabled() {
            return;
        }
        if self.observed.len() >= crate::application::state::MAX_BLINK_OBSERVATIONS {
            return;
        }
        let now = Instant::now();
        self.observed.push(now);
        if let Some(previous) = self.observed.iter().rev().nth(1) {
            self.intervals.push(
                seconds_to_game_frames(now.duration_since(*previous).as_secs_f64(), fps).round()
                    as i64,
            );
        }
    }

    /// 一致済み候補を観測した間隔1つ分だけ延長する。
    ///
    /// 候補には最後に一致した瞬き直後の状態が保存されているため、次の瞬きまで
    /// 進めればよく、Shift入力ごとにSFMT検索全体をやり直さずに済む。
    pub fn advance_candidates_by_interval(&mut self, expected: i64) -> usize {
        let tolerance = self.tolerance.trim().parse::<i64>().ok().unwrap_or(-1);
        if tolerance < 0 || self.results.is_empty() {
            return 0;
        }
        let mut survivors = Vec::with_capacity(self.results.len());
        for mut result in self.results.drain(..) {
            let previous_last = result.last_wait_frame;
            let Some(next_index) = result
                .candidate_blinks
                .iter()
                .position(|blink| blink.timeline_frame > previous_last && !blink.observed)
            else {
                continue;
            };
            let next = {
                let blink = &mut result.candidate_blinks[next_index];
                (blink.timeline_frame, blink.frame, blink.target_consumption)
            };
            let (next_wait, next_consumption, next_target_consumption) = next;
            let actual = next_wait.saturating_sub(previous_last);
            if (actual - expected).abs() > tolerance {
                continue;
            }

            let target_absolute = result
                .target_wait_frame
                .map(|wait| previous_last.saturating_add(wait));
            result.candidate_blinks[next_index].observed = true;
            result.last_wait_frame = next_wait;
            result.last_consumption = next_consumption;
            result.target_blink_count_offset_from_first_row = result
                .target_blink_count_offset_from_first_row
                .saturating_add(1);
            result.intervals.push(actual);
            if let Some(target_consumption) = next_target_consumption {
                result.target_consumptions.push(target_consumption);
            }
            result.next_blink_wait_frame = result
                .candidate_blinks
                .iter()
                .find(|blink| blink.timeline_frame > next_wait)
                .map(|blink| blink.timeline_frame - next_wait)
                .unwrap_or_default();
            result.target_wait_frame = target_absolute
                .and_then(|absolute| (absolute >= next_wait).then_some(absolute - next_wait));
            if let Some(absolute) = target_absolute {
                result.target_blink_count = Some(if next_wait < absolute {
                    result
                        .target_blink_count
                        .unwrap_or_default()
                        .saturating_sub(1)
                } else {
                    0
                });
            }
            survivors.push(result);
        }
        self.results = survivors;
        if self.results.is_empty() {
            self.clear_timeleap_frame1_preview();
        }
        if self.results.len() == 1 {
            self.results[0].candidate_blinks.clear();
        }
        self.selected_result = 0;
        self.table_last_followed_row = None;
        self.refresh_nearest_timeline_frame();
        self.results.len()
    }

    pub fn start_blink_timer(&mut self, fps: f64) -> AppResult<()> {
        if self.mode == FieldMode::Correction {
            return Err(AppError::InvalidState(StateIssue::CorrectionModeLocksTimer));
        }
        if self.results.len() != 1 {
            return Err(AppError::InvalidState(StateIssue::CandidatesNotUnique));
        }
        if self.timeline_limit_reached() {
            self.stop_blink_timer_out_of_range();
            return Err(AppError::InvalidState(StateIssue::TimelineLimitReached));
        }
        if self
            .results
            .get(self.selected_result)
            .is_some_and(|result| {
                result.timeline_cursor.is_some() || result.target_cursor.is_some()
            })
        {
            self.prepare_shared_timeline_cache()?;
        }
        self.rebuild_blink_timer_queue(fps);
        if self.blink_wait_queue.is_empty() && self.timeline_limit_reached() {
            self.stop_blink_timer_out_of_range();
            return Err(AppError::InvalidState(StateIssue::TimelineLimitReached));
        }
        self.start_blink_timer_from_queue(fps)?;
        self.next_blink_out_of_range = false;
        Ok(())
    }
}

/// 表示用の履歴だけを上限内に保つ。Timeline継続カーソルや観測結果の要約には触れない。
pub(super) fn trim_timeline_history(result: &mut FieldResult) {
    let excess = result
        .timeline
        .len()
        .saturating_sub(crate::application::field::MAX_RETAINED_TIMELINE_ROWS);
    if excess > 0 {
        result.timeline.drain(..excess);
        result.target_blink_count_offset_from_first_row = result
            .target_blink_count_offset_from_first_row
            .saturating_sub(excess as i64);
    }
}

impl TimelineExtensionRequest {
    fn current_timeline_frame(&self, fps: f64, now: Instant) -> Option<i64> {
        if matches!(
            self.timeleap_stage,
            TimeleapStage::ShiftWaiting | TimeleapStage::Failed | TimeleapStage::Recheck
        ) {
            if let Some(frame) = self.timeleap_recheck_pause_frame {
                return Some(frame);
            }
        }
        let anchor = self.timeleap_timeline_anchor.or(self.observation_anchor)?;
        Some(
            self.result.last_wait_frame
                + seconds_to_game_frames(now.saturating_duration_since(anchor).as_secs_f64(), fps)
                    .floor() as i64,
        )
    }

    pub(crate) fn extend(
        mut self,
        fps: f64,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> AppResult<TimelineExtensionResult> {
        if cancel.is_some_and(|token| token.load(std::sync::atomic::Ordering::Acquire)) {
            return Err(AppError::Cancelled);
        }
        let shared = FieldState::ensure_resolved_timeline_cache_for(
            self.identity.seed,
            &self.result,
            self.observation_cache.as_ref(),
            &mut self.resolved_timeline_cache,
            &mut self.projection_through_tick,
            cancel,
        )?;
        let current = self
            .current_timeline_frame(fps, Instant::now())
            .ok_or(AppError::InvalidState(StateIssue::ObservationAnchorMissing))?;
        let display_end_tick = current
            .div_euclid(2)
            .saturating_add(TIMELINE_CHUNK_TICKS * 2);
        let ticks_needed = match shared.lock() {
            Ok(cache) => {
                let display_ticks = display_end_tick.saturating_sub(cache.cursor.tick).max(0);
                let frame1_ticks = self
                    .frame1_target_consumption
                    .is_some_and(|frame1| cache.cursor.consumption < frame1)
                    .then_some(TIMELINE_CHUNK_TICKS)
                    .unwrap_or_default();
                display_ticks.max(frame1_ticks) as usize
            }
            Err(_) => {
                return Err(AppError::InvalidState(
                    StateIssue::TimelineCursorUnavailable,
                ))
            }
        };
        let mut ticks_remaining = ticks_needed.min(TIMELINE_CHUNK_TICKS as usize);
        while ticks_remaining > 0 {
            if cancel.is_some_and(|token| token.load(std::sync::atomic::Ordering::Acquire)) {
                return Err(AppError::Cancelled);
            }
            let Ok(mut cache) = shared.lock() else {
                return Err(AppError::InvalidState(
                    StateIssue::TimelineCursorUnavailable,
                ));
            };
            let appended = cache.append_chunk_cancelable(
                ticks_remaining.min(RESOLVED_TIMELINE_LOCK_BATCH_TICKS),
                cancel,
            );
            drop(cache);
            if cancel.is_some_and(|token| token.load(std::sync::atomic::Ordering::Acquire)) {
                return Err(AppError::Cancelled);
            }
            std::thread::yield_now();
            if appended == 0 {
                break;
            }
            ticks_remaining = ticks_remaining.saturating_sub(appended);
        }
        let Ok(cache) = shared.lock() else {
            return Err(AppError::InvalidState(
                StateIssue::TimelineCursorUnavailable,
            ));
        };
        self.projection_through_tick = self
            .projection_through_tick
            .max(display_end_tick.min(cache.cursor.tick));
        let last_row_tick = self
            .result
            .timeline
            .last()
            .map_or(self.result.last_wait_frame / 2, |row| {
                row.timeline_frame / 2
            });
        let first_event = cache
            .blinks
            .partition_point(|event| i64::from(event.tick) <= last_row_tick);
        let events = cache.blinks[first_event..]
            .iter()
            .take_while(|event| i64::from(event.tick) <= display_end_tick)
            .take(MAX_RETAINED_TIMELINE_ROWS + 1)
            .map(|event| {
                let tick = i64::from(event.tick);
                (tick * 2, cache.consumption_at_tick(tick))
            })
            .collect::<Vec<_>>();
        let cursor = cache.cursor.clone();
        let frame1_future_blink_count = self.frame1_target_consumption.and_then(|frame1| {
            (cursor.consumption >= frame1).then(|| {
                let last_visible_consumption = events
                    .last()
                    .map(|(_, consumption)| *consumption)
                    .or_else(|| self.result.timeline.last().map(|row| row.frame))
                    .unwrap_or(self.result.last_consumption);
                let future_blinks = if frame1 > last_visible_consumption {
                    let first = cache
                        .blinks
                        .partition_point(|event| event.consumption <= last_visible_consumption);
                    let end = cache
                        .blinks
                        .partition_point(|event| event.consumption <= frame1);
                    end.saturating_sub(first)
                } else {
                    0
                };
                (frame1, future_blinks)
            })
        });
        let safe_frame = cache.safe_frame;
        let cache_at_limit =
            cursor.tick >= MAX_FUTURE_TIMELINE_TICKS || cursor.consumption >= MAX_SFMT_FRAME;
        let can_continue = !cache_at_limit || self.projection_through_tick < cursor.tick;
        drop(cache);
        for (wait_frame, consumption) in events {
            if let Some(previous) = self.result.timeline.last_mut() {
                previous.duration_frames = wait_frame.saturating_sub(previous.timeline_frame);
            }
            self.result.timeline.push(FieldTimelineRow {
                frame: consumption,
                timeline_frame: wait_frame,
                duration_frames: 0,
            });
        }
        // Targetまでの瞬き数はTarget判定時に確定済み。表示用の先読みでは再加算しない。
        trim_timeline_history(&mut self.result);
        let frame1_table_boundary = frame1_future_blink_count.map(|(frame1, future_blinks)| {
            let visible_boundary = self
                .result
                .timeline
                .partition_point(|row| row.frame <= frame1);
            let frame_is_after_visible_rows = self
                .result
                .timeline
                .last()
                .is_none_or(|row| row.frame < frame1);
            (
                frame1,
                visible_boundary.saturating_add(if frame_is_after_visible_rows {
                    future_blinks
                } else {
                    0
                }),
            )
        });
        self.result.timeline_cursor = Some(cursor.clone());
        if let Some(observation_cache) = self
            .observation_cache
            .as_ref()
            .filter(|cache| cache.seed == self.identity.seed)
        {
            let required_end = cursor
                .stream_cursor
                .checked_add(311)
                .unwrap_or(crate::domain::rng::MAX_SFMT_FRAME)
                .min(crate::domain::rng::MAX_SFMT_FRAME);
            observation_cache.ensure_through(required_end, cancel)?;
        }
        Ok(TimelineExtensionResult {
            identity: self.identity,
            timeleap_mode: self.timeleap_mode,
            result: self.result,
            resolved_timeline_cache: Some(shared),
            projection_through_tick: self.projection_through_tick,
            safe_frame,
            frame1_table_boundary,
            can_continue,
        })
    }
}
