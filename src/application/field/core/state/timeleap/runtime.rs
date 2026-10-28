use super::*;

impl FieldState {
    pub(crate) fn timeleap_shift_resolution_pending(&self) -> bool {
        self.timeleap_shift_resolution_pending
    }

    pub(crate) fn timeleap_recheck_position_radius_active(&self) -> bool {
        self.timeleap_recheck_interval_tolerance.is_some()
    }

    pub(crate) fn tolerance_for_persistence(&self) -> &str {
        self.timeleap_recheck_interval_tolerance
            .as_deref()
            .unwrap_or(&self.tolerance)
    }

    pub(super) fn activate_timeleap_recheck_interval_tolerance(&mut self) {
        if self.timeleap_recheck_interval_tolerance.is_none() {
            self.timeleap_recheck_interval_tolerance = Some(self.tolerance.clone());
            self.tolerance = DEFAULT_TIMELEAP_RECHECK_POSITION_RADIUS.to_string();
        }
    }

    pub(in crate::application::field::core::state) fn restore_timeleap_recheck_interval_tolerance(
        &mut self,
    ) {
        if let Some(tolerance) = self.timeleap_recheck_interval_tolerance.take() {
            self.tolerance = tolerance;
        }
    }

    pub(crate) fn timeleap_recheck_intervals(&self, fps: f64) -> Option<Vec<i64>> {
        self.timeleap_waiting_for_blink_observation().then(|| {
            self.timeleap_shift_times
                .windows(2)
                .map(|pair| {
                    crate::domain::timing::seconds_to_game_frames(
                        pair[1].saturating_duration_since(pair[0]).as_secs_f64(),
                        fps,
                    )
                    .round() as i64
                })
                .collect()
        })
    }

    pub fn timeleap_frame1_waiting(&self) -> bool {
        self.timeleap_lifecycle_policy().phase == TimeleapLifecyclePhase::Frame1Waiting
    }

    /// Frame1のカウントダウンだけを取り消し、観測済みTimelineを保って入力可能に戻す。
    pub fn cancel_timeleap_ready(&mut self) {
        if self.timeleap_stage != TimeleapStage::Frame1Waiting {
            return;
        }
        self.timeleap_stage = TimeleapStage::Off;
        self.timeleap_locked = false;
        self.target_input_locked = false;
        self.timeleap_execution_candidates.clear();
        self.timeleap_nominal_tick = None;
        self.timeleap_frame1_target = None;
        self.timeleap_selected_frame = None;
        self.timeleap_selected_sfmt_position = None;
        self.timeleap_confirmed_frame1 = None;
        self.timeleap_confirmed_candidate = None;
        self.cancel_timeleap_shift_resolution();
        self.timeleap_shift_times.clear();
        self.timeleap_wait_started = None;
        self.timeleap_observation_anchor_frame = None;
        self.clear_notice();
    }

    pub fn tick_timeleap_frame1(&mut self, now: Instant) -> bool {
        if self.timeleap_stage != TimeleapStage::Frame1Waiting {
            return false;
        }
        if self
            .timeleap_frame1_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            let deadline = self.timeleap_frame1_deadline.unwrap_or(now);
            self.timer_seconds = 0.0;
            self.timer_start = None;
            self.timer_finished = false;
            self.timeleap_stage = TimeleapStage::Assumed;
            self.timeleap_wait_started = Some(deadline);
            self.clear_notice();
            return true;
        }
        false
    }

    pub fn timeleap_waiting_for_blink_observation(&self) -> bool {
        matches!(
            self.timeleap_stage,
            TimeleapStage::ShiftWaiting | TimeleapStage::Failed | TimeleapStage::Recheck
        )
    }

    pub fn record_timeleap_shift(&mut self, at: Instant) -> bool {
        if !self.timeleap_accepts_shift() {
            return false;
        }
        if self.timeleap_shift_times.len() >= 32 {
            self.timeleap_stage = TimeleapStage::Failed;
            return false;
        }
        self.timeleap_shift_times.push(at);
        true
    }

    pub(crate) fn prepare_timeleap_shift_resolution(
        &mut self,
        at: Instant,
        fps: f64,
    ) -> Result<Option<TimeleapShiftResolutionRequest>, String> {
        if !self.timeleap_accepts_shift() {
            return Ok(None);
        }
        if !self.record_timeleap_shift(at) {
            return Ok(None);
        }
        self.clear_notice();
        self.timeleap_shift_resolution_generation =
            self.timeleap_shift_resolution_generation.wrapping_add(1);
        let generation = self.timeleap_shift_resolution_generation;
        let validation = (|| {
            if !fps.is_finite() || fps <= 0.0 {
                return Err("FPSを確認してください。".to_owned());
            }
            let started = self
                .timeleap_wait_started
                .ok_or_else(|| "瞬き再確認の開始時刻がありません。".to_owned())?;
            let tolerance = self
                .tolerance
                .trim()
                .parse::<i64>()
                .ok()
                .filter(|value| (0..=crate::domain::rng::MAX_SFMT_FRAME).contains(value))
                .ok_or_else(|| "予測現在地の±範囲（F）を確認してください。".to_owned())?;
            let model = self
                .results
                .get(self.selected_result)
                .map(|result| result.observed_model)
                .ok_or_else(|| "観測済みのNPC番号がありません。".to_owned())?;
            let seed = parse_seed(&self.seed)?;
            let cache = self
                .observation_cache
                .as_ref()
                .filter(|cache| cache.seed == seed)
                .cloned()
                .ok_or_else(|| "現在のSeedに対応するSFMTキャッシュがありません。".to_owned())?;
            let observation_anchor_frame = self
                .timeleap_observation_anchor_frame
                .ok_or_else(|| "瞬き再確認の基準Frameがありません。".to_owned())?;
            let nominal_tick = self
                .timeleap_nominal_tick
                .ok_or_else(|| "Frame1の基準tickがありません。".to_owned())?;
            let recheck_range_frames = self
                .timeleap_search_range
                .trim()
                .parse::<i64>()
                .ok()
                .filter(|range| (0..=98).contains(range))
                .ok_or_else(|| "再検索範囲は0～98Fで指定してください。".to_owned())?;
            if self.timeleap_execution_candidates.is_empty()
                || self
                    .timeleap_execution_candidates
                    .iter()
                    .all(|candidate| model >= candidate.post_status.model.remain.len())
            {
                return Err("瞬き再確認に使えるFrame1候補がありません。".to_owned());
            }
            let observed = self
                .timeleap_shift_times
                .iter()
                .map(|shift| {
                    let elapsed = (shift.duration_since(started).as_secs_f64() * fps).round();
                    observation_anchor_frame.saturating_add(elapsed as i64)
                })
                .collect::<Vec<_>>();
            let last = observed
                .last()
                .copied()
                .ok_or_else(|| "瞬き入力がありません。".to_owned())?;
            let until_frame = last.saturating_sub(self.adjust).saturating_add(tolerance);
            Ok(TimeleapShiftResolutionRequest {
                generation,
                cache,
                candidates: self.timeleap_execution_candidates.clone(),
                model,
                observed,
                correction: self.adjust,
                tolerance,
                until_frame,
                nominal_tick,
                recheck_range_frames,
                selected_frame1: self.timeleap_selected_frame,
            })
        })();
        let request = match validation {
            Ok(request) => request,
            Err(message) => {
                self.timeleap_shift_resolution_pending = false;
                self.timeleap_stage = TimeleapStage::Failed;
                self.notice = Some(message.clone());
                return Err(message);
            }
        };
        self.timeleap_shift_resolution_pending = true;
        Ok(Some(request))
    }

    pub(crate) fn apply_timeleap_shift_resolution(
        &mut self,
        generation: u64,
        summary: TimeleapShiftResolutionSummary,
    ) -> Option<TimeleapStage> {
        if generation != self.timeleap_shift_resolution_generation
            || !self.timeleap_shift_resolution_pending
        {
            return None;
        }
        self.timeleap_shift_resolution_pending = false;
        if summary.matches_found && !summary.multiple_future_states {
            self.timeleap_confirmed_frame1 = summary.confirmed_frame1;
            if summary.confirmed_frame1.is_some() {
                self.sync_timeleap_source_frame1_from_reset_value();
            }
            self.timeleap_confirmed_candidate = summary.confirmed_candidate;
            self.timeleap_stage = TimeleapStage::Succeeded;
        } else if !summary.matches_found || self.timeleap_shift_times.len() >= 32 {
            self.timeleap_stage = TimeleapStage::Failed;
        }
        Some(self.timeleap_stage)
    }

    pub(crate) fn cancel_timeleap_shift_resolution(&mut self) {
        self.timeleap_shift_resolution_generation =
            self.timeleap_shift_resolution_generation.wrapping_add(1);
        self.timeleap_shift_resolution_pending = false;
    }

    #[cfg(test)]
    pub fn resolve_timeleap_shift(&mut self, at: Instant, fps: f64) -> TimeleapStage {
        let Ok(Some(request)) = self.prepare_timeleap_shift_resolution(at, fps) else {
            return self.timeleap_stage;
        };
        let generation = request.generation;
        let summary = request.resolve(None).unwrap_or_default();
        self.apply_timeleap_shift_resolution(generation, summary)
            .unwrap_or(self.timeleap_stage)
    }

    pub fn timeleap_failed(&self) -> bool {
        self.timeleap_stage == TimeleapStage::Failed
    }

    pub fn timeleap_succeeded(&self) -> bool {
        self.timeleap_stage == TimeleapStage::Succeeded
    }

    pub fn timeleap_assumed(&self) -> bool {
        self.timeleap_stage == TimeleapStage::Assumed
    }

    pub fn timeleap_frame1_has_passed(&self) -> bool {
        self.timeleap_lifecycle_policy().frame1_has_passed
    }

    pub fn can_edit_timeleap_research_range(&self) -> bool {
        matches!(
            self.timeleap_lifecycle_policy().phase,
            TimeleapLifecyclePhase::Frame1Failed
                | TimeleapLifecyclePhase::PostTimeleapAssumed
                | TimeleapLifecyclePhase::PostTimeleapRecheckFailed
                | TimeleapLifecyclePhase::PostTimeleapConfirmed
        )
    }

    pub fn timeleap_timeline_adopted(&self) -> bool {
        self.timeleap_timeline_anchor.is_some()
    }

    #[cfg(test)]
    pub(crate) fn set_timeleap_timeline_anchor_for_test(
        &mut self,
        anchor: Option<Instant>,
        original_starting_frame: Option<i64>,
    ) {
        self.timeleap_timeline_anchor = anchor;
        self.timeleap_original_starting_frame = original_starting_frame;
    }

    pub(crate) fn timeleap_lifecycle_policy(&self) -> TimeleapLifecyclePolicy {
        let adopted = self.timeleap_timeline_adopted();
        let phase = if !self.timeleap_enabled {
            TimeleapLifecyclePhase::Disabled
        } else if adopted {
            match self.timeleap_stage {
                TimeleapStage::ShiftWaiting | TimeleapStage::Recheck => {
                    TimeleapLifecyclePhase::PostTimeleapRechecking
                }
                TimeleapStage::Failed => TimeleapLifecyclePhase::PostTimeleapRecheckFailed,
                TimeleapStage::Succeeded => TimeleapLifecyclePhase::PostTimeleapConfirmed,
                _ => TimeleapLifecyclePhase::PostTimeleapAssumed,
            }
        } else if self.timeleap_frame1_reachable == Some(false) {
            TimeleapLifecyclePhase::Frame1Unreachable
        } else {
            match self.timeleap_stage {
                TimeleapStage::Frame1Waiting => TimeleapLifecyclePhase::Frame1Waiting,
                TimeleapStage::Assumed => TimeleapLifecyclePhase::PostTimeleapAssumed,
                TimeleapStage::ShiftWaiting | TimeleapStage::Recheck => {
                    TimeleapLifecyclePhase::PostTimeleapRechecking
                }
                TimeleapStage::Succeeded => TimeleapLifecyclePhase::PostTimeleapConfirmed,
                TimeleapStage::Failed => TimeleapLifecyclePhase::Frame1Failed,
                TimeleapStage::Off
                    if self.timeleap_frame1_preparing
                        || self.timeleap_frame1_preview_is_pending() =>
                {
                    TimeleapLifecyclePhase::Frame1Preparing
                }
                _ => TimeleapLifecyclePhase::Frame1Setup,
            }
        };
        let table_mode = if self.timeleap_enabled && !adopted {
            TimeleapTableMode::Frame1
        } else {
            TimeleapTableMode::Target
        };
        let frame1_boundary_target = (table_mode == TimeleapTableMode::Frame1
            && self.timeleap_frame1_reachable != Some(false))
        .then(|| self.timeleap_frame1_input.trim().parse::<i64>().ok())
        .flatten()
        .filter(|frame| (0..=crate::domain::rng::MAX_SFMT_FRAME).contains(frame));
        let summary_frame1 = adopted
            .then(|| match self.timeleap_stage {
                TimeleapStage::Assumed => self
                    .timeleap_selected_frame
                    .or(self.timeleap_confirmed_frame1),
                _ => self
                    .timeleap_confirmed_frame1
                    .or(self.timeleap_selected_frame),
            })
            .flatten();
        let frame1_input_enabled = self.timeleap_enabled
            && !self.timeleap_locked
            && !self.timeleap_frame1_preparing
            && table_mode == TimeleapTableMode::Frame1;
        let frame1_has_passed = matches!(
            phase,
            TimeleapLifecyclePhase::Frame1Failed
                | TimeleapLifecyclePhase::PostTimeleapAssumed
                | TimeleapLifecyclePhase::PostTimeleapRechecking
                | TimeleapLifecyclePhase::PostTimeleapConfirmed
                | TimeleapLifecyclePhase::PostTimeleapRecheckFailed
        );
        let candidate_search_enabled = self.timeleap_enabled
            && (!self.timeleap_locked || frame1_has_passed)
            && !self.timeleap_frame1_preparing;

        TimeleapLifecyclePolicy {
            phase,
            table_mode,
            frame1_boundary_target,
            summary_frame1,
            configuration_locked: self.timeleap_locked,
            frame1_input_enabled,
            candidate_search_enabled,
            frame1_has_passed,
        }
    }

    pub(crate) fn starting_frame_for_display(&self, result: &FieldResult) -> Option<i64> {
        if self.timeleap_timeline_adopted() {
            self.timeleap_original_starting_frame
        } else {
            // A confirmed ordinary Timeline has a known start regardless of
            // whether Timeleap is enabled for a later transition.
            Some(result.start_consumption)
        }
    }

    /// 成功したTimeleap候補のイベント後状態を、通常Timelineの開始カーソルへ接続する。
    /// 成功前に生成していた候補Timelineを残すと、イベント前の状態を表示・延長して
    /// しまうため、接続点に1行だけ置いて以後は通常の延長処理へ渡す。
    pub(crate) fn adopt_timeleap_timeline(&mut self) -> bool {
        let Some(candidate) = self.timeleap_confirmed_candidate.clone() else {
            self.timeleap_stage = TimeleapStage::Failed;
            return false;
        };
        let Some(last_shift) = self.timeleap_shift_times.last().copied() else {
            self.timeleap_stage = TimeleapStage::Failed;
            return false;
        };
        self.adopt_timeleap_candidate(candidate, last_shift)
    }

    /// Frame1期限では入力値に厳密一致する候補を仮採用する。
    /// Shift再検索が成功した場合は、その実測候補で通常の確定採用へ置き換わる。
    pub(crate) fn adopt_assumed_timeleap_timeline(&mut self, adopted_at: Instant) -> bool {
        if !self.timeleap_assumed() {
            return false;
        }
        let Some(frame1) = self.timeleap_selected_frame else {
            self.timeleap_stage = TimeleapStage::Failed;
            return false;
        };
        let Some(candidate) = self
            .timeleap_execution_candidates
            .iter()
            .find(|candidate| candidate.frame1 == frame1)
            .cloned()
        else {
            self.timeleap_stage = TimeleapStage::Failed;
            return false;
        };
        let transition_frames = candidate
            .post_tick
            .saturating_sub(candidate.frame1_tick)
            .max(0)
            .saturating_mul(2);
        // The Frame1 deadline has already consumed the current correction.
        // Carry that baseline into the provisional Target clock so the same
        // correction is not applied a second time after adoption.
        let consumed_adjustment = self.adjust;
        if !self.adopt_timeleap_candidate(candidate, adopted_at) {
            return false;
        }
        self.sync_timeleap_source_frame1_from_reset_value();
        self.timeleap_frame1_adjustment_consumed = Some(consumed_adjustment);
        self.timeleap_assumed_transition_frames = transition_frames;
        true
    }

    /// Ready後の予定候補だけを独立snapshotで採用し、Target計算を先行する。
    pub(crate) fn direct_encounter_target_snapshot(&self) -> Option<(TimeleapCandidate, Self)> {
        if !self.timeleap_enabled
            || !self.timeleap_direct_encounter
            || self.timeleap_stage != TimeleapStage::Frame1Waiting
        {
            return None;
        }
        let frame1 = self.timeleap_selected_frame?;
        let candidate = self
            .timeleap_execution_candidates
            .iter()
            .find(|candidate| candidate.frame1 == frame1)?
            .clone();
        let mut snapshot = Self {
            seed: self.seed.clone(),
            results: vec![self.results.get(self.selected_result)?.clone()],
            observation_cache: self.observation_cache.clone(),
            timeleap_enabled: true,
            timeleap_direct_encounter: true,
            timeleap_stage: TimeleapStage::Assumed,
            timeleap_selected_frame: Some(frame1),
            timeleap_execution_candidates: vec![candidate.clone()],
            timeleap_original_starting_frame: self.timeleap_original_starting_frame,
            adjust: self.adjust,
            ..Self::default()
        };
        snapshot
            .adopt_assumed_timeleap_timeline(Instant::now())
            .then_some((candidate, snapshot))
    }

    /// 仮採用の直後に、同じ候補から事前計算したTarget情報だけを反映する。
    pub(crate) fn apply_prepared_direct_target(
        &mut self,
        candidate: &TimeleapCandidate,
        prepared: &FieldResult,
    ) -> bool {
        if !self.timeleap_enabled
            || !self.timeleap_direct_encounter
            || self.timeleap_stage != TimeleapStage::Assumed
            || !self.timeleap_timeline_adopted()
            || self.timeleap_selected_frame != Some(candidate.frame1)
            || !self
                .timeleap_execution_candidates
                .iter()
                .any(|current| current == candidate)
        {
            return false;
        }
        let Some(result) = self.results.get_mut(self.selected_result) else {
            return false;
        };
        if result.start_consumption != candidate.post_sfmt_position
            || result.last_consumption != candidate.post_sfmt_position
            || result.last_wait_frame != candidate.post_tick.saturating_mul(2)
            || result.models != prepared.models
            || result.observed_model != prepared.observed_model
            || result.target_consumptions != prepared.target_consumptions
        {
            return false;
        }
        result.target_wait_frame = prepared.target_wait_frame;
        result.target_exact = prepared.target_exact;
        result.target_frame_before = prepared.target_frame_before;
        result.target_frame_after = prepared.target_frame_after;
        result.target_blink_count = prepared.target_blink_count;
        self.target_research_pending = false;
        true
    }

    pub(super) fn adopt_timeleap_candidate(
        &mut self,
        candidate: TimeleapCandidate,
        anchor: Instant,
    ) -> bool {
        self.confirmed_target_timeline_generation =
            self.confirmed_target_timeline_generation.wrapping_add(1);
        self.invalidate_confirmed_target_search();
        let original_starting_frame = self.timeleap_original_starting_frame.or_else(|| {
            self.results
                .get(self.selected_result)
                .map(|result| result.start_consumption)
        });
        let Some(original_starting_frame) = original_starting_frame else {
            self.timeleap_stage = TimeleapStage::Failed;
            return false;
        };
        let Some(result) = self.results.get_mut(self.selected_result) else {
            self.timeleap_stage = TimeleapStage::Failed;
            return false;
        };
        let runtime = TimeleapContinuation::from(&candidate.post_status);
        let cursor = TimelineCursor {
            status: candidate.post_status.model,
            stream_cursor: candidate.post_sfmt_position,
            consumption: candidate.post_sfmt_position,
            tick: candidate.post_tick,
            timeleap_runtime: Some(runtime),
        };
        result.start_consumption = candidate.post_sfmt_position;
        result.first_consumption = candidate.post_sfmt_position;
        result.last_consumption = candidate.post_sfmt_position;
        let post_timeline_frame = candidate.post_tick.saturating_mul(2);
        result.first_wait_frame = post_timeline_frame;
        result.last_wait_frame = post_timeline_frame;
        result.next_blink_wait_frame = 0;
        result.target_wait_frame = None;
        result.target_exact = None;
        result.target_frame_before = None;
        result.target_frame_after = None;
        result.target_blink_count = None;
        result.target_blink_count_offset_from_first_row = 0;
        result.timeline = vec![FieldTimelineRow {
            frame: candidate.post_sfmt_position,
            timeline_frame: post_timeline_frame,
            duration_frames: 0,
        }];
        result.candidate_blinks.clear();
        result.timeline_cursor = Some(cursor.clone());
        result.target_cursor = Some(cursor.clone());
        self.resolved_timeline_cache = None;
        self.timeline_projection_through_tick = cursor.tick;
        self.last_current_sfmt_frame = None;
        self.timeleap_timeline_anchor = Some(anchor);
        self.timeleap_frame1_adjustment_consumed = None;
        self.timeleap_assumed_transition_frames = 0;
        self.timeleap_original_starting_frame
            .get_or_insert(original_starting_frame);
        self.timeleap_recheck_pause_frame = None;
        self.target_input_locked = false;
        self.timeleap_safe_frame_display = None;
        self.timeleap_frame1_table_boundary = None;
        self.timer_kind = if self.timeleap_direct_encounter {
            FieldTimerKind::Target
        } else {
            FieldTimerKind::NextBlink
        };
        self.timer_start = None;
        self.timer_seconds = 0.0;
        self.timer_finished = false;
        // Frame1 has passed: switch the shared tolerance input to the predicted
        // current-position radius before the user explicitly starts rechecking.
        self.activate_timeleap_recheck_interval_tolerance();
        true
    }

    pub fn begin_timeleap_recheck(&mut self, target_passed: bool, fps: f64) -> AppResult<()> {
        self.begin_timeleap_recheck_at(target_passed, fps, Instant::now())
    }

    pub(super) fn begin_timeleap_recheck_at(
        &mut self,
        target_passed: bool,
        fps: f64,
        now: Instant,
    ) -> AppResult<()> {
        if target_passed {
            self.confirmed_target_timeline_generation =
                self.confirmed_target_timeline_generation.wrapping_add(1);
            self.invalidate_confirmed_target_search();
            self.enter_correction_mode(fps);
            return Ok(());
        }
        let recheck_range_frames = self
            .timeleap_search_range
            .trim()
            .parse::<i64>()
            .ok()
            .filter(|range| (0..=98).contains(range))
            .ok_or(AppError::InvalidState(StateIssue::CandidateUnavailable))?;
        self.confirmed_target_timeline_generation =
            self.confirmed_target_timeline_generation.wrapping_add(1);
        self.invalidate_confirmed_target_search();
        let pause_frame = self
            .current_timeline_frame_at(fps, now)
            .or(self.timeleap_observation_anchor_frame)
            .ok_or(AppError::InvalidState(StateIssue::ObservationAnchorMissing))?;
        self.timeleap_recheck_pause_frame = Some(pause_frame);
        if self.timeleap_timeline_adopted() && self.timeleap_confirmed_candidate.is_none() {
            let nominal_tick = self
                .timeleap_nominal_tick
                .ok_or(AppError::InvalidState(StateIssue::CandidateUnavailable))?;
            if !self.timeleap_execution_candidates.iter().any(|candidate| {
                frame1_tick_is_in_research_range(
                    candidate.frame1_tick,
                    nominal_tick,
                    recheck_range_frames,
                )
            }) {
                return Err(AppError::InvalidState(StateIssue::CandidateUnavailable));
            }
            self.clear_notice();
            self.activate_timeleap_recheck_interval_tolerance();
            self.timeleap_stage = TimeleapStage::ShiftWaiting;
            self.cancel_timeleap_shift_resolution();
            self.timeleap_shift_times.clear();
            self.timer_start = None;
            self.timer_seconds = 0.0;
            self.timer_finished = false;
            // Keep Frame1 as the absolute observation origin; the assumed live
            // branch is not evidence and must not replace the candidate set.
            return Ok(());
        }
        let current_frame = self
            .current_timeline_frame_at(fps, now)
            .ok_or(AppError::InvalidState(StateIssue::ObservationAnchorMissing))?;
        let current_tick = current_frame.div_euclid(2);
        let shared = self.ensure_resolved_timeline_cache()?;
        let observation_cache = self
            .observation_cache
            .as_ref()
            .filter(|cache| parse_seed(&self.seed).ok() == Some(cache.seed))
            .ok_or(AppError::InvalidState(StateIssue::CacheUnavailable))?;
        let current_cursor = shared
            .lock()
            .map_err(|_| AppError::InvalidState(StateIssue::TimelineCursorUnavailable))?
            .cursor_at_tick(current_tick, observation_cache)?
            .ok_or(AppError::InvalidState(
                StateIssue::TimelineCursorUnavailable,
            ))?;
        let mut candidate = self
            .timeleap_confirmed_candidate
            .clone()
            .ok_or(AppError::InvalidState(StateIssue::CandidateUnavailable))?;
        let runtime = current_cursor
            .timeleap_runtime
            .as_ref()
            .ok_or(AppError::InvalidState(StateIssue::CalculationUnavailable))?;
        candidate.post_frame = current_cursor.consumption;
        candidate.post_sfmt_position = current_cursor.stream_cursor;
        candidate.post_tick = current_cursor.tick;
        candidate.post_status = TimeleapStatus {
            model: current_cursor.status,
            phase: runtime.phase,
            fidget_cd: runtime.fidget_cd,
            is_boy: runtime.is_boy,
            raining: runtime.raining,
        };
        self.timeleap_execution_candidates = vec![candidate];
        self.clear_notice();
        self.timeleap_stage = TimeleapStage::Recheck;
        self.cancel_timeleap_shift_resolution();
        self.timeleap_shift_times.clear();
        self.timer_start = None;
        self.timer_seconds = 0.0;
        self.timer_finished = false;
        self.timeleap_observation_anchor_frame = Some(current_frame);
        self.timeleap_wait_started = Some(now);
        self.activate_timeleap_recheck_interval_tolerance();
        Ok(())
    }

    /// Timeleap有効時のTargetタイマー枠をFrame1カウントダウンとして選ぶ。
    pub(crate) fn select_timeleap_frame1_timer(&mut self) {
        self.timer_kind = FieldTimerKind::Target;
        self.timer_start = None;
        self.timer_seconds = 0.0;
        self.timer_finished = false;
    }

    pub fn timeleap_retry(&mut self, _fps: f64) -> bool {
        if !self.timeleap_locked || self.timeleap_stage != TimeleapStage::Failed {
            return false;
        }
        self.clear_notice();
        self.cancel_timeleap_shift_resolution();
        self.timeleap_shift_times.clear();
        self.activate_timeleap_recheck_interval_tolerance();
        self.timer_start = None;
        self.timer_finished = false;
        self.timeleap_stage = TimeleapStage::ShiftWaiting;
        true
    }
    /// 保存値またはUI確定値を、Target上限と同期して状態へ反映する。
    pub(crate) fn commit_target_input(&mut self) -> AppResult<()> {
        if self.target_consumption.trim().is_empty() {
            self.target_consumption.clear();
            self.target_last_committed.clear();
            self.target_dirty = false;
            return Ok(());
        }

        let parsed = match self.target_consumption.trim().parse::<i64>() {
            Ok(value) => value,
            Err(_) => {
                self.target_consumption = self.target_last_committed.clone();
                self.target_dirty = false;
                return Err(AppError::InvalidInteger(InputField::TargetConsumption));
            }
        };
        if parsed < 0 {
            self.target_consumption = self.target_last_committed.clone();
            self.target_dirty = false;
            return Err(AppError::NegativeValue(InputField::TargetConsumption));
        }
        if parsed > crate::domain::rng::MAX_SFMT_FRAME {
            self.target_consumption = self.target_last_committed.clone();
            self.target_dirty = false;
            return Err(AppError::OutOfRange {
                field: InputField::TargetConsumption,
                min: 0,
                max: crate::domain::rng::MAX_SFMT_FRAME,
            });
        }

        self.target_consumption = parsed.to_string();
        self.target_last_committed = self.target_consumption.clone();
        self.target_dirty = false;
        Ok(())
    }

    /// 永続化値は起動時にも上限を検証し、範囲外値を再計算へ渡さない。
    pub(crate) fn restore_target_input(&mut self, value: String) {
        self.target_consumption = value;
        self.target_last_committed.clear();
        if self.commit_target_input().is_err() {
            self.target_consumption.clear();
            self.target_last_committed.clear();
            self.target_dirty = false;
        }
    }

    pub(crate) fn timeleap_frame1_remaining(&self, fps: f64) -> Option<f64> {
        if self.timeleap_waiting_for_blink_observation() {
            return Some(0.0);
        }
        self.timeleap_frame1_remaining_at(Instant::now(), fps)
    }

    pub(in crate::application::field::core::state) fn timeleap_frame1_remaining_at(
        &self,
        now: Instant,
        _fps: f64,
    ) -> Option<f64> {
        let deadline = self.timeleap_frame1_deadline?;
        Some(deadline.saturating_duration_since(now).as_secs_f64())
    }

    pub(in crate::application::field::core::state) fn start_timeleap_frame1_timer(
        &mut self,
        now: Instant,
        _fps: f64,
    ) {
        let Some(deadline) = self.timeleap_frame1_deadline else {
            return;
        };
        self.timer_kind = FieldTimerKind::Target;
        self.timer_start = Some(now);
        self.timer_seconds = deadline.saturating_duration_since(now).as_secs_f64();
        self.timer_finished = deadline <= now;
    }
}
