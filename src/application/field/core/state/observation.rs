use super::*;

impl FieldState {
    #[cfg(test)]
    pub(crate) fn empty_timeline_for_test() -> Self {
        Self {
            seed: "00000001".into(),
            results: vec![FieldResult {
                start_consumption: 0,
                first_wait_frame: 0,
                last_wait_frame: 0,
                first_consumption: 0,
                last_consumption: 0,
                intervals: vec![1],
                target_consumptions: vec![0],
                next_blink_wait_frame: 0,
                target_wait_frame: None,
                target_exact: None,
                target_frame_before: None,
                target_frame_after: None,
                observed_model: 0,
                models: 1,
                timeline: Vec::new(),
                candidate_blinks: Vec::new(),
                target_blink_count: None,
                target_blink_count_offset_from_first_row: 0,
                timeline_cursor: None,
                target_cursor: None,
            }],
            next_blink_out_of_range: true,
            ..Self::default()
        }
    }

    pub(crate) fn set_candidate_not_found_notice(&mut self, message: impl Into<String>) {
        self.notice = Some(message.into());
    }

    pub(crate) fn set_search_range_notice(&mut self, message: impl Into<String>) {
        let message = message.into();
        if message.is_empty() {
            self.clear_notice();
        } else {
            self.notice = Some(message);
        }
    }

    pub(crate) fn clear_notice(&mut self) {
        self.notice = None;
    }

    /// 設定画面の初期分岐探索範囲を検証して返す。
    pub fn initial_branch_scan_span_value(&self) -> AppResult<i64> {
        let value = parse_nonnegative(
            &self.initial_branch_scan_span,
            InputField::InitialBranchSearchSpan,
        )?;
        if value > crate::application::field::MAX_INITIAL_BRANCH_SCAN_SPAN {
            return Err(AppError::OutOfRange {
                field: InputField::InitialBranchSearchSpan,
                min: 0,
                max: crate::application::field::MAX_INITIAL_BRANCH_SCAN_SPAN,
            });
        }
        Ok(value)
    }

    /// UIのNPC欄を単一または範囲として展開した検索設定。
    /// 主人公1体は各設定の`models`へ暗黙に加算する。
    pub fn configs(&self) -> AppResult<Vec<FieldConfig>> {
        let seed = u32::from_str_radix(self.seed.trim().trim_start_matches("0x"), 16)
            .map_err(|_| AppError::InvalidSeed)?;
        let range_start = parse_nonnegative(&self.range_start, InputField::SearchStart)?;
        let range_end = parse_nonnegative(&self.range_end, InputField::SearchEnd)?;
        let npc_counts = parse_npc_counts(&self.npc_count, self.multiple_npc_search)?;
        // NPC番号は検索開始時のTimelineを絞る条件にしないが、入力自体は必須とする。
        // 空欄を0へ暗黙変換すると、未入力のまま主人公として検索されるため、
        // UIの入力ミスと本当に0を指定したケースを区別できなくなる。
        let target_model = usize::try_from(parse_nonnegative(
            &self.target_model,
            InputField::NpcNumber,
        )?)
        .map_err(|_| AppError::NpcNumberTooLarge)?;
        let tolerance = parse_nonnegative(&self.tolerance, InputField::Tolerance)?;
        let target_consumption = if self.target_consumption.trim().is_empty() {
            None
        } else {
            Some(parse_nonnegative(
                &self.target_consumption,
                InputField::TargetConsumption,
            )?)
        };
        let branch_span = parse_nonnegative(
            &self.initial_branch_scan_span,
            InputField::InitialBranchSearchSpan,
        )?;
        if branch_span > crate::application::field::MAX_INITIAL_BRANCH_SCAN_SPAN {
            return Err(AppError::OutOfRange {
                field: InputField::InitialBranchSearchSpan,
                min: 0,
                max: crate::application::field::MAX_INITIAL_BRANCH_SCAN_SPAN,
            });
        }

        let mut configs = Vec::with_capacity(npc_counts.len());
        for npc_count in npc_counts {
            let models = npc_count.checked_add(1).ok_or(AppError::NpcCountTooLarge)?;
            if models > MAX_MODELS {
                return Err(AppError::OutOfRange {
                    field: InputField::NpcCount,
                    min: 0,
                    max: MAX_MODELS as i64 - 1,
                });
            }
            configs.push(FieldConfig {
                seed,
                range_start,
                range_end,
                models,
                target_model,
                observed_intervals: self.intervals.clone(),
                tolerance,
                target_consumption,
            });
        }
        if configs.is_empty() {
            return Err(AppError::NpcRangeDoesNotContainTarget);
        }
        for config in &configs {
            validate_static_config(config)?;
        }
        Ok(configs)
    }

    /// 単一設定を必要とする計算へ、展開後の先頭設定を返す。
    #[allow(dead_code)]
    pub fn config(&self) -> AppResult<FieldConfig> {
        self.configs()?
            .into_iter()
            .next()
            .ok_or(AppError::NpcRangeDoesNotContainTarget)
    }

    /// 観測列に一致した候補を、完全な現在状態が同じ場合だけ統合する。
    ///
    /// 異なる開始位置が観測対象の瞬きだけ同じになる場合、有限の観測列から
    /// 全Timelineの同一性は保証できない。そのため、保存済みカーソルがある候補は
    /// 全モデルの状態とSFMTカーソルが一致した場合だけ統合する。過去の対象NPCの
    /// SFMT位置列や間隔列は、現在状態が一致した候補を分ける理由にはしない。
    /// カーソルを持たない合成結果については内部状態を比較できないため、従来どおり
    /// 観測履歴と開始位置が完全一致する場合だけ統合する。
    pub(crate) fn normalize_search_results(
        &mut self,
        results: Vec<FieldResult>,
    ) -> Vec<FieldResult> {
        let requested = self.target_model.trim().parse::<usize>().ok();
        let mut groups: Vec<Vec<FieldResult>> = Vec::new();
        for result in results {
            let same_window = |candidate: &FieldResult| {
                let same_identity = candidate.models == result.models
                    && candidate.observed_model == result.observed_model;
                if !same_identity {
                    return false;
                }

                let has_full_state = candidate
                    .target_cursor
                    .as_ref()
                    .or(candidate.timeline_cursor.as_ref())
                    .is_some()
                    && result
                        .target_cursor
                        .as_ref()
                        .or(result.timeline_cursor.as_ref())
                        .is_some();
                if has_full_state {
                    // 現在状態が完全一致していれば、異なる開始位置や過去の
                    // target_consumptionsを持つ候補でも同じTimelineへ合流している。
                    same_full_timeline_state(candidate, &result)
                } else {
                    // 状態を保存していないテスト用・合成候補は、観測履歴まで
                    // 一致しなければ同一性を判定できない。
                    candidate.start_consumption == result.start_consumption
                        && candidate.intervals == result.intervals
                        && candidate.target_consumptions == result.target_consumptions
                        && same_full_timeline_state(candidate, &result)
                }
            };
            if let Some(group) = groups
                .iter_mut()
                .find(|group| group.first().is_some_and(same_window))
            {
                group.push(result);
            } else {
                groups.push(vec![result]);
            }
        }

        let requested_groups = requested
            .map(|index| {
                groups
                    .iter()
                    .enumerate()
                    .filter(|(_, group)| group.iter().any(|result| result.observed_model == index))
                    .map(|(group_index, _)| group_index)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let selected_group =
            (requested.is_some() && requested_groups.len() == 1).then(|| requested_groups[0]);
        let mut representatives = Vec::with_capacity(selected_group.map_or(groups.len(), |_| 1));
        for (group_index, group) in groups.iter().enumerate() {
            if selected_group.is_some_and(|selected| selected != group_index) {
                continue;
            }
            let representative = requested
                .and_then(|index| group.iter().find(|result| result.observed_model == index))
                .or_else(|| group.first())
                .cloned();
            if let Some(representative) = representative {
                representatives.push(representative);
            }
        }
        representatives.sort_by_key(|result| {
            (
                result.start_consumption,
                result.models,
                result.first_wait_frame,
                result.last_wait_frame,
            )
        });
        if representatives.len() == 1 {
            representatives[0].candidate_blinks.clear();
        }
        representatives
    }

    /// 確定済みTimelineの観測NPC番号と、条件設定のNPC番号を直接比較する。
    ///
    /// 表示用の派生値であり、検索やタイマーの状態として保存しない。
    pub(crate) fn selected_npc_number_match(&self) -> Option<bool> {
        let requested = self.target_model.trim().parse::<usize>().ok();
        match (requested, self.results.as_slice()) {
            (Some(requested), [result]) => Some(result.observed_model == requested),
            _ => None,
        }
    }

    pub(super) fn enter_idle(&mut self) {
        self.mode = FieldMode::Idle;
    }

    pub(super) fn enter_observing(&mut self) {
        self.mode = FieldMode::Observing;
    }

    /// フィールド側の内部状態を共通フェーズへ写像する。
    pub fn session_phase(&self) -> SessionPhase {
        if self.mode == FieldMode::Correction {
            return SessionPhase::Correction;
        }
        if self.timer_start.is_some() {
            return SessionPhase::Timer;
        }
        if self.timer_finished {
            return SessionPhase::Timer;
        }
        if self.mode == FieldMode::Observing {
            return SessionPhase::Observing;
        }
        if self.results.len() == 1 {
            return SessionPhase::TimelineReady;
        }
        SessionPhase::Input
    }

    pub fn can_edit_general_settings(&self) -> bool {
        !self.timeleap_locked
            && matches!(
                self.session_phase(),
                SessionPhase::Input | SessionPhase::Observing | SessionPhase::TimelineReady
            )
    }

    /// 瞬き観測の範囲・許容差・観測対象は開始前に確定する。
    /// 観測後に変更する場合はリセットして新しいセッションを開始する。
    pub fn can_edit_observation_settings(&self) -> bool {
        !self.timeleap_locked && self.session_phase() == SessionPhase::Input
    }

    pub fn can_edit_correction_inputs(&self) -> bool {
        self.session_phase() == SessionPhase::Correction
    }

    /// TargetFrameは観測結果確定後からEnterまでは編集でき、
    /// 明示的に補正モードへ進んだ後だけロックする。
    pub fn can_edit_target(&self) -> bool {
        !self.target_input_locked && self.mode != FieldMode::Correction
    }

    pub fn can_start_timer(&self) -> bool {
        self.session_phase() == SessionPhase::TimelineReady
    }

    pub fn is_observing(&self) -> bool {
        self.mode == FieldMode::Observing
    }

    pub fn is_correction_mode(&self) -> bool {
        self.mode == FieldMode::Correction
    }

    pub fn timer_is_running(&self) -> bool {
        self.timer_start.is_some()
    }

    pub fn timer_start_time(&self) -> Option<Instant> {
        self.timer_start
    }

    pub fn timer_kind(&self) -> FieldTimerKind {
        self.timer_kind
    }

    pub fn timer_is_finished(&self) -> bool {
        self.timer_finished
    }

    pub fn timer_seconds_value(&self) -> f64 {
        self.timer_seconds
    }

    /// 現在の瞬き区間を終了し、次の区間へ進めるための遷移。
    /// タイマーの開始時刻をUIから直接消去しないようにする。
    pub fn finish_blink_timer_segment(&mut self, fps: f64) -> AppResult<()> {
        self.timer_start = None;
        self.advance_blink_timer(fps)
    }

    /// Target区間を0秒で保持する終端遷移。以後の再開可否は、明示的な
    /// `toggle_timer_kind`または`enter_correction_mode`だけが決める。
    pub fn finish_target_timer(&mut self) {
        self.timer_finished = true;
    }

    /// 指定時刻までにTargetタイマーが経過していれば、0秒表示の完了状態へ移す。
    pub(crate) fn finish_target_timer_if_elapsed(&mut self, now: Instant) -> bool {
        if self.timer_kind != FieldTimerKind::Target
            || self.timer_finished
            || !self
                .timer_remaining_at(now)
                .is_some_and(|remaining| remaining <= 0.0)
        {
            return false;
        }
        self.finish_target_timer();
        true
    }

    /// 検索結果が一意になったときの遷移を一箇所へ集約する。
    pub fn mark_timeline_ready(&mut self) {
        if self.results.len() == 1 && self.mode != FieldMode::Correction {
            self.enter_idle();
        }
    }

    /// 検索スレッドの結果を現在のセッションへ反映できる状態か返す。
    ///
    /// 一意候補を得た後にTargetだけを変更した再計算も受け付ける。
    /// 通常のキャンセル後に古いスレッド結果を受け取らない条件は維持する。
    pub fn accepts_search_result(&self) -> bool {
        if self.target_research_pending {
            return true;
        }
        self.mode == FieldMode::Observing
            || (self.mode == FieldMode::Idle
                && self.results.is_empty()
                && self.intervals.len() >= crate::application::state::MIN_BLINK_INTERVALS
                && self.timer_start.is_none())
    }

    pub fn is_observation_input_enabled(&self) -> bool {
        // 候補0件で検索が終わっても、孵化側と同じく追加の瞬き入力を
        // 受け付けて再検索できるようにする。一意候補だけは停止する。
        self.mode == FieldMode::Observing && self.results.len() != 1
    }

    /// 入力条件が変わったとき、古い検索結果を現在の条件へ流用しない。
    /// 観測した時刻列は残すので、Target変更などは同じ観測から再検索できる。
    pub fn invalidate_search_results(&mut self) {
        self.clear_timeleap_frame1_preview();
        self.results.clear();
        self.target_research_pending = false;
        self.search_exhausted = false;
        self.inferred_indices.clear();
        self.correction_calculated = false;
        self.correction_suggested_offset = None;
        self.idx_fallback_indices.clear();
        self.nearest_timeline_frame = None;
        self.selected_result = 0;
        self.timer_start = None;
        self.timer_finished = false;
        self.correction_timeline_frame = None;
        self.blink_wait_queue.clear();
        self.table_last_followed_row = None;
        self.observation_cache = None;
        self.resolved_timeline_cache = None;
        self.timeline_projection_through_tick = 0;
        self.last_current_sfmt_frame = None;
        self.timeline_stream_error = None;
    }

    /// Targetだけを再計算する。観測候補と瞬きタイマーは保持し、Target
    /// の計算中も次の瞬き通知を止めない。
    pub fn prepare_target_research(&mut self, fps: f64) {
        if self.timer_kind == FieldTimerKind::Target {
            if self.timeleap_direct_encounter && self.timeleap_timeline_adopted() {
                // Keep the requested Target clock selected, but stop its stale
                // countdown until the new destination has been calculated.
                self.timer_start = None;
                self.timer_seconds = 0.0;
                self.timer_finished = false;
            } else {
                self.timer_kind = FieldTimerKind::NextBlink;
                if self.results.len() == 1 {
                    let _ = self.start_blink_timer(fps);
                } else {
                    self.timer_start = None;
                }
            }
        }
        for result in &mut self.results {
            result.target_wait_frame = None;
            result.target_exact = None;
            result.target_frame_before = None;
            result.target_frame_after = None;
            result.target_blink_count = None;
        }
        self.target_research_pending = true;
        self.search_exhausted = false;
        self.correction_calculated = false;
        self.correction_suggested_offset = None;
    }

    /// Target欄を空に戻した場合は、再検索せず既存Timelineを使い続ける。
    pub fn clear_target_prediction(&mut self, fps: f64) {
        self.target_research_pending = false;
        self.correction_calculated = false;
        self.correction_suggested_offset = None;
        if self.timer_kind == FieldTimerKind::Target {
            self.timer_kind = FieldTimerKind::NextBlink;
            if self.results.len() == 1 {
                let _ = self.start_blink_timer(fps);
            }
        }
        for result in &mut self.results {
            result.target_wait_frame = None;
            result.target_exact = None;
            result.target_frame_before = None;
            result.target_frame_after = None;
            result.target_blink_count = None;
        }
    }

    pub fn refresh_nearest_timeline_frame(&mut self) {
        self.nearest_timeline_frame =
            if self.target_mode == FieldTargetMode::Known && self.results.len() == 1 {
                self.results
                    .iter()
                    .filter(|result| result.start_consumption >= MIN_TIMELINE_FRAME)
                    .map(|result| result.start_consumption)
                    .min()
            } else {
                None
            };
    }

    pub fn start_observation(&mut self) {
        self.restore_timeleap_recheck_interval_tolerance();
        self.confirmed_target_timeline_generation =
            self.confirmed_target_timeline_generation.wrapping_add(1);
        self.invalidate_confirmed_target_search();
        self.enter_observing();
        self.clear_notice();
        self.observed.clear();
        self.intervals.clear();
        self.results.clear();
        self.search_exhausted = false;
        self.inferred_indices.clear();
        self.correction_calculated = false;
        self.correction_suggested_offset = None;
        self.idx_fallback_indices.clear();
        self.nearest_timeline_frame = None;
        self.target_dirty = false;
        self.target_research_pending = false;
        self.next_blink_out_of_range = false;
        self.target_input_locked = false;
        self.timeleap_locked = false;
        self.timeleap_stage = TimeleapStage::Off;
        self.cancel_timeleap_shift_resolution();
        self.timeleap_shift_times.clear();
        self.timeleap_wait_started = None;
        self.timeleap_observation_anchor_frame = None;
        self.timeleap_frame1_target = None;
        self.timeleap_nominal_tick = None;
        self.timeleap_frame1_preparing = false;
        self.timeleap_execution_candidates.clear();
        self.clear_timeleap_frame1_preview();
        self.timeleap_timeline_anchor = None;
        self.timeleap_frame1_adjustment_consumed = None;
        self.timeleap_assumed_transition_frames = 0;
        self.timeleap_recheck_pause_frame = None;
        self.timeleap_original_starting_frame = None;
        self.timeleap_safe_frame_display = None;
        self.timeleap_selected_sfmt_position = None;
        self.timeleap_confirmed_frame1 = None;
        self.timeleap_confirmed_candidate = None;
        self.selected_result = 0;
        self.timer_start = None;
        self.timer_finished = false;
        self.correction_timeline_frame = None;
        self.blink_wait_queue.clear();
        self.table_last_followed_row = None;
        self.observation_cache = None;
        self.resolved_timeline_cache = None;
        self.timeline_projection_through_tick = 0;
        self.last_current_sfmt_frame = None;
        self.timeline_stream_error = None;
    }

    pub fn cancel_observation(&mut self) {
        self.restore_timeleap_recheck_interval_tolerance();
        self.confirmed_target_timeline_generation =
            self.confirmed_target_timeline_generation.wrapping_add(1);
        self.invalidate_confirmed_target_search();
        self.enter_idle();
        self.clear_notice();
        self.observed.clear();
        self.intervals.clear();
        self.results.clear();
        self.search_exhausted = false;
        self.inferred_indices.clear();
        self.correction_calculated = false;
        self.correction_suggested_offset = None;
        self.idx_fallback_indices.clear();
        self.nearest_timeline_frame = None;
        self.target_dirty = false;
        self.target_research_pending = false;
        self.next_blink_out_of_range = false;
        self.target_input_locked = false;
        self.timeleap_locked = false;
        self.timeleap_stage = TimeleapStage::Off;
        self.cancel_timeleap_shift_resolution();
        self.timeleap_shift_times.clear();
        self.timeleap_wait_started = None;
        self.timeleap_observation_anchor_frame = None;
        self.timeleap_frame1_target = None;
        self.timeleap_nominal_tick = None;
        self.timeleap_frame1_preparing = false;
        self.timeleap_execution_candidates.clear();
        self.clear_timeleap_frame1_preview();
        self.timeleap_timeline_anchor = None;
        self.timeleap_frame1_adjustment_consumed = None;
        self.timeleap_assumed_transition_frames = 0;
        self.timeleap_recheck_pause_frame = None;
        self.timeleap_original_starting_frame = None;
        self.timeleap_safe_frame_display = None;
        self.timeleap_selected_sfmt_position = None;
        self.timeleap_confirmed_frame1 = None;
        self.timeleap_confirmed_candidate = None;
        self.selected_result = 0;
        self.timer_start = None;
        self.timer_finished = false;
        self.correction_timeline_frame = None;
        self.blink_wait_queue.clear();
        self.table_last_followed_row = None;
        self.observation_cache = None;
        self.resolved_timeline_cache = None;
        self.timeline_projection_through_tick = 0;
        self.last_current_sfmt_frame = None;
        self.timeline_stream_error = None;
    }

    pub(crate) fn set_observation_cache(&mut self, cache: ObservationCache) {
        self.observation_cache = Some(cache);
        self.resolved_timeline_cache = None;
        self.timeline_projection_through_tick = 0;
        self.last_current_sfmt_frame = None;
        self.timeline_stream_error = None;
    }

    pub(crate) fn observation_cache_for_seed(&self, seed: u32) -> Option<ObservationCache> {
        self.observation_cache
            .as_ref()
            .filter(|cache| cache.seed == seed)
            .cloned()
    }
}
