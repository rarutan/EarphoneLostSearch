use super::*;

impl FieldState {
    pub(crate) fn clear_timeleap_candidates(&mut self) {
        if !self.timeleap_enabled {
            self.restore_timeleap_recheck_interval_tolerance();
        }
        self.confirmed_target_timeline_generation =
            self.confirmed_target_timeline_generation.wrapping_add(1);
        self.invalidate_confirmed_target_search();
        self.timeleap_candidate_groups.clear();
        self.timeleap_execution_candidates.clear();
        self.timeleap_nominal_tick = None;
        self.timeleap_frame1_preparing = false;
        self.timeleap_search_status = TimeleapSearchStatus::NotRequested;
        self.timeleap_candidate_search_started = false;
        self.timeleap_candidate_search_key = None;
        self.timeleap_candidate_search_started_at = None;
        self.timeleap_selected_frame = None;
        self.timeleap_selected_sfmt_position = None;
        self.timeleap_confirmed_frame1 = None;
        self.timeleap_confirmed_candidate = None;
        self.timeleap_frame1_preview_state = TimeleapFrame1PreviewState::Idle;
        self.timeleap_target_check_pending = false;
        self.timeleap_target_reachable = None;
        self.timeleap_frame1_reachable = None;
        self.timeleap_frame1_deadline = None;
        self.timeleap_frame1_preview_generation =
            self.timeleap_frame1_preview_generation.wrapping_add(1);
    }

    pub(crate) fn invalidate_timeleap_frame1_preview(&mut self) {
        self.timeleap_frame1_preview_generation =
            self.timeleap_frame1_preview_generation.wrapping_add(1);
        self.timeleap_frame1_preview_state = TimeleapFrame1PreviewState::Pending;
        self.timeleap_target_check_pending =
            self.timeleap_enabled && self.committed_target_frame().is_some();
        self.timeleap_target_reachable = None;
        self.timeleap_frame1_reachable = None;
        self.timeleap_frame1_deadline = None;
        self.timeleap_frame1_table_boundary = None;
        if self.timeleap_enabled && self.timer_kind == FieldTimerKind::Target {
            self.timer_start = None;
            self.timer_finished = false;
        }
    }

    /// Drop a preview when its source Timeline no longer exists. Unlike a
    /// changed Frame1 input, this must remain Idle until a new unique Timeline
    /// is available, rather than showing a calculation that cannot start yet.
    pub(in crate::application::field::core::state) fn clear_timeleap_frame1_preview(&mut self) {
        self.timeleap_frame1_preview_generation =
            self.timeleap_frame1_preview_generation.wrapping_add(1);
        self.timeleap_frame1_preview_state = TimeleapFrame1PreviewState::Idle;
        self.timeleap_target_check_pending = false;
        self.timeleap_target_reachable = None;
        self.timeleap_frame1_reachable = None;
        self.timeleap_frame1_deadline = None;
        self.timeleap_frame1_table_boundary = None;
        self.timeleap_frame1_wait_seconds = 0.0;
        if self.timeleap_enabled && self.timer_kind == FieldTimerKind::Target {
            self.timer_start = None;
            self.timer_finished = false;
        }
    }

    pub(crate) fn set_timeleap_frame1_preview(
        &mut self,
        generation: u64,
        config: TimeleapConfig,
        frame1: i64,
        target_frame: Option<i64>,
        start: crate::domain::timeleap::TimeleapTimelineStart,
        prepared_at: Instant,
        preparation: crate::domain::timeleap::TimeleapFrame1Preparation,
        fps: f64,
    ) {
        if generation != self.timeleap_frame1_preview_generation || !self.timeleap_enabled {
            return;
        }
        self.timeleap_frame1_reachable = Some(true);
        let (deadline, duration) = self.frame1_deadline(preparation.nominal_tick, prepared_at, fps);
        self.timeleap_frame1_wait_seconds = duration.as_secs_f64();
        self.timeleap_frame1_deadline = Some(deadline);
        self.timeleap_frame1_preview_state =
            TimeleapFrame1PreviewState::Ready(TimeleapFrame1Preview {
                config,
                frame1,
                target_frame,
                start,
                preparation,
            });
        if self.timeleap_enabled && self.timer_kind == FieldTimerKind::Target {
            self.start_timeleap_frame1_timer(Instant::now(), fps);
        }
    }

    pub(crate) fn has_timeleap_frame1_preview(&self) -> bool {
        self.timeleap_enabled && self.timeleap_frame1_preview_state.is_ready()
    }

    pub(crate) fn timeleap_frame1_preview_is_pending(&self) -> bool {
        self.timeleap_frame1_preview_state.is_pending()
    }

    pub(crate) fn timeleap_frame1_preview_failed(&self) -> bool {
        self.timeleap_frame1_preview_state.is_failed()
    }

    pub(crate) fn needs_timeleap_frame1_preview(&self) -> bool {
        self.timeleap_enabled
            && self.results.len() == 1
            && self.timeleap_stage == TimeleapStage::Off
            && !self.timeleap_locked
            && !self.timeleap_frame1_preparing
            && matches!(
                self.timeleap_frame1_preview_state,
                TimeleapFrame1PreviewState::Idle
            )
    }

    pub(crate) fn reconcile_timeleap_frame1_preview(
        &mut self,
    ) -> TimeleapFrame1PreviewReconciliation {
        if !self.timeleap_enabled
            || self.results.len() != 1
            || self.timeleap_locked
            || self.timeleap_frame1_preparing
            || self.timeleap_frame1_preview_failed()
        {
            return TimeleapFrame1PreviewReconciliation::Stable;
        }
        if self.timeleap_frame1_preview_is_stale() {
            self.invalidate_timeleap_frame1_preview();
            return TimeleapFrame1PreviewReconciliation::Restart;
        }
        if self.needs_timeleap_frame1_preview() {
            self.invalidate_timeleap_frame1_preview();
            return TimeleapFrame1PreviewReconciliation::Schedule;
        }
        TimeleapFrame1PreviewReconciliation::Stable
    }

    pub(crate) fn timeleap_frame1_preview_matches_current(&self) -> bool {
        let TimeleapFrame1PreviewState::Ready(preview) = &self.timeleap_frame1_preview_state else {
            return false;
        };
        let Ok(seed) = parse_seed(&self.seed) else {
            return false;
        };
        let Ok(models) = self.timeleap_models() else {
            return false;
        };
        let Ok(frame1) = self.timeleap_frame1_input.trim().parse::<i64>() else {
            return false;
        };
        let Ok(courier) = self.timeleap_courier_config() else {
            return false;
        };
        // The live cursor advances continuously while waiting for Frame1. Compare
        // only the request's stable inputs; calling prepare_timeleap_frame1 here
        // would revalidate against that moving cursor and invalidate a valid
        // preview as soon as its Frame1 fell behind the current position.
        let config = TimeleapConfig {
            seed,
            start_frame: preview.config.start_frame,
            target_frame: frame1,
            models,
            min_frame1_elapsed_ticks: 0,
            max_frame1_elapsed_ticks: 0,
            mode: self.timeleap_mode,
            is_boy: self.timeleap_is_boy,
            handoff_range_frames: 98,
            courier,
        };
        // The preview is anchored to the confirmed Timeline snapshot it was
        // calculated from. `target_cursor` is a Target-facing cursor and may be
        // refreshed as the live Timeline advances; comparing it here makes the
        // Frame1-only worker invalidate itself during ordinary countdown ticks.
        // Timeline/input changes explicitly advance preview_generation instead.
        preview.frame1 == frame1 && preview.config == config
    }

    pub(crate) fn timeleap_frame1_preview_is_stale(&self) -> bool {
        matches!(
            self.timeleap_frame1_preview_state,
            TimeleapFrame1PreviewState::Ready(_)
        ) && !self.timeleap_frame1_preview_matches_current()
    }

    pub(crate) fn mark_timeleap_frame1_preview_failed(&mut self) {
        self.timeleap_frame1_preview_state = TimeleapFrame1PreviewState::Failed;
    }

    pub(crate) fn fail_timeleap_frame1_preview_generation(&mut self, generation: u64) -> bool {
        if self.timeleap_frame1_preview_generation != generation {
            return false;
        }
        self.timeleap_target_check_pending = false;
        self.mark_timeleap_frame1_preview_failed();
        true
    }

    pub(crate) fn reset_timeleap_frame1_preview_to_idle(&mut self) {
        self.timeleap_frame1_preview_state = TimeleapFrame1PreviewState::Idle;
    }

    pub(crate) fn refresh_timeleap_target_check_target(&mut self) {
        let target_frame = self.committed_target_frame();
        let TimeleapFrame1PreviewState::Ready(preview) = &mut self.timeleap_frame1_preview_state
        else {
            return;
        };
        if preview.target_frame == target_frame {
            return;
        }
        preview.target_frame = target_frame;
        self.timeleap_target_reachable = None;
        self.timeleap_target_check_pending = target_frame.is_some();
    }

    pub(crate) fn committed_target_frame(&self) -> Option<i64> {
        self.target_last_committed.trim().parse::<i64>().ok()
    }

    pub(crate) fn prepare_timeleap_target_check_request(
        &mut self,
    ) -> Option<(
        u64,
        i64,
        i64,
        crate::domain::timeleap::TimeleapFrame1Preparation,
        ObservationCache,
    )> {
        let snapshot = match &self.timeleap_frame1_preview_state {
            TimeleapFrame1PreviewState::Ready(preview) => {
                let target_frame = preview.target_frame;
                let cache = self
                    .observation_cache
                    .as_ref()
                    .filter(|cache| cache.seed == preview.config.seed)
                    .cloned();
                target_frame.zip(cache).map(|(target_frame, cache)| {
                    (
                        self.timeleap_frame1_preview_generation,
                        preview.frame1,
                        target_frame,
                        preview.preparation.clone(),
                        cache,
                    )
                })
            }
            _ => None,
        };
        if snapshot.is_none() {
            self.timeleap_target_check_pending = false;
        }
        snapshot
    }

    pub(crate) fn timeleap_target_check_is_pending(&self) -> bool {
        self.timeleap_target_check_pending
    }

    pub(crate) fn clear_timeleap_target_check_for_frame1_generation(
        &mut self,
        generation: u64,
    ) -> bool {
        if self.timeleap_frame1_preview_generation != generation {
            return false;
        }
        self.timeleap_target_check_pending = false;
        true
    }

    pub(crate) fn timeleap_target_check_matches(&self, generation: u64, target: i64) -> bool {
        self.timeleap_enabled
            && self.timeleap_frame1_preview_generation == generation
            && self.committed_target_frame() == Some(target)
    }

    pub(crate) fn apply_timeleap_target_check(
        &mut self,
        generation: u64,
        target: i64,
        reachable: bool,
    ) {
        if !self.timeleap_target_check_matches(generation, target) {
            return;
        }
        self.timeleap_target_check_pending = false;
        self.timeleap_target_reachable = Some(reachable);
    }

    pub(crate) fn fail_timeleap_target_check(&mut self, generation: u64, target: i64) -> bool {
        if !self.timeleap_target_check_matches(generation, target) {
            return false;
        }
        self.timeleap_target_check_pending = false;
        self.timeleap_target_reachable = None;
        true
    }

    pub(crate) fn target_reachability_for_display(
        &self,
        ordinary_timeline_result: Option<bool>,
    ) -> Option<bool> {
        match self.timeleap_lifecycle_policy().phase {
            TimeleapLifecyclePhase::Frame1Setup
            | TimeleapLifecyclePhase::Frame1Preparing
            | TimeleapLifecyclePhase::Frame1Waiting
            | TimeleapLifecyclePhase::Frame1Unreachable => self.timeleap_target_reachable,
            TimeleapLifecyclePhase::PostTimeleapAssumed
            | TimeleapLifecyclePhase::PostTimeleapConfirmed
            | TimeleapLifecyclePhase::Disabled => ordinary_timeline_result,
            TimeleapLifecyclePhase::Frame1Failed
            | TimeleapLifecyclePhase::PostTimeleapRechecking
            | TimeleapLifecyclePhase::PostTimeleapRecheckFailed => None,
        }
    }

    pub(in crate::application::field::core::state) fn timeleap_frame1_timer_available(
        &mut self,
        fps: f64,
    ) -> bool {
        let lifecycle = self.timeleap_lifecycle_policy();
        self.has_timeleap_frame1_preview()
            || (self.timeleap_frame1_preview_is_pending()
                && self.results.len() == 1
                && self
                    .timeleap_frame1_input
                    .trim()
                    .parse::<i64>()
                    .ok()
                    .zip(self.current_sfmt_frame(fps))
                    .is_some_and(|(frame1, current)| {
                        frame1 >= current && frame1 <= crate::domain::rng::MAX_SFMT_FRAME
                    }))
            || (lifecycle.configuration_locked
                && matches!(
                    lifecycle.phase,
                    TimeleapLifecyclePhase::Frame1Waiting
                        | TimeleapLifecyclePhase::PostTimeleapRechecking
                ))
    }

    pub(crate) fn timeleap_frame1_preview_generation(&self) -> u64 {
        self.timeleap_frame1_preview_generation
    }

    pub(in crate::application::field::core::state) fn active_timeleap_frame1_target_consumption(
        &self,
    ) -> Option<i64> {
        self.timeleap_lifecycle_policy().frame1_boundary_target
    }

    pub(crate) fn timeleap_frame1_table_boundary(&self, frame1: i64) -> Option<usize> {
        self.timeleap_frame1_table_boundary
            .filter(|(cached_frame1, _)| *cached_frame1 == frame1)
            .map(|(_, boundary)| boundary)
    }

    pub(crate) fn timeleap_frame1_indicator(&self) -> Option<TimeleapFrame1Indicator> {
        if !self.timeleap_enabled {
            return None;
        }
        let reachable = self.timeleap_frame1_reachable?;
        Some(if reachable {
            TimeleapFrame1Indicator::Reachable
        } else {
            TimeleapFrame1Indicator::Unreachable
        })
    }

    pub(crate) fn set_timeleap_frame1_unreachable(&mut self) {
        self.timeleap_frame1_preview_state = TimeleapFrame1PreviewState::Unreachable;
        self.timeleap_target_check_pending = false;
        self.timeleap_target_reachable = Some(false);
        self.timeleap_frame1_reachable = Some(false);
        self.timeleap_frame1_deadline = None;
        self.timeleap_frame1_table_boundary = None;
        self.timeleap_frame1_wait_seconds = 0.0;
        if self.timeleap_enabled && self.timer_kind == FieldTimerKind::Target {
            self.timer_start = None;
            self.timer_finished = false;
        }
    }

    pub(crate) fn take_matching_timeleap_frame1_preview(
        &mut self,
        config: &TimeleapConfig,
        frame1: i64,
        start: &crate::domain::timeleap::TimeleapTimelineStart,
    ) -> Option<crate::domain::timeleap::TimeleapFrame1Preparation> {
        let state = std::mem::replace(
            &mut self.timeleap_frame1_preview_state,
            TimeleapFrame1PreviewState::Idle,
        );
        let TimeleapFrame1PreviewState::Ready(preview) = state else {
            self.timeleap_frame1_preview_state = state;
            return None;
        };
        let mut preview_config = preview.config;
        let mut current_config = config.clone();
        preview_config.start_frame = 0;
        current_config.start_frame = 0;
        (preview.frame1 == frame1
            && preview.target_frame == self.committed_target_frame()
            && preview_config == current_config
            && preview.start == *start)
            .then_some(preview.preparation)
    }

    pub(crate) fn begin_timeleap_candidate_search(&mut self) {
        self.timeleap_candidate_groups.clear();
        self.timeleap_selected_sfmt_position = None;
        self.timeleap_candidate_search_started = true;
        self.timeleap_search_status = TimeleapSearchStatus::Running;
        self.timeleap_candidate_search_key = None;
        self.timeleap_candidate_search_started_at = Some(Instant::now());
    }

    pub(crate) fn fail_timeleap_candidate_search(&mut self) {
        self.timeleap_candidate_groups.clear();
        self.timeleap_selected_sfmt_position = None;
        self.timeleap_search_status = TimeleapSearchStatus::Failed;
        self.timeleap_candidate_search_key = None;
        self.timeleap_candidate_search_started_at = None;
    }

    /// Stop only the explicit Frame1-candidate search lifecycle. Keep the
    /// Timeleap selection, prepared preview, and runtime state untouched.
    pub(crate) fn cancel_timeleap_candidate_search(&mut self) {
        if self.timeleap_search_status != TimeleapSearchStatus::Running {
            return;
        }
        self.timeleap_search_status = TimeleapSearchStatus::NotRequested;
        self.timeleap_candidate_search_started = false;
        self.timeleap_candidate_search_key = None;
        self.timeleap_candidate_search_started_at = None;
    }

    #[cfg(test)]
    pub(crate) fn timeleap_candidate_search_target_frame(&self) -> Option<i64> {
        self.timeleap_candidate_search_key
            .as_ref()
            .map(|key| key.config.target_frame)
    }

    pub(crate) fn set_timeleap_candidate_search_key(&mut self, key: TimeleapSearchKey) {
        self.timeleap_candidate_search_key = Some(key);
        self.timeleap_candidate_search_started_at = Some(Instant::now());
    }

    /// 検索時点からの相対Frame1 tickを、同じ通常Timelineの実行中だけライブ減算する。
    pub(crate) fn timeleap_candidate_seconds_remaining(
        &self,
        frame1_tick: i64,
        fps: f64,
        now: Instant,
    ) -> i64 {
        if self.timeleap_source_matches_running_timeline(fps) {
            let elapsed_seconds = self
                .timeleap_candidate_search_started_at
                .map(|started_at| now.saturating_duration_since(started_at).as_secs_f64())
                .unwrap_or_default();
            (crate::domain::timing::blink_ticks_to_seconds(frame1_tick, fps) - elapsed_seconds)
                .floor()
                .max(0.0) as i64
        } else {
            crate::domain::timing::blink_ticks_to_seconds(frame1_tick, fps)
                .floor()
                .max(0.0) as i64
        }
    }

    pub(crate) fn timeleap_candidate_search_inputs_ready(&self, _fps: f64) -> bool {
        if !self.timeleap_lifecycle_policy().candidate_search_enabled
            || (self.timeleap_consider_delay_enabled
                && self.timeleap_consider_delay_value().is_err())
        {
            return false;
        }
        self.parsed_timeleap_source().is_ok()
            && self
                .timeleap_generation_start_frame
                .trim()
                .parse::<i64>()
                .is_ok_and(|frame| (0..=crate::domain::rng::MAX_SFMT_FRAME).contains(&frame))
    }

    pub(crate) fn timeleap_search_completed_without_candidates(&self) -> bool {
        self.timeleap_search_status == TimeleapSearchStatus::Completed
            && self.timeleap_candidate_groups.is_empty()
    }

    pub(crate) fn timeleap_courier_validation(&self) -> Result<(), String> {
        if self.timeleap_mode != TimeleapMode::Courier {
            return Ok(());
        }
        if self.timeleap_courier_pid_type == 3 && self.timeleap_courier_pid.trim().is_empty() {
            return Err("固定PIDを入力してください。".into());
        }
        let iv_count = self
            .timeleap_courier_iv_count
            .trim()
            .parse::<u8>()
            .map_err(|_| "配達員のV個体数を確認してください。".to_string())?;
        if iv_count > 6 {
            return Err("配達員のV個体数は6以下にしてください。".into());
        }
        if self
            .timeleap_courier_fixed_ivs
            .iter()
            .any(|value| *value < -1 || *value > 31)
        {
            return Err("配達員の固定IVは-1から31で指定してください。".into());
        }
        let fixed_count = self
            .timeleap_courier_fixed_ivs
            .iter()
            .filter(|value| **value >= 0)
            .count();
        if iv_count as usize > 6usize.saturating_sub(fixed_count) {
            return Err("配達員の確定IV数とV個体数の組み合わせを確認してください。".into());
        }
        Ok(())
    }

    pub(super) fn timeleap_models(&self) -> Result<usize, String> {
        let models = if self.results.is_empty() {
            self.npc_count
                .trim()
                .parse::<usize>()
                .map(|npc_count| npc_count.saturating_add(1))
                .map_err(|_| "NPC数を確認してください。".to_owned())?
        } else {
            self.results
                .get(self.selected_result)
                .map(|result| result.models)
                .ok_or_else(|| "NPC数を確認してください。".to_owned())?
        };
        if (1..=MAX_MODELS).contains(&models) {
            Ok(models)
        } else {
            Err("NPC数を確認してください。".to_owned())
        }
    }

    pub(crate) fn prepare_timeleap_candidate_search(
        &mut self,
        fps: f64,
    ) -> Result<(TimeleapSearchKey, Option<ObservationCache>), String> {
        if self.timeleap_enabled && self.timeleap_consider_delay_enabled {
            self.timeleap_consider_delay_value()?;
        }
        self.timeleap_courier_validation()?;
        let (seed, start_frame, models) = self.parsed_timeleap_source()?;
        let current_tick = if self.timeleap_source_matches_running_timeline(fps) {
            self.current_timeline_frame_at(fps, Instant::now())
                .unwrap_or_default()
                .div_euclid(2)
        } else {
            0
        };
        let timeline_start =
            crate::domain::timeleap::TimeleapTimelineStart::initial(start_frame, models);
        let (min_ticks, max_ticks, frame1_filter) = if self.timeleap_filter_mode == 1 {
            let min_frame = parse_bounded_timeleap_value(
                &self.timeleap_frame1_min,
                0,
                crate::domain::rng::MAX_SFMT_FRAME,
                "Frame1の下限",
            )?;
            let max_frame = parse_bounded_timeleap_value(
                &self.timeleap_frame1_max,
                0,
                crate::domain::rng::MAX_SFMT_FRAME,
                "Frame1の上限",
            )?;
            if min_frame > max_frame {
                return Err("Frame1の下限は上限以下にしてください。".into());
            }
            let max_ticks = max_frame
                .saturating_sub(start_frame)
                .max(0)
                .div_euclid(2)
                .saturating_add(1);
            (0, max_ticks, Some((min_frame, max_frame)))
        } else {
            let min_seconds =
                parse_bounded_timeleap_value(&self.timeleap_seconds_min, 0, 999, "秒数の下限")?;
            let max_seconds =
                parse_bounded_timeleap_value(&self.timeleap_seconds_max, 0, 999, "秒数の上限")?;
            if min_seconds > max_seconds {
                return Err("秒数の下限は上限以下にしてください。".into());
            }
            (
                min_seconds.saturating_mul(30),
                max_seconds
                    .saturating_add(1)
                    .saturating_mul(30)
                    .saturating_sub(1),
                None,
            )
        };
        let handoff_range =
            parse_bounded_timeleap_value(&self.timeleap_search_range, 0, 98, "乗り移り範囲")?;
        let generation_start_frame = self
            .timeleap_generation_start_frame
            .trim()
            .parse::<i64>()
            .map_err(|_| "生成開始Frameを確認してください。")?;
        if !(0..=crate::domain::rng::MAX_SFMT_FRAME).contains(&generation_start_frame) {
            return Err("生成開始Frameは0〜50,000,000Fで入力してください。".into());
        }
        let config = TimeleapConfig {
            seed,
            start_frame,
            target_frame: generation_start_frame,
            models,
            min_frame1_elapsed_ticks: min_ticks,
            max_frame1_elapsed_ticks: max_ticks,
            mode: self.timeleap_mode,
            is_boy: self.timeleap_is_boy,
            handoff_range_frames: handoff_range,
            courier: CourierConfig {
                no_dex: self.timeleap_no_dex && self.timeleap_uses_courier_method(),
                your_id: self.timeleap_courier_your_id,
                is_egg: self.timeleap_courier_is_egg,
                iv_count: self
                    .timeleap_courier_iv_count
                    .trim()
                    .parse()
                    .map_err(|_| "配達員のV個体数を確認してください。")?,
                ability_locked: self.timeleap_courier_ability_locked,
                nature_locked: self.timeleap_courier_nature_locked,
                gender_locked: self.timeleap_courier_gender_locked,
                ec: if self.timeleap_courier_ec.trim().is_empty() {
                    None
                } else {
                    Some(parse_seed(&self.timeleap_courier_ec)?)
                },
                pid_type: self.timeleap_courier_pid_type,
                pid: if self.timeleap_courier_pid.trim().is_empty() {
                    0
                } else {
                    parse_seed(&self.timeleap_courier_pid)?
                },
                fixed_ivs: self.timeleap_courier_fixed_ivs,
                ability: self.timeleap_courier_ability,
                nature: self.timeleap_courier_nature,
                gender_ratio: self.timeleap_courier_gender_ratio,
            },
        };
        let consider_delay = if self.timeleap_enabled && self.timeleap_consider_delay_enabled {
            Some(crate::domain::timeleap::ConsiderDelayConfig {
                delay_frames: self.timeleap_consider_delay_value()?,
                generator: self.timeleap_consider_delay_generator(),
                courier: config.courier.clone(),
            })
        } else {
            None
        };
        let grace_min = parse_bounded_timeleap_value(&self.timeleap_grace_min, 0, 98, "猶予Frame")?;
        let target_grace_min =
            parse_bounded_timeleap_value(&self.timeleap_target_grace_min, 0, 98, "猶予Frame")?;
        let cache = self
            .observation_cache
            .as_ref()
            .filter(|cache| cache.seed == seed)
            .cloned();
        Ok((
            TimeleapSearchKey {
                config,
                consider_delay,
                grace_min,
                target_grace_min,
                frame1_filter,
                timeline_start,
                current_tick,
            },
            cache,
        ))
    }

    pub(crate) fn prepare_timeleap_frame1(
        &mut self,
        fps: f64,
    ) -> Result<
        (
            TimeleapConfig,
            i64,
            ObservationCache,
            crate::domain::timeleap::TimeleapTimelineStart,
        ),
        String,
    > {
        if self.timeleap_enabled && self.timeleap_consider_delay_enabled {
            self.timeleap_consider_delay_value()?;
        }
        let models = self.timeleap_models()?;
        let frame1 = self
            .timeleap_frame1_input
            .trim()
            .parse::<i64>()
            .map_err(|_| "Frame1を入力してください。".to_string())?;
        self.timeleap_courier_validation()?;
        let start_frame = self.timeleap_start_frame(fps)?;
        if frame1 < start_frame || frame1 > crate::domain::rng::MAX_SFMT_FRAME {
            return Err("Frame1は現在位置以降の範囲で入力してください。".into());
        }
        let seed = parse_seed(&self.seed)?;
        let start = self
            .results
            .get(self.selected_result)
            .and_then(crate::application::field::FieldResult::timeleap_timeline_start)
            .unwrap_or_else(|| {
                crate::domain::timeleap::TimeleapTimelineStart::initial(start_frame, models)
            });
        // Re-search range is selected after Frame1 passes. Keep every supported
        // branch ready before Ready so changing the range does not require a
        // new Frame1 simulation after the in-game action.
        let handoff_range_frames = 98;
        let config = TimeleapConfig {
            seed,
            start_frame,
            // Frame1準備ではTarget到達判定を行わない。この値は実行候補生成に使わない。
            target_frame: frame1,
            models,
            min_frame1_elapsed_ticks: 0,
            max_frame1_elapsed_ticks: 0,
            mode: self.timeleap_mode,
            is_boy: self.timeleap_is_boy,
            handoff_range_frames,
            courier: CourierConfig {
                no_dex: self.timeleap_no_dex && self.timeleap_uses_courier_method(),
                your_id: self.timeleap_courier_your_id,
                is_egg: self.timeleap_courier_is_egg,
                iv_count: self
                    .timeleap_courier_iv_count
                    .trim()
                    .parse()
                    .map_err(|_| "配達員のV個体数を確認してください。")?,
                ability_locked: self.timeleap_courier_ability_locked,
                nature_locked: self.timeleap_courier_nature_locked,
                gender_locked: self.timeleap_courier_gender_locked,
                ec: if self.timeleap_courier_ec.trim().is_empty() {
                    None
                } else {
                    Some(parse_seed(&self.timeleap_courier_ec)?)
                },
                pid_type: self.timeleap_courier_pid_type,
                pid: if self.timeleap_courier_pid.trim().is_empty() {
                    0
                } else {
                    parse_seed(&self.timeleap_courier_pid)?
                },
                fixed_ivs: self.timeleap_courier_fixed_ivs,
                ability: self.timeleap_courier_ability,
                nature: self.timeleap_courier_nature,
                gender_ratio: self.timeleap_courier_gender_ratio,
            },
        };
        let cache = self
            .observation_cache
            .as_ref()
            .filter(|cache| cache.seed == seed)
            .ok_or_else(|| "Timeleap用のSFMTキャッシュがありません。".to_string())?
            .clone();
        Ok((config, frame1, cache, start))
    }

    pub(super) fn timeleap_start_frame(&mut self, fps: f64) -> Result<i64, String> {
        if let Some(current_sfmt_frame) = self.current_sfmt_frame(fps) {
            return Ok(current_sfmt_frame);
        }

        // Before a resolved Timeline exists, retain the typed starting value for
        // direct/manual preparation paths. A confirmed GUI Timeline must never
        // silently fall back to that stale observation input.
        if self.results.is_empty() {
            return self
                .range_start
                .trim()
                .parse()
                .map_err(|_| "現在Frameを確認してください。".to_string());
        }

        Err("現在Frameを確認してください。".to_string())
    }

    pub(crate) fn begin_timeleap_frame1_preparation(&mut self) {
        self.clear_notice();
        // Readyが所有する以後の時計を、Ready前のpreview完了が上書きしない。
        self.timeleap_frame1_preview_generation =
            self.timeleap_frame1_preview_generation.wrapping_add(1);
        self.timeleap_frame1_preview_state = TimeleapFrame1PreviewState::Idle;
        self.timeleap_target_check_pending = false;
        self.timeleap_frame1_preparing = true;
        self.timeleap_nominal_tick = None;
        self.timeleap_execution_candidates.clear();
        self.timeleap_locked = true;
        self.target_input_locked = true;
    }

    pub(crate) fn abort_timeleap_frame1_preparation(&mut self) {
        self.timeleap_frame1_preparing = false;
        self.timeleap_nominal_tick = None;
        self.timeleap_locked = false;
        self.target_input_locked = false;
    }

    pub(crate) fn apply_timeleap_frame1_candidates(
        &mut self,
        preparation: crate::domain::timeleap::TimeleapFrame1Preparation,
        fps: f64,
    ) {
        let (deadline, _) = self.frame1_deadline(preparation.nominal_tick, Instant::now(), fps);
        self.apply_timeleap_frame1_candidates_at(preparation, deadline, fps);
    }

    /// Frame1 timer ends at the selected pre-event Frame1 position. Consider Delay
    /// is simulated only after the Timeleap event while checking Target reachability.
    pub(super) fn frame1_deadline(
        &self,
        nominal_tick: i64,
        now: Instant,
        fps: f64,
    ) -> (Instant, Duration) {
        let remaining_ticks = self.timeleap_frame1_remaining_ticks(nominal_tick, fps, now);
        let duration = Duration::from_secs_f64(remaining_ticks as f64 * 2.0 / fps);
        let deadline = Self::apply_frame1_adjustment(
            now.checked_add(duration).unwrap_or(now),
            self.adjust,
            fps,
        );
        (deadline, duration)
    }

    pub(super) fn apply_frame1_adjustment(deadline: Instant, adjust: i64, fps: f64) -> Instant {
        let shift =
            Duration::from_secs_f64(game_frames_to_seconds(adjust.unsigned_abs() as f64, fps));
        if adjust.is_positive() {
            deadline.checked_sub(shift).unwrap_or(deadline)
        } else if adjust.is_negative() {
            deadline.checked_add(shift).unwrap_or(deadline)
        } else {
            deadline
        }
    }

    pub(super) fn timeleap_frame1_remaining_ticks(
        &self,
        nominal_tick: i64,
        fps: f64,
        now: Instant,
    ) -> i64 {
        let current_tick = self
            .current_timeline_frame_at(fps, now)
            .map(|frame| frame.div_euclid(2))
            .unwrap_or_default();
        nominal_tick.saturating_sub(current_tick).max(0)
    }

    pub(crate) fn apply_timeleap_frame1_candidates_at(
        &mut self,
        preparation: crate::domain::timeleap::TimeleapFrame1Preparation,
        deadline: Instant,
        fps: f64,
    ) {
        self.timeleap_frame1_preparing = false;
        self.timeleap_frame1_reachable = Some(true);
        let selected_sfmt_position = preparation
            .candidates
            .iter()
            .find(|candidate| candidate.frame1 == preparation.nominal_frame1)
            .or_else(|| preparation.candidates.first())
            .map(|candidate| candidate.post_sfmt_position);
        self.timeleap_execution_candidates = preparation.candidates;
        self.timeleap_selected_frame = Some(preparation.nominal_frame1);
        self.timeleap_selected_sfmt_position = selected_sfmt_position;
        self.timeleap_frame1_target = Some(preparation.nominal_frame1);
        self.timeleap_nominal_tick = Some(preparation.nominal_tick);
        self.timeleap_observation_anchor_frame = Some(preparation.nominal_tick.saturating_mul(2));
        self.timeleap_confirmed_frame1 = None;
        self.timeleap_confirmed_candidate = None;
        self.timeleap_locked = true;
        self.timeleap_stage = TimeleapStage::Frame1Waiting;
        self.cancel_timeleap_shift_resolution();
        self.timeleap_shift_times.clear();
        self.timeleap_wait_started = Some(Instant::now());
        self.timeleap_frame1_deadline = Some(deadline);
        self.timer_finished = false;
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .as_secs_f64();
        self.timeleap_frame1_wait_seconds = remaining;
        if self.timer_kind == FieldTimerKind::Target {
            self.start_timeleap_frame1_timer(Instant::now(), fps);
        }
        self.target_input_locked = true;
    }

    pub(crate) fn timeleap_search_key_matches(&self, key: &TimeleapSearchKey) -> bool {
        self.timeleap_candidate_search_key.as_ref() == Some(key)
    }

    pub(crate) fn apply_timeleap_candidate_groups(&mut self, groups: Vec<TimeleapCandidateGroup>) {
        self.timeleap_candidate_groups = groups;
        self.timeleap_search_status = TimeleapSearchStatus::Completed;
        self.timeleap_selected_sfmt_position = select_candidate(
            &self.timeleap_candidate_groups,
            self.timeleap_selected_frame,
        )
        .map(|selection| selection.post_sfmt_position);
    }

    /// Apply the representative Frame1 and A-input TargetFrame from a search row.
    pub(crate) fn set_timeleap_candidate_inputs(&mut self, frame1: i64, target_frame: i64) -> bool {
        if self.timeleap_locked
            || self.timeleap_frame1_preparing
            || !(0..=crate::domain::rng::MAX_SFMT_FRAME).contains(&frame1)
            || !(0..=crate::domain::rng::MAX_SFMT_FRAME).contains(&target_frame)
        {
            return false;
        }

        self.confirmed_target_timeline_generation =
            self.confirmed_target_timeline_generation.wrapping_add(1);
        self.invalidate_confirmed_target_search();

        self.timeleap_selected_frame = Some(frame1);
        self.timeleap_frame1_input = frame1.to_string();
        self.target_consumption = target_frame.to_string();
        self.target_dirty = true;
        self.commit_target_input().is_ok()
    }

    pub fn timeleap_accepts_shift(&self) -> bool {
        !self.timeleap_shift_resolution_pending
            && self.timeleap_locked
            && matches!(
                self.timeleap_stage,
                TimeleapStage::ShiftWaiting | TimeleapStage::Recheck
            )
    }
}
