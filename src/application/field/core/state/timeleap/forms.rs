use super::*;

pub(super) fn confirmed_target_search_failure_status(
    failure: crate::domain::timeleap::TimeleapSearchFailure,
) -> Option<ConfirmedTargetSearchStatus> {
    use crate::domain::timeleap::TimeleapSearchFailure;

    match failure {
        TimeleapSearchFailure::Cancelled => None,
        TimeleapSearchFailure::Cache(error) => {
            Some(ConfirmedTargetSearchStatus::Failed(error.to_string()))
        }
        TimeleapSearchFailure::Calculation(message) => {
            Some(ConfirmedTargetSearchStatus::Failed(message))
        }
        TimeleapSearchFailure::Frame1Unreachable => {
            Some(ConfirmedTargetSearchStatus::Frame1Unreachable)
        }
        TimeleapSearchFailure::GenerationFrameUnreachable => {
            Some(ConfirmedTargetSearchStatus::GenerationFrameUnreachable)
        }
    }
}

impl FieldState {
    pub(crate) fn timeleap_courier_category(&self) -> usize {
        crate::domain::fixed_rng_catalog::categories(self.timeleap_game_edition).len()
    }

    pub(crate) fn timeleap_is_courier_category(&self) -> bool {
        self.timeleap_search_category == self.timeleap_courier_category()
    }

    pub(super) fn timeleap_consider_delay_generator(
        &self,
    ) -> crate::domain::timeleap::ConsiderDelayGenerator {
        if self.timeleap_is_courier_category() {
            return crate::domain::timeleap::ConsiderDelayGenerator::Event;
        }

        crate::domain::timeleap::ConsiderDelayGenerator::FixedSymbol(
            crate::domain::fixed_rng_catalog::selected_preset_for(
                self.timeleap_game_edition,
                self.timeleap_search_category,
                self.timeleap_search_preset,
            )
            .map(|preset| preset.delay_type)
            .unwrap_or_default(),
        )
    }

    pub(crate) fn refresh_timeleap_courier_delay(&mut self) {
        if !self.timeleap_is_courier_category() {
            return;
        }
        let has_event_generation = self.timeleap_no_dex
            || (self.timeleap_courier_your_id && !self.timeleap_courier_is_egg);
        let delay = if has_event_generation {
            self.timeleap_game_edition.courier_event_delay_frames()
        } else {
            0
        };
        self.timeleap_consider_delay_frames = delay.to_string();
    }

    pub(crate) fn courier_condition_changed(&mut self) -> bool {
        self.refresh_timeleap_courier_delay();
        self.invalidate_timeleap_frame1_preview_after_input_change()
    }

    pub(crate) fn invalidate_timeleap_frame1_preview_after_input_change(&mut self) -> bool {
        if self.timeleap_enabled
            && !self.timeleap_locked
            && self.timeleap_stage == crate::application::field::TimeleapStage::Off
        {
            self.invalidate_timeleap_frame1_preview();
            true
        } else {
            false
        }
    }

    pub(crate) fn set_timeleap_game_edition(
        &mut self,
        edition: crate::domain::fixed_rng_catalog::GameEdition,
    ) -> bool {
        use crate::domain::fixed_rng_catalog::{
            categories, find_unique_preset, selected_preset_for,
        };

        if self.timeleap_game_edition == edition {
            return false;
        }

        let was_courier =
            self.timeleap_search_category == categories(self.timeleap_game_edition).len();
        let previous_preset = (!was_courier)
            .then(|| {
                selected_preset_for(
                    self.timeleap_game_edition,
                    self.timeleap_search_category,
                    self.timeleap_search_preset,
                )
                .copied()
            })
            .flatten();

        self.timeleap_game_edition = edition;
        if was_courier {
            self.timeleap_search_category = categories(edition).len();
            self.timeleap_search_preset = 0;
            self.timeleap_mode = crate::domain::timeleap::TimeleapMode::Courier;
            self.refresh_timeleap_courier_delay();
        } else {
            let mapped = previous_preset
                .and_then(|preset| find_unique_preset(edition, preset.species_id, preset.suffix));
            if let Some((category, preset)) = mapped {
                self.timeleap_search_category = category;
                self.timeleap_search_preset = preset;
            } else {
                self.timeleap_search_category = 0;
                self.timeleap_search_preset = 0;
            }
            if let Some(preset) = selected_preset_for(
                edition,
                self.timeleap_search_category,
                self.timeleap_search_preset,
            )
            .copied()
            {
                self.apply_timeleap_preset(&preset);
            }
        }

        self.invalidate_timeleap_frame1_preview_after_input_change()
    }

    pub(crate) fn apply_timeleap_preset(
        &mut self,
        preset: &crate::domain::fixed_rng_catalog::FixedRngPreset,
    ) {
        use crate::domain::fixed_rng_catalog::PresetMode;

        match preset.mode {
            PresetMode::Menu => {
                self.timeleap_mode = TimeleapMode::Menu;
                self.timeleap_is_boy = self.timeleap_default_is_boy;
            }
            PresetMode::Dialogue => self.timeleap_mode = TimeleapMode::Dialogue,
            PresetMode::Unsupported => {}
        }
        self.timeleap_consider_delay_frames = preset.delay_frames.to_string();
    }

    pub(crate) fn timeleap_uses_courier_method(&self) -> bool {
        self.timeleap_mode == TimeleapMode::Courier
    }

    #[allow(dead_code)] // 到達可否タブは非表示だが、計算機能の状態初期化は残す。
    pub(crate) fn initialize_timeleap_reachability_form(&mut self) {
        if self.timeleap_reachability_form.initialized {
            return;
        }
        let seed = self.seed.clone();
        let resolved_result = (self.results.len() == 1)
            .then(|| self.results.get(self.selected_result))
            .flatten()
            .filter(|result| !result.timeline.is_empty());
        let models = resolved_result
            .map(|result| result.models)
            .or_else(|| {
                self.npc_count
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .map(|npc_count| npc_count.saturating_add(1))
            })
            .unwrap_or_default();
        // Match the StartingFrame shown in the search-window header. Do not
        // substitute the live runtime cursor when that displayed value is unknown.
        let starting_frame = resolved_result
            .and_then(|result| self.starting_frame_for_display(result))
            .map(|frame| frame.to_string())
            .unwrap_or_default();
        let frame1 = self
            .timeleap_frame1_target
            .or_else(|| self.timeleap_frame1_input.trim().parse().ok())
            .map(|frame| frame.to_string())
            .unwrap_or_else(|| starting_frame.clone());
        let courier = self.timeleap_courier_config().unwrap_or_default();
        let form = &mut self.timeleap_reachability_form;
        form.seed = seed;
        form.models = models;
        form.game_edition = self.timeleap_game_edition;
        form.category = self.timeleap_search_category;
        form.preset = self.timeleap_search_preset;
        form.mode = self.timeleap_mode;
        form.is_boy = self.timeleap_is_boy;
        form.starting_frame = starting_frame;
        form.frame1 = frame1;
        form.consider_delay_enabled = self.timeleap_consider_delay_enabled;
        form.consider_delay_frames = self.timeleap_consider_delay_frames.clone();
        form.generation_start_frame = self.timeleap_generation_start_frame.clone();
        form.courier = courier;
        form.status = TimeleapReachabilityStatus::Idle;
        form.initialized = true;
    }

    pub(crate) fn begin_timeleap_reachability_check(
        &mut self,
    ) -> Result<
        (
            u64,
            crate::domain::timeleap::TimeleapReachabilityRequest,
            ObservationCache,
        ),
        String,
    > {
        let form = self.timeleap_reachability_form.clone();
        let seed = parse_seed(&form.seed)?;
        if !(1..=MAX_MODELS).contains(&form.models) {
            return Err("NPC数を確認してください。".into());
        }
        let starting_frame = parse_bounded_timeleap_value(
            &form.starting_frame,
            0,
            crate::domain::rng::MAX_SFMT_FRAME,
            "StartingFrame",
        )?;
        let frame1 = parse_bounded_timeleap_value(
            &form.frame1,
            0,
            crate::domain::rng::MAX_SFMT_FRAME,
            "Frame1",
        )?;
        let generation_start_frame = parse_bounded_timeleap_value(
            &form.generation_start_frame,
            0,
            crate::domain::rng::MAX_SFMT_FRAME,
            "生成開始Frame",
        )?;
        if starting_frame > generation_start_frame {
            return Err("StartingFrameは生成開始Frame以下にしてください。".into());
        }
        let delay_frames =
            parse_bounded_timeleap_value(&form.consider_delay_frames, 0, 9998, "Consider Delay")?;
        Self::timeleap_courier_validation_for(&form.courier)?;
        let mode = form.mode;
        let courier = form.courier.clone();
        let config = crate::domain::timeleap::TimeleapConfig {
            seed,
            start_frame: starting_frame,
            target_frame: generation_start_frame,
            models: form.models,
            min_frame1_elapsed_ticks: 0,
            max_frame1_elapsed_ticks: 0,
            mode,
            is_boy: form.is_boy,
            handoff_range_frames: 0,
            courier: courier.clone(),
        };
        let delay = timeleap_consider_delay_config(
            form.game_edition,
            form.category,
            form.preset,
            form.consider_delay_enabled,
            delay_frames,
            &courier,
        );
        let cache = self
            .observation_cache
            .as_ref()
            .filter(|cache| cache.seed == seed)
            .ok_or_else(|| "入力SeedのSFMTキャッシュがありません。".to_owned())?
            .clone();
        let form = &mut self.timeleap_reachability_form;
        form.generation = form.generation.wrapping_add(1);
        form.status = TimeleapReachabilityStatus::Running;
        Ok((
            form.generation,
            crate::domain::timeleap::TimeleapReachabilityRequest {
                config,
                frame1,
                consider_delay: delay,
            },
            cache,
        ))
    }

    pub(crate) fn apply_timeleap_reachability_result(
        &mut self,
        generation: u64,
        result: Result<
            crate::domain::timeleap::TimeleapReachability,
            crate::domain::timeleap::TimeleapSearchFailure,
        >,
    ) {
        let form = &mut self.timeleap_reachability_form;
        if form.generation != generation {
            return;
        }
        form.status = match result {
            Ok(crate::domain::timeleap::TimeleapReachability::Frame1Unreachable) => {
                TimeleapReachabilityStatus::Frame1Unreachable
            }
            Ok(crate::domain::timeleap::TimeleapReachability::GenerationFrameUnreachable) => {
                TimeleapReachabilityStatus::GenerationFrameUnreachable
            }
            Ok(crate::domain::timeleap::TimeleapReachability::Reachable {
                target_input_frames,
            }) => TimeleapReachabilityStatus::Reachable(target_input_frames),
            Err(crate::domain::timeleap::TimeleapSearchFailure::Cancelled) => {
                TimeleapReachabilityStatus::Idle
            }
            Err(crate::domain::timeleap::TimeleapSearchFailure::Cache(error)) => {
                TimeleapReachabilityStatus::Failed(error.to_string())
            }
            Err(crate::domain::timeleap::TimeleapSearchFailure::Calculation(message)) => {
                TimeleapReachabilityStatus::Failed(message)
            }
            Err(crate::domain::timeleap::TimeleapSearchFailure::Frame1Unreachable) => {
                TimeleapReachabilityStatus::Frame1Unreachable
            }
            Err(crate::domain::timeleap::TimeleapSearchFailure::GenerationFrameUnreachable) => {
                TimeleapReachabilityStatus::GenerationFrameUnreachable
            }
        };
    }

    pub(crate) fn initialize_confirmed_target_search_form(&mut self) {
        if self.confirmed_target_search_form.initialized {
            return;
        }
        let form = &mut self.confirmed_target_search_form;
        form.initialized = true;
        form.drift_range_ticks = defaults::FIELD_TIMELEAP_DRIFT_RANGE_TICKS.into();
        form.grace_min_frames = "0".into();
    }

    pub(crate) fn initialize_timeleap_source_inputs(&mut self) {
        if self.timeleap_source_inputs_initialized {
            return;
        }
        self.timeleap_source_inputs_initialized = true;
        self.timeleap_source_seed = self.seed.clone();
        let internal = self.internal_timeleap_source_values();
        if let Some((starting_frame, npc_count)) = internal {
            self.timeleap_source_starting_frame = starting_frame.to_string();
            self.timeleap_source_npc_count = npc_count.to_string();
        }
        if let Some(frame1) = self.timeleap_confirmed_frame1 {
            self.timeleap_source_frame1 = frame1.to_string();
        } else if let Some(frame1) = valid_frame1_input(&self.timeleap_frame1_input) {
            self.timeleap_source_frame1 = frame1.to_string();
        }
    }

    pub(crate) fn sync_timeleap_source_inputs_from_internal_timeline(&mut self) {
        if !self.timeleap_source_inputs_initialized {
            return;
        }
        let Some((starting_frame, npc_count)) = self.internal_timeleap_source_values() else {
            return;
        };
        self.timeleap_source_seed.clone_from(&self.seed);
        self.timeleap_source_starting_frame = starting_frame.to_string();
        self.timeleap_source_npc_count = npc_count.to_string();
    }

    pub(crate) fn reset_timeleap_source_inputs(&mut self) {
        self.timeleap_source_seed = self.seed.clone();
        self.timeleap_source_starting_frame.clear();
        self.timeleap_source_npc_count.clear();
        self.timeleap_source_frame1.clear();
        if let Some((starting_frame, npc_count)) = self.internal_timeleap_source_values() {
            self.timeleap_source_starting_frame = starting_frame.to_string();
            self.timeleap_source_npc_count = npc_count.to_string();
        }
        if let Some(frame1) = self.timeleap_source_frame1_reset_value() {
            self.timeleap_source_frame1 = frame1.to_string();
        }
        self.timeleap_source_inputs_initialized = true;
    }

    pub(crate) fn can_edit_timeleap_frame1_input(&self) -> bool {
        self.timeleap_lifecycle_policy().frame1_input_enabled
    }

    fn timeleap_source_frame1_reset_value(&self) -> Option<i64> {
        self.timeleap_confirmed_frame1
            .or_else(|| valid_frame1_input(&self.timeleap_frame1_input))
    }

    pub(crate) fn sync_timeleap_source_frame1_from_reset_value(&mut self) {
        if self.timeleap_source_inputs_initialized {
            if let Some(frame1) = self.timeleap_source_frame1_reset_value() {
                self.timeleap_source_frame1 = frame1.to_string();
            }
        }
    }

    pub(crate) fn set_timeleap_source_frame1(
        &mut self,
        frame1: String,
        main_frame1_editable: bool,
    ) {
        self.timeleap_source_frame1 = frame1;
        if main_frame1_editable {
            self.timeleap_frame1_input
                .clone_from(&self.timeleap_source_frame1);
        }
    }

    pub(crate) fn sync_timeleap_source_frame1_from_main(&mut self) {
        if self.timeleap_source_inputs_initialized {
            self.timeleap_source_frame1
                .clone_from(&self.timeleap_frame1_input);
        }
    }

    pub(crate) fn restore_timeleap_source_frame1_from_main(&mut self) {
        if let Some(frame1) = valid_frame1_input(&self.timeleap_frame1_input) {
            self.timeleap_source_frame1 = frame1.to_string();
        }
    }

    pub(crate) fn set_timeleap_source_seed(&mut self, seed: String, main_seed_editable: bool) {
        self.timeleap_source_seed = seed;
        if main_seed_editable {
            self.seed.clone_from(&self.timeleap_source_seed);
        }
    }

    pub(crate) fn sync_timeleap_source_seed_from_main(&mut self) {
        if self.timeleap_source_inputs_initialized {
            self.timeleap_source_seed.clone_from(&self.seed);
        }
    }

    pub(crate) fn restore_timeleap_source_seed_from_main(&mut self) {
        self.timeleap_source_seed.clone_from(&self.seed);
    }

    pub(super) fn internal_timeleap_source_values(&self) -> Option<(i64, usize)> {
        let result = self.results.get(self.selected_result)?;
        if result.timeline.is_empty()
            && result.timeline_cursor.is_none()
            && result.target_cursor.is_none()
            && self.results.len() != 1
        {
            return None;
        }
        let starting_frame = if self.timeleap_timeline_adopted() {
            self.timeleap_original_starting_frame?
        } else {
            result.start_consumption
        };
        Some((starting_frame, result.models.saturating_sub(1)))
    }

    pub(crate) fn parsed_timeleap_source(&self) -> Result<(u32, i64, usize), String> {
        let seed = parse_seed(&self.timeleap_source_seed)?;
        let starting_frame = parse_bounded_timeleap_value(
            &self.timeleap_source_starting_frame,
            0,
            crate::domain::rng::MAX_SFMT_FRAME,
            "StartingFrame",
        )?;
        let npc_count = parse_bounded_timeleap_value(
            &self.timeleap_source_npc_count,
            0,
            (MAX_MODELS - 1) as i64,
            "NPC数",
        )? as usize;
        Ok((seed, starting_frame, npc_count + 1))
    }

    pub(crate) fn timeleap_source_matches_running_timeline(&self, fps: f64) -> bool {
        if !self.timer_is_running() || self.timeleap_timeline_adopted() {
            return false;
        }
        let Ok((seed, starting_frame, models)) = self.parsed_timeleap_source() else {
            return false;
        };
        let Some(result) = self.results.get(self.selected_result) else {
            return false;
        };
        seed == parse_seed(&self.seed).unwrap_or(u32::MAX)
            // Display may intentionally hide the pre-Timeleap StartingFrame.
            // Matching uses the underlying ordinary Timeline's real origin.
            && starting_frame == result.start_consumption
            && models == result.models
            && self.current_timeline_frame(fps).is_some()
    }

    pub(crate) fn invalidate_confirmed_target_search(&mut self) {
        let form = &mut self.confirmed_target_search_form;
        form.generation = form.generation.wrapping_add(1);
        form.request = None;
        if matches!(form.status, ConfirmedTargetSearchStatus::Running) {
            form.status = form
                .previous_status
                .take()
                .map(restore_confirmed_target_search_status)
                .unwrap_or_default();
        } else {
            form.previous_status = None;
        }
    }

    /// Complete a user-requested cancellation after its worker has stopped,
    /// restoring the prior result snapshot or the initial Idle state.
    pub(crate) fn cancel_confirmed_target_search(&mut self) {
        if self.confirmed_target_search_form.request.is_some() {
            self.invalidate_confirmed_target_search();
        }
    }

    pub(crate) fn fail_confirmed_target_search(&mut self, message: String) {
        let form = &mut self.confirmed_target_search_form;
        form.generation = form.generation.wrapping_add(1);
        form.request = None;
        form.previous_status = None;
        form.status = ConfirmedTargetSearchStatus::Failed(message);
    }

    pub(crate) fn confirmed_target_search_is_running(&self) -> bool {
        self.confirmed_target_search_form.request.is_some()
    }

    pub(crate) fn begin_confirmed_target_search(
        &mut self,
    ) -> Result<
        (
            u64,
            crate::domain::timeleap::ConfirmedTargetSearchRequest,
            Option<ObservationCache>,
        ),
        String,
    > {
        self.initialize_timeleap_source_inputs();
        self.initialize_confirmed_target_search_form();
        let (seed, start_frame, models) = self.parsed_timeleap_source()?;
        let confirmed_frame1 = parse_bounded_timeleap_value(
            &self.timeleap_source_frame1,
            start_frame,
            crate::domain::rng::MAX_SFMT_FRAME,
            "Frame1",
        )?;
        let generation_start_frame = parse_bounded_timeleap_value(
            &self.timeleap_generation_start_frame,
            0,
            crate::domain::rng::MAX_SFMT_FRAME,
            "生成開始Frame",
        )?;
        let drift_search_generation_start_frame =
            if self.confirmed_target_search_form.drift_search_enabled {
                Some(parse_bounded_timeleap_value(
                    &self
                        .confirmed_target_search_form
                        .drift_search_generation_start_frame,
                    0,
                    crate::domain::rng::MAX_SFMT_FRAME,
                    "ズレ検索の生成開始Frame",
                )?)
            } else {
                None
            };
        let drift_range_ticks = parse_bounded_timeleap_value(
            &self.confirmed_target_search_form.drift_range_ticks,
            0,
            100_000,
            "ずれ検索tick範囲",
        )?;
        let grace_min_frames = parse_bounded_timeleap_value(
            &self.confirmed_target_search_form.grace_min_frames,
            0,
            98,
            "猶予",
        )?;
        let baseline_target_frame = self
            .committed_target_frame()
            .ok_or_else(|| "基準となるTargetFrameを入力してください。".to_owned())?;
        let courier = self.timeleap_courier_config()?;
        let consider_delay = if self.timeleap_consider_delay_enabled {
            Some(crate::domain::timeleap::ConsiderDelayConfig {
                delay_frames: self.timeleap_consider_delay_value()?,
                generator: self.timeleap_consider_delay_generator(),
                courier,
            })
        } else {
            None
        };
        let cache = self
            .observation_cache
            .as_ref()
            .filter(|cache| cache.seed == seed)
            .cloned();
        let source_config = crate::domain::timeleap::TimeleapConfig {
            seed,
            start_frame,
            target_frame: generation_start_frame
                .max(confirmed_frame1)
                .max(start_frame),
            models,
            min_frame1_elapsed_ticks: 0,
            max_frame1_elapsed_ticks: 0,
            mode: self.timeleap_mode,
            is_boy: self.timeleap_is_boy,
            handoff_range_frames: 0,
            courier: self.timeleap_courier_config()?,
        };
        let fallback_start = self
            .timeleap_confirmed_candidate
            .as_ref()
            .map(
                |candidate| crate::domain::timeleap::ConfirmedTimeleapStart {
                    frame: candidate.post_frame,
                    sfmt_position: candidate.post_sfmt_position,
                    tick: candidate.post_tick,
                    status: candidate.post_status.clone(),
                },
            )
            .unwrap_or_else(|| crate::domain::timeleap::ConfirmedTimeleapStart {
                frame: start_frame,
                sfmt_position: start_frame,
                tick: 0,
                status: crate::domain::timeleap::TimeleapStatus {
                    model: crate::domain::rng::ModelStatus::new(models),
                    phase: 0,
                    fidget_cd: -1,
                    is_boy: self.timeleap_is_boy,
                    raining: false,
                },
            });
        let request = crate::domain::timeleap::ConfirmedTargetSearchRequest {
            seed,
            confirmed_frame1,
            timeline_generation: self.confirmed_target_timeline_generation,
            source_config: Some(source_config),
            start: fallback_start,
            baseline_target_frame,
            generation_start_frame,
            drift_search_generation_start_frame,
            drift_range_ticks,
            grace_min_frames,
            consider_delay,
        };
        let form = &mut self.confirmed_target_search_form;
        form.generation = form.generation.wrapping_add(1);
        if !matches!(form.status, ConfirmedTargetSearchStatus::Running) {
            form.previous_status = Some(Arc::new(std::mem::take(&mut form.status)));
        }
        form.status = ConfirmedTargetSearchStatus::Running;
        form.request = Some(request.clone());
        Ok((form.generation, request, cache))
    }

    pub(crate) fn apply_confirmed_target_search_result(
        &mut self,
        generation: u64,
        request: &crate::domain::timeleap::ConfirmedTargetSearchRequest,
        result: Result<
            crate::domain::timeleap::ConfirmedTargetSearchResult,
            crate::domain::timeleap::TimeleapSearchFailure,
        >,
    ) -> bool {
        if self.confirmed_target_search_form.generation != generation
            || self.confirmed_target_search_form.request.as_ref() != Some(request)
            || request.timeline_generation != self.confirmed_target_timeline_generation
        {
            return false;
        }
        if matches!(
            &result,
            Err(crate::domain::timeleap::TimeleapSearchFailure::Cancelled)
        ) {
            self.confirmed_target_search_form.request = None;
            self.confirmed_target_search_form.status = self
                .confirmed_target_search_form
                .previous_status
                .take()
                .map(restore_confirmed_target_search_status)
                .unwrap_or_default();
            return false;
        }
        self.confirmed_target_search_form.request = None;
        self.confirmed_target_search_form.previous_status = None;
        self.confirmed_target_search_form
            .result_generation_start_frame = Some(request.generation_start_frame);
        self.confirmed_target_search_form
            .result_drift_search_generation_start_frame =
            request.drift_search_generation_start_frame;
        self.confirmed_target_search_form
            .result_drift_marker_boundary = None;
        let unique_target = match result {
            Ok(result) if result.rows.is_empty() => {
                self.confirmed_target_search_form
                    .result_drift_marker_boundary = None;
                self.confirmed_target_search_form.status = ConfirmedTargetSearchStatus::NoResults;
                None
            }
            Ok(result) => {
                self.confirmed_target_search_form
                    .result_drift_marker_boundary = result.drift_marker_boundary;
                let unique_target = if result.rows.len() != 1
                    || result.rows[0].target_frame == request.baseline_target_frame
                {
                    None
                } else {
                    Some(result.rows[0].target_frame)
                };
                self.confirmed_target_search_form.status =
                    ConfirmedTargetSearchStatus::Results(result.rows);
                unique_target
            }
            Err(crate::domain::timeleap::TimeleapSearchFailure::Cancelled) => {
                unreachable!("cancelled requests return before changing the displayed result")
            }
            Err(error) => {
                self.confirmed_target_search_form
                    .result_drift_marker_boundary = None;
                self.confirmed_target_search_form.status =
                    confirmed_target_search_failure_status(error)
                        .expect("cancelled requests return before applying their result");
                None
            }
        };
        if let Some(target_frame) = unique_target {
            self.target_consumption = target_frame.to_string();
            self.target_dirty = true;
            if self.commit_target_input().is_ok() {
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    pub(crate) fn set_confirmed_target_frame_from_search_row(
        &mut self,
        row: &crate::domain::timeleap::ConfirmedTargetSearchRow,
    ) -> bool {
        if !self.can_set_confirmed_target_frame_from_search() {
            return false;
        }
        let ConfirmedTargetSearchStatus::Results(rows) = &self.confirmed_target_search_form.status
        else {
            return false;
        };
        if !rows.contains(row)
            || !(0..=crate::domain::rng::MAX_SFMT_FRAME).contains(&row.target_frame)
        {
            return false;
        }
        self.target_consumption = row.target_frame.to_string();
        self.target_dirty = true;
        self.commit_target_input().is_ok()
    }

    pub(crate) fn can_set_confirmed_target_frame_from_search(&self) -> bool {
        !self.is_correction_mode()
    }

    /// Signed Timeline drift is converted to an updated Field offset using the
    /// same add/subtract direction as the main correction flow.
    pub(crate) fn apply_timeleap_offset_drift(
        &mut self,
        drift_frames: i64,
        _fastest_close: bool,
    ) -> Option<i64> {
        let current = self.encounter_offset.trim().parse::<i64>().ok()?;
        let corrected = timeleap_offset_after_drift(current, drift_frames)?;
        self.use_offset = true;
        self.encounter_offset = corrected.to_string();
        Some(corrected)
    }

    pub(crate) fn preview_timeleap_offset_drift(
        &self,
        drift_frames: i64,
        _fastest_close: bool,
    ) -> Option<i64> {
        let current = self.encounter_offset.trim().parse::<i64>().ok()?;
        timeleap_offset_after_drift(current, drift_frames)
    }

    pub(crate) fn apply_timeleap_frame1_offset(
        &mut self,
        candidate_frame1: i64,
        fastest_close: bool,
    ) -> Option<i64> {
        let drift_frames = self.timeleap_frame1_offset_drift(candidate_frame1)?;
        self.apply_timeleap_offset_drift(drift_frames, fastest_close)
    }

    pub(crate) fn preview_timeleap_frame1_offset(
        &self,
        candidate_frame1: i64,
        fastest_close: bool,
    ) -> Option<i64> {
        let drift_frames = self.timeleap_frame1_offset_drift(candidate_frame1)?;
        self.preview_timeleap_offset_drift(drift_frames, fastest_close)
    }

    pub(super) fn timeleap_frame1_offset_drift(&self, candidate_frame1: i64) -> Option<i64> {
        let selected_frame1 = self.timeleap_frame1_input.trim().parse::<i64>().ok()?;
        let tick_for_frame = |frame| {
            let mut matching_ticks = self
                .timeleap_candidate_groups
                .iter()
                .flat_map(|group| group.candidates.iter())
                .filter(|candidate| candidate.frame1 == frame)
                .map(|candidate| candidate.frame1_tick);
            let tick = matching_ticks.next()?;
            matching_ticks.all(|other| other == tick).then_some(tick)
        };
        let candidate_tick = tick_for_frame(candidate_frame1)?;
        let selected_tick = tick_for_frame(selected_frame1)?;
        timeleap_frame1_offset_drift(candidate_tick, selected_tick)
    }

    pub(super) fn timeleap_courier_validation_for(courier: &CourierConfig) -> Result<(), String> {
        if courier.pid_type == 3 && courier.pid == 0 {
            return Err("配達員の固定PIDを確認してください。".into());
        }
        if courier.iv_count as usize
            > 6usize.saturating_sub(
                courier
                    .fixed_ivs
                    .iter()
                    .filter(|value| **value >= 0)
                    .count(),
            )
        {
            return Err("配達員の確定IV数とV個体数を確認してください。".into());
        }
        if courier
            .fixed_ivs
            .iter()
            .any(|value| *value < -1 || *value > 31)
        {
            return Err("配達員の固定IVは-1から31で指定してください。".into());
        }
        Ok(())
    }

    pub(super) fn timeleap_courier_config(&self) -> Result<CourierConfig, String> {
        self.timeleap_courier_validation()?;
        Ok(CourierConfig {
            no_dex: self.timeleap_no_dex && self.timeleap_uses_courier_method(),
            your_id: self.timeleap_courier_your_id,
            is_egg: self.timeleap_courier_is_egg,
            iv_count: self
                .timeleap_courier_iv_count
                .trim()
                .parse()
                .map_err(|_| "配達員のV個体数を確認してください。".to_owned())?,
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
        })
    }

    pub(super) fn timeleap_consider_delay_value(&self) -> Result<i64, String> {
        let value = self
            .timeleap_consider_delay_frames
            .trim()
            .parse::<i64>()
            .map_err(|_| "Consider Delayは0〜9998Fで入力してください。".to_string())?;
        if (0..=9998).contains(&value) {
            return Ok(value);
        }
        let selected_source_value = crate::domain::fixed_rng_catalog::selected_preset_for(
            self.timeleap_game_edition,
            self.timeleap_search_category,
            self.timeleap_search_preset,
        )
        .map(|preset| i64::from(preset.delay_frames));
        if selected_source_value == Some(value) {
            return Ok(value);
        }
        Err("Consider Delayは0〜9998Fで入力してください。".into())
    }
}

fn restore_confirmed_target_search_status(
    previous: Arc<ConfirmedTargetSearchStatus>,
) -> ConfirmedTargetSearchStatus {
    Arc::try_unwrap(previous).unwrap_or_else(|shared| (*shared).clone())
}

pub(super) fn timeleap_consider_delay_config(
    edition: crate::domain::fixed_rng_catalog::GameEdition,
    category: usize,
    preset: usize,
    enabled: bool,
    delay_frames: i64,
    courier: &crate::domain::timeleap::CourierConfig,
) -> Option<crate::domain::timeleap::ConsiderDelayConfig> {
    if !enabled {
        return None;
    }

    let generator = if category == crate::domain::fixed_rng_catalog::categories(edition).len() {
        crate::domain::timeleap::ConsiderDelayGenerator::Event
    } else {
        let delay_type =
            crate::domain::fixed_rng_catalog::selected_preset_for(edition, category, preset)
                .map(|preset| preset.delay_type)
                .unwrap_or_default();
        crate::domain::timeleap::ConsiderDelayGenerator::FixedSymbol(delay_type)
    };
    Some(crate::domain::timeleap::ConsiderDelayConfig {
        delay_frames,
        generator,
        courier: courier.clone(),
    })
}
