use super::*;

#[test]
fn timeleap_frame1_deadline_is_independent_of_the_selected_timer() {
    let now = Instant::now();
    let mut state = FieldState {
        timeleap_enabled: true,
        timeleap_locked: true,
        timeleap_stage: TimeleapStage::Frame1Waiting,
        timeleap_frame1_deadline: Some(now - Duration::from_millis(20)),
        timer_kind: FieldTimerKind::NextBlink,
        timer_start: Some(now),
        timer_seconds: 4.0,
        ..FieldState::default()
    };

    assert!(state.target_timer_available(59.8621));
    assert!(state.tick_timeleap_frame1(now + Duration::from_millis(30)));
    assert_eq!(state.timeleap_stage, TimeleapStage::Assumed);
    assert_eq!(state.timer_kind, FieldTimerKind::NextBlink);
    assert_eq!(state.timer_seconds_value(), 0.0);
    assert!(!state.timer_is_running());
    assert!(state.timeleap_assumed());
    assert!(!state.timeleap_waiting_for_blink_observation());
    assert_eq!(state.timeleap_frame1_remaining(59.8621), Some(0.0));
    assert!(state.toggle_timer_kind(59.8621).is_err());
}

#[test]
fn direct_encounter_keeps_target_selected_while_target_is_recomputed() {
    let anchor = Instant::now();
    let status = TimeleapStatus {
        model: ModelStatus::new(1),
        phase: 0,
        fidget_cd: 1,
        is_boy: true,
        raining: false,
    };
    let candidate = TimeleapCandidate {
        frame1: 200,
        frame1_tick: 100,
        post_tick: 101,
        grace_frames: 0,
        pre_frame: 200,
        pre_sfmt_position: 200,
        pre_status: status.clone(),
        post_frame: 202,
        post_sfmt_position: 203,
        post_status: status,
        target_inputs: Vec::new().into(),
    };
    let mut state = FieldState {
        timeleap_enabled: true,
        timeleap_locked: true,
        timeleap_direct_encounter: true,
        timeleap_stage: TimeleapStage::Assumed,
        timeleap_selected_frame: Some(200),
        timeleap_execution_candidates: vec![candidate],
        results: vec![FieldResult {
            start_consumption: 150,
            first_wait_frame: 0,
            last_wait_frame: 0,
            first_consumption: 150,
            last_consumption: 150,
            intervals: Vec::new(),
            target_consumptions: Vec::new(),
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
        ..FieldState::default()
    };

    assert!(state.adopt_assumed_timeleap_timeline(anchor));
    assert_eq!(state.timer_kind, FieldTimerKind::Target);
    state.prepare_target_research(crate::domain::timing::DEFAULT_FPS);
    state.normalize_timer_kind(crate::domain::timing::DEFAULT_FPS);
    assert_eq!(state.timer_kind, FieldTimerKind::Target);
    assert!(state.target_research_pending);
    assert!(!state.timer_is_running());
}

#[test]
fn frame1_deadline_offset_and_transition_time_are_applied_once_after_provisional_adoption() {
    let fps = crate::domain::timing::DEFAULT_FPS;
    let nominal_deadline = Instant::now() + Duration::from_secs(2);
    let adjust = 20;
    let deadline = FieldState::apply_frame1_adjustment(nominal_deadline, adjust, fps);
    let adopted_at = deadline;
    let status = TimeleapStatus {
        model: ModelStatus::new(2),
        phase: 1,
        fidget_cd: 2,
        is_boy: true,
        raining: false,
    };
    let candidate = TimeleapCandidate {
        frame1: 200,
        frame1_tick: 100,
        post_tick: 101,
        grace_frames: 0,
        pre_frame: 200,
        pre_sfmt_position: 200,
        pre_status: status.clone(),
        post_frame: 202,
        post_sfmt_position: 203,
        post_status: status,
        target_inputs: Vec::new().into(),
    };
    let mut state = FieldState {
        timeleap_enabled: true,
        timeleap_locked: true,
        timeleap_stage: TimeleapStage::Frame1Waiting,
        adjust,
        timeleap_selected_frame: Some(200),
        timeleap_execution_candidates: vec![candidate],
        timeleap_frame1_deadline: Some(deadline),
        timeleap_frame1_reachable: Some(true),
        results: vec![FieldResult {
            start_consumption: 150,
            first_wait_frame: 0,
            last_wait_frame: 0,
            first_consumption: 150,
            last_consumption: 150,
            intervals: Vec::new(),
            target_consumptions: Vec::new(),
            next_blink_wait_frame: 0,
            target_wait_frame: None,
            target_exact: None,
            target_frame_before: None,
            target_frame_after: None,
            observed_model: 0,
            models: 2,
            timeline: Vec::new(),
            candidate_blinks: Vec::new(),
            target_blink_count: None,
            target_blink_count_offset_from_first_row: 0,
            timeline_cursor: None,
            target_cursor: None,
        }],
        ..FieldState::default()
    };

    assert!(state.tick_timeleap_frame1(deadline));
    assert!(state.adopt_assumed_timeleap_timeline(adopted_at));

    assert!(state.timeleap_timeline_adopted());
    assert!(!state.timeleap_succeeded());
    assert_eq!(state.timeleap_confirmed_frame1, None);
    assert_eq!(state.timeleap_confirmed_candidate, None);
    assert_eq!(
        state.timeleap_frame1_indicator(),
        Some(TimeleapFrame1Indicator::Reachable)
    );
    assert_eq!(state.timeleap_timeline_anchor, Some(adopted_at));
    assert_eq!(state.timeleap_frame1_adjustment_consumed, Some(adjust));
    assert_eq!(
        state.adjust, adjust,
        "adoption must not reset the input correction"
    );
    let post_adoption_remaining = state
        .remaining_from_observation_at(120, fps, adopted_at)
        .expect("post-Timeleap timer remains available");
    let expected_remaining = crate::domain::timing::game_frames_to_seconds(122.0, fps);
    assert!((post_adoption_remaining - expected_remaining).abs() < f64::EPSILON * 8.0);
    assert_eq!(state.timeleap_assumed_transition_frames, 2);
    state.blink_wait_queue.push_back(120);
    state
        .start_blink_timer_from_queue(fps)
        .expect("provisional adoption starts its first post-Timeleap blink timer");
    assert!(
            (state.timer_seconds_value() - expected_remaining).abs() < 0.1,
            "Frame1 already consumed the 20F Offset; initial Target timer must not apply it again, got {}s",
            state.timer_seconds_value()
        );
    state.adjust = 25;
    let changed_adjust_remaining = state
        .remaining_from_observation_at(120, fps, adopted_at)
        .expect("post-adoption adjustment delta remains usable");
    let expected_after_adjust_change = crate::domain::timing::game_frames_to_seconds(117.0, fps);
    assert!((changed_adjust_remaining - expected_after_adjust_change).abs() < f64::EPSILON * 8.0);
    let cursor = state.results[0]
        .timeline_cursor
        .as_ref()
        .expect("provisional cursor is available to the normal timeline");
    assert_eq!(cursor.tick, 101);
    assert_eq!(cursor.stream_cursor, 203);
    assert_eq!(
        state.target_reachability_for_display(Some(true)),
        Some(true)
    );
    assert!(!state.timeleap_waiting_for_blink_observation());
}

#[test]
fn provisional_timer_accounts_for_each_timeleap_modes_transition_tick_count() {
    let fps = crate::domain::timing::DEFAULT_FPS;
    let anchor = Instant::now();
    let mut state = FieldState {
        timeleap_timeline_anchor: Some(anchor),
        timeleap_frame1_adjustment_consumed: Some(0),
        ..FieldState::default()
    };

    for (mode, transition_ticks) in [("menu close", 0), ("dialogue", 3), ("courier", 20)] {
        state.timeleap_assumed_transition_frames = transition_ticks * 2;
        let expected_frames = 120 + transition_ticks * 2;
        let remaining = state
            .remaining_from_observation_at(120, fps, anchor)
            .expect("post-Timeleap timer remains available");
        assert!(
            (remaining
                - crate::domain::timing::game_frames_to_seconds(expected_frames as f64, fps,))
            .abs()
                < 1e-9,
            "{mode} must include only its logical transition ticks"
        );

        let elapsed_one_frame =
            Duration::from_secs_f64(crate::domain::timing::game_frames_to_seconds(1.0, fps));
        let after_elapsed = state
            .remaining_from_observation_at(120, fps, anchor + elapsed_one_frame)
            .expect("timer remains available during the transition");
        assert!(
            (after_elapsed
                - crate::domain::timing::game_frames_to_seconds(
                    expected_frames.saturating_sub(1) as f64,
                    fps,
                ))
            .abs()
                < 1e-9,
            "{mode} timer must count down from the Frame1 anchor"
        );
    }
}

#[test]
fn provisional_adoption_carries_negative_adjustment_and_applies_only_later_delta() {
    let fps = crate::domain::timing::DEFAULT_FPS;
    let anchor = Instant::now();
    let mut state = FieldState {
        timeleap_enabled: true,
        timeleap_frame1_adjustment_consumed: Some(-20),
        timeleap_timeline_anchor: Some(anchor),
        adjust: -20,
        ..FieldState::default()
    };

    let unchanged = state
        .remaining_from_observation_at(120, fps, anchor)
        .expect("already-consumed negative adjustment is not repeated");
    assert!((unchanged - crate::domain::timing::game_frames_to_seconds(120.0, fps)).abs() < 1e-9);

    state.adjust = -25;
    let changed = state
        .remaining_from_observation_at(120, fps, anchor)
        .expect("later negative adjustment delta remains active");
    let expected = crate::domain::timing::game_frames_to_seconds(125.0, fps);
    assert!((changed - expected).abs() < 1e-9);
}

#[test]
fn shift_confirmed_adoption_keeps_the_measured_time_as_its_anchor() {
    let fps = crate::domain::timing::DEFAULT_FPS;
    let measured_shift_at = Instant::now();
    let status = TimeleapStatus {
        model: ModelStatus::new(1),
        phase: 0,
        fidget_cd: -1,
        is_boy: true,
        raining: false,
    };
    let candidate = TimeleapCandidate {
        frame1: 200,
        frame1_tick: 100,
        post_tick: 101,
        grace_frames: 0,
        pre_frame: 200,
        pre_sfmt_position: 200,
        pre_status: status.clone(),
        post_frame: 202,
        post_sfmt_position: 201,
        post_status: status,
        target_inputs: Vec::new().into(),
    };
    let mut state = FieldState {
        timeleap_enabled: true,
        timeleap_direct_encounter: true,
        timeleap_stage: TimeleapStage::Succeeded,
        timeleap_confirmed_candidate: Some(candidate),
        timeleap_shift_times: vec![measured_shift_at],
        adjust: 20,
        results: vec![FieldResult {
            start_consumption: 150,
            first_wait_frame: 0,
            last_wait_frame: 0,
            first_consumption: 150,
            last_consumption: 150,
            intervals: Vec::new(),
            target_consumptions: Vec::new(),
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
        ..FieldState::default()
    };

    assert!(state.adopt_timeleap_timeline());

    assert_eq!(state.timer_kind, FieldTimerKind::Target);
    state.prepare_target_research(fps);
    state.normalize_timer_kind(fps);
    assert_eq!(state.timer_kind, FieldTimerKind::Target);
    assert!(state.target_research_pending);
    assert!(!state.timer_is_running());
    assert_eq!(state.timeleap_timeline_anchor, Some(measured_shift_at));
    assert_eq!(state.timeleap_frame1_adjustment_consumed, None);
    assert_eq!(state.timeleap_assumed_transition_frames, 0);
    let remaining = state
        .remaining_from_observation_at(120, fps, measured_shift_at)
        .expect("measured post-Timeleap countdown remains available");
    let expected = crate::domain::timing::game_frames_to_seconds(100.0, fps);
    assert!((remaining - expected).abs() < f64::EPSILON * 8.0);
}

#[test]
fn rechecking_assumed_timeline_keeps_all_frame1_branches_and_deadline_origin() {
    let deadline = Instant::now() - Duration::from_secs(3);
    let make_candidate = |frame1| TimeleapCandidate {
        frame1,
        frame1_tick: frame1 / 2,
        post_tick: frame1 / 2 + 1,
        grace_frames: 0,
        pre_frame: frame1,
        pre_sfmt_position: frame1,
        pre_status: TimeleapStatus {
            model: ModelStatus::new(1),
            phase: 0,
            fidget_cd: -1,
            is_boy: true,
            raining: false,
        },
        post_frame: frame1 + 2,
        post_sfmt_position: frame1 + 1,
        post_status: TimeleapStatus {
            model: ModelStatus::new(1),
            phase: 0,
            fidget_cd: -1,
            is_boy: true,
            raining: false,
        },
        target_inputs: Vec::new().into(),
    };
    let candidates = vec![
        make_candidate(198),
        make_candidate(200),
        make_candidate(202),
    ];
    let mut state = FieldState {
        timeleap_enabled: true,
        timeleap_locked: true,
        timeleap_stage: TimeleapStage::Assumed,
        timeleap_nominal_tick: Some(100),
        timeleap_execution_candidates: candidates.clone(),
        timeleap_frame1_deadline: Some(deadline),
        timeleap_wait_started: Some(deadline),
        timeleap_observation_anchor_frame: Some(200),
        timeleap_timeline_anchor: Some(deadline),
        timeleap_frame1_reachable: Some(true),
        timeleap_shift_times: vec![deadline + Duration::from_secs(1)],
        timer_start: Some(deadline),
        ..FieldState::default()
    };

    state
        .begin_timeleap_recheck_at(false, 59.8621, Instant::now())
        .expect("provisional branch can be rechecked without re-running Frame1");

    assert_eq!(state.timeleap_stage, TimeleapStage::ShiftWaiting);
    assert_eq!(state.timeleap_execution_candidates, candidates);
    assert_eq!(state.timeleap_wait_started, Some(deadline));
    assert_eq!(state.timeleap_observation_anchor_frame, Some(200));
    assert!(state.timeleap_shift_times.is_empty());
    assert!(state.timer_start.is_none());
    assert_eq!(
        state.timeleap_frame1_indicator(),
        Some(TimeleapFrame1Indicator::Reachable)
    );
}

#[test]
fn frame1_adoption_switches_to_recheck_radius_and_preserves_origin_through_recheck() {
    let deadline = Instant::now() - Duration::from_secs(1);
    let status = TimeleapStatus {
        model: ModelStatus::new(1),
        phase: 0,
        fidget_cd: -1,
        is_boy: true,
        raining: false,
    };
    let candidate = TimeleapCandidate {
        frame1: 51_337,
        frame1_tick: 25_668,
        post_tick: 25_669,
        grace_frames: 0,
        pre_frame: 51_337,
        pre_sfmt_position: 51_337,
        pre_status: status.clone(),
        post_frame: 51_339,
        post_sfmt_position: 71_134,
        post_status: status.clone(),
        target_inputs: Vec::new().into(),
    };
    let mut state = FieldState {
        timeleap_frame1_input: "51337".into(),
        seed: "12345678".into(),
        timeleap_enabled: true,
        timeleap_locked: true,
        timeleap_stage: TimeleapStage::Frame1Waiting,
        timeleap_selected_frame: Some(51_337),
        timeleap_nominal_tick: Some(25_668),
        timeleap_execution_candidates: vec![candidate.clone()],
        timeleap_frame1_deadline: Some(deadline),
        timeleap_frame1_reachable: Some(true),
        timeleap_observation_anchor_frame: Some(51_337),
        timeleap_source_inputs_initialized: true,
        timeleap_source_starting_frame: "497".into(),
        timeleap_source_npc_count: "0".into(),
        timeleap_source_frame1: "50000".into(),
        tolerance: "15".into(),
        results: vec![FieldResult {
            start_consumption: 497,
            first_wait_frame: 0,
            last_wait_frame: 0,
            first_consumption: 497,
            last_consumption: 497,
            intervals: Vec::new(),
            target_consumptions: Vec::new(),
            next_blink_wait_frame: 0,
            target_wait_frame: None,
            target_exact: None,
            target_frame_before: None,
            target_frame_after: None,
            observed_model: 0,
            models: 1,
            timeline: vec![FieldTimelineRow {
                frame: 497,
                timeline_frame: 0,
                duration_frames: 0,
            }],
            candidate_blinks: Vec::new(),
            target_blink_count: None,
            target_blink_count_offset_from_first_row: 0,
            timeline_cursor: None,
            target_cursor: None,
        }],
        ..FieldState::default()
    };

    assert!(!state.timeleap_recheck_position_radius_active());
    assert_eq!(state.tolerance, "15");
    assert!(state.tick_timeleap_frame1(deadline));
    assert!(state.adopt_assumed_timeleap_timeline(deadline));

    // Switch immediately on Frame1 adoption, without waiting for the recheck button.
    assert!(state.timeleap_recheck_position_radius_active());
    assert_eq!(state.timeleap_confirmed_frame1, None);
    assert_eq!(
        state.timeleap_source_frame1, "51337",
        "provisional Frame1 mirrors the valid main input like Reset without becoming confirmed"
    );
    state.reset_timeleap_source_inputs();
    assert_eq!(state.timeleap_source_frame1, "51337");
    assert_eq!(state.timeleap_confirmed_frame1, None);
    assert_eq!(state.tolerance, "100");
    assert_eq!(state.tolerance_for_persistence(), "15");
    assert_eq!(state.timeleap_original_starting_frame, Some(497));
    assert_eq!(state.results[0].start_consumption, 71_134);
    assert_eq!(
        state.starting_frame_for_display(&state.results[0]),
        Some(497)
    );
    let context = eframe::egui::Context::default();
    let texts = crate::ui::i18n::Texts::new(crate::ui::i18n::Language::Japanese);
    let render_field = |state: &mut FieldState| {
        context.run(
            eframe::egui::RawInput {
                screen_rect: Some(eframe::egui::Rect::from_min_size(
                    eframe::egui::Pos2::ZERO,
                    eframe::egui::vec2(1_000.0, 700.0),
                )),
                ..eframe::egui::RawInput::default()
            },
            |context| {
                eframe::egui::CentralPanel::default().show(context, |ui| {
                    crate::ui::field_view::field_ui(
                        ui,
                        state,
                        crate::ui::field_view::FieldUiWorkerState::default(),
                        context,
                        crate::domain::timing::DEFAULT_FPS,
                        false,
                        true,
                        crate::ui::widgets::ColorScheme::Standard,
                        &texts,
                        false,
                    );
                });
            },
        )
    };
    let output = render_field(&mut state);
    assert!(output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        eframe::egui::Shape::Text(text) if text.galley.job.text == "予測現在地 ±"
    )));

    // The user can change the radius after Frame1, and it stays active across
    // recheck failure, retry, and successful adoption.
    state.tolerance = "120".into();
    state
        .begin_timeleap_recheck_at(
            false,
            crate::domain::timing::DEFAULT_FPS,
            deadline + Duration::from_secs(1),
        )
        .expect("assumed Timeleap can enter blink recheck");
    assert!(state.timeleap_recheck_position_radius_active());
    assert_eq!(state.tolerance, "120");
    assert_eq!(state.tolerance_for_persistence(), "15");

    let generation = state.timeleap_shift_resolution_generation;
    state.timeleap_shift_resolution_pending = true;
    assert_eq!(
        state.apply_timeleap_shift_resolution(
            generation,
            TimeleapShiftResolutionSummary::default(),
        ),
        Some(TimeleapStage::Failed)
    );
    assert_eq!(state.tolerance, "120");
    assert!(state.timeleap_recheck_position_radius_active());

    assert!(state.timeleap_retry(crate::domain::timing::DEFAULT_FPS));
    assert_eq!(state.tolerance, "120");
    assert!(state.timeleap_recheck_position_radius_active());

    state
        .timeleap_shift_times
        .push(deadline + Duration::from_secs(2));
    state.timeleap_shift_resolution_pending = true;
    let generation = state.timeleap_shift_resolution_generation;
    assert_eq!(
        state.apply_timeleap_shift_resolution(
            generation,
            TimeleapShiftResolutionSummary {
                matches_found: true,
                multiple_future_states: false,
                confirmed_frame1: Some(51_337),
                confirmed_candidate: Some(candidate),
            },
        ),
        Some(TimeleapStage::Succeeded)
    );
    assert!(state.adopt_timeleap_timeline());
    assert_eq!(state.timeleap_original_starting_frame, Some(497));
    assert_eq!(state.results[0].start_consumption, 71_134);
    assert_eq!(
        state.starting_frame_for_display(&state.results[0]),
        Some(497)
    );
    assert_eq!(
        state.timeleap_lifecycle_policy().summary_frame1,
        Some(51_337)
    );
    assert_eq!(
        state.timeleap_source_frame1, "51337",
        "the uniquely confirmed actual Frame1 replaces the search source immediately"
    );
    state.reset_timeleap_source_inputs();
    assert_eq!(
        state.timeleap_source_frame1, "51337",
        "Reset prefers the confirmed actual Frame1 over the editable main input"
    );

    let extension = state.timeline_extension_result_for_test(Some(71_134));
    assert_eq!(state.apply_timeline_extension(extension), Some(true));
    assert_eq!(state.timeleap_safe_frame_display, Some(71_134));
    assert_eq!(
        state.starting_frame_for_display(&state.results[0]),
        Some(497)
    );
    assert_eq!(state.timeleap_source_starting_frame, "497");
    assert_eq!(state.timeleap_source_frame1, "51337");
    assert_eq!(state.tolerance, "120");
    assert_eq!(state.tolerance_for_persistence(), "15");
    let output = render_field(&mut state);
    assert!(output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        eframe::egui::Shape::Text(text) if text.galley.job.text == texts.starting_frame(497)
    )));
    assert!(output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        eframe::egui::Shape::Text(text) if text.galley.job.text == "Frame1: 51337"
    )));
    assert!(!output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        eframe::egui::Shape::Text(text) if text.galley.job.text == texts.starting_frame(71_134)
    )));

    state.clear_timeleap_candidates();
    assert_eq!(state.tolerance, "120");
    assert!(state.timeleap_recheck_position_radius_active());

    state.cancel_observation();
    assert_eq!(state.tolerance, "15");
    assert!(!state.timeleap_recheck_position_radius_active());
}

#[test]
fn provisional_frame1_initializes_search_source_when_window_opens_later() {
    let anchor = Instant::now();
    let status = TimeleapStatus {
        model: ModelStatus::new(1),
        phase: 0,
        fidget_cd: -1,
        is_boy: true,
        raining: false,
    };
    let candidate = TimeleapCandidate {
        frame1: 200,
        frame1_tick: 100,
        post_tick: 101,
        grace_frames: 0,
        pre_frame: 200,
        pre_sfmt_position: 200,
        pre_status: status.clone(),
        post_frame: 202,
        post_sfmt_position: 201,
        post_status: status,
        target_inputs: Vec::new().into(),
    };
    let mut state = FieldState {
        timeleap_frame1_input: "200".into(),
        timeleap_enabled: true,
        timeleap_locked: true,
        timeleap_stage: TimeleapStage::Assumed,
        timeleap_selected_frame: Some(200),
        timeleap_execution_candidates: vec![candidate],
        results: vec![FieldResult {
            start_consumption: 150,
            first_wait_frame: 0,
            last_wait_frame: 0,
            first_consumption: 150,
            last_consumption: 150,
            intervals: Vec::new(),
            target_consumptions: Vec::new(),
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
        ..FieldState::default()
    };

    assert!(state.adopt_assumed_timeleap_timeline(anchor));
    assert!(!state.timeleap_source_inputs_initialized);
    assert_eq!(state.timeleap_confirmed_frame1, None);

    state.initialize_timeleap_source_inputs();

    assert_eq!(state.timeleap_source_frame1, "200");
    assert_eq!(state.timeleap_confirmed_frame1, None);
}

#[test]
fn frame1_timer_can_be_selected_while_its_preview_is_pending() {
    let now = Instant::now();
    let mut state = FieldState {
        seed: "12345678".into(),
        timeleap_enabled: true,
        timeleap_frame1_input: "200".into(),
        timeleap_frame1_preview_state: TimeleapFrame1PreviewState::Pending,
        observed: vec![now],
        timer_kind: FieldTimerKind::NextBlink,
        results: vec![FieldResult {
            start_consumption: 100,
            first_wait_frame: 2,
            last_wait_frame: 100,
            first_consumption: 101,
            last_consumption: 150,
            intervals: vec![98],
            target_consumptions: Vec::new(),
            next_blink_wait_frame: 2,
            target_wait_frame: Some(200),
            target_exact: Some(true),
            target_frame_before: Some(200),
            target_frame_after: Some(200),
            observed_model: 0,
            models: 2,
            timeline: Vec::new(),
            candidate_blinks: Vec::new(),
            target_blink_count: Some(1),
            target_blink_count_offset_from_first_row: 0,
            timeline_cursor: None,
            target_cursor: None,
        }],
        ..FieldState::default()
    };
    let seed = 0x1234_5678;
    state.set_observation_cache(
        crate::domain::rng::generate_observation_cache(
            seed,
            0..=1_000,
            1_000,
            crate::domain::timing::DEFAULT_FPS,
        )
        .expect("generate test cache for the live SFMT position"),
    );
    state.results[0].timeline_cursor = Some(TimelineCursor {
        status: ModelStatus::new(2),
        stream_cursor: 150,
        consumption: 150,
        tick: 50,
        timeleap_runtime: None,
    });
    state
        .prepare_shared_timeline_cache()
        .expect("prepare the confirmed Timeline cache");

    assert!(state.target_timer_available(59.8621));
    state.toggle_timer_kind(59.8621).unwrap();
    assert_eq!(state.timer_kind, FieldTimerKind::Target);
    assert!(state.timer_start.is_none());
}

#[test]
fn field_offset_only_shifts_target_wait_frame() {
    let mut state = FieldState::default();
    state.use_offset = true;
    state.encounter_offset = "+2".into();
    let result = FieldResult {
        start_consumption: 0,
        first_wait_frame: 2,
        last_wait_frame: 4,
        first_consumption: 1,
        last_consumption: 2,
        intervals: vec![2],
        target_consumptions: Vec::new(),
        next_blink_wait_frame: 2,
        target_wait_frame: Some(100),
        target_exact: Some(true),
        target_frame_before: Some(50),
        target_frame_after: Some(50),
        observed_model: 0,
        models: 2,
        timeline: Vec::new(),
        candidate_blinks: Vec::new(),
        target_blink_count: Some(1),
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: None,
    };
    assert_eq!(state.target_wait_frame_with_offset(&result), Some(102));
    state.encounter_offset = "-2".into();
    assert_eq!(state.target_wait_frame_with_offset(&result), Some(98));
    state.encounter_offset = (MAX_SFMT_FRAME + 2).to_string();
    assert_eq!(state.target_wait_frame_with_offset(&result), None);
    state.encounter_offset.clear();
    assert_eq!(state.target_wait_frame_with_offset(&result), Some(100));
    state.use_offset = false;
    assert_eq!(state.target_wait_frame_with_offset(&result), Some(100));
}

#[test]
fn consider_delay_moves_only_the_post_timeleap_target_timer_to_a_input() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_consider_delay_enabled = true;
    state.timeleap_consider_delay_frames = "10".into();
    let result = FieldResult {
        start_consumption: 0,
        first_wait_frame: 2,
        last_wait_frame: 4,
        first_consumption: 1,
        last_consumption: 2,
        intervals: vec![2],
        target_consumptions: Vec::new(),
        next_blink_wait_frame: 2,
        target_wait_frame: Some(100),
        target_exact: Some(true),
        target_frame_before: Some(50),
        target_frame_after: Some(50),
        observed_model: 0,
        models: 2,
        timeline: Vec::new(),
        candidate_blinks: Vec::new(),
        target_blink_count: Some(1),
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: None,
    };

    // Before the Timeleap Timeline is adopted, the Target timer is the Frame1 timer.
    assert_eq!(state.target_timer_wait_frame(&result), Some(100));

    // Consider Delay is confined to search; the main timer uses Target directly.
    state.timeleap_timeline_anchor = Some(Instant::now());
    assert_eq!(state.target_wait_frame_with_offset(&result), Some(100));
    assert_eq!(state.target_timer_wait_frame(&result), Some(100));

    state.timeleap_consider_delay_enabled = false;
    assert_eq!(state.target_timer_wait_frame(&result), Some(100));
}

#[test]
fn adopted_timeleap_target_timer_uses_target_frame_directly() {
    let fps = crate::domain::timing::DEFAULT_FPS;
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_consider_delay_enabled = true;
    state.timeleap_consider_delay_frames = "10".into();
    state.timeleap_timeline_anchor = Some(Instant::now());
    // A distant Frame1 deadline must not be reused after the Timeleap Timeline is adopted.
    state.timeleap_frame1_deadline = Some(Instant::now() + Duration::from_secs(60));
    state.results.push(FieldResult {
        start_consumption: 0,
        first_wait_frame: 0,
        last_wait_frame: 0,
        first_consumption: 0,
        last_consumption: 0,
        intervals: Vec::new(),
        target_consumptions: Vec::new(),
        next_blink_wait_frame: 0,
        target_wait_frame: Some(100),
        target_exact: Some(true),
        target_frame_before: Some(100),
        target_frame_after: Some(100),
        observed_model: 0,
        models: 1,
        timeline: Vec::new(),
        candidate_blinks: Vec::new(),
        target_blink_count: Some(0),
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: None,
    });

    state.start_target_timer(fps).unwrap();

    assert_eq!(state.timer_kind, FieldTimerKind::Target);
    assert!(state.timer_start.is_some());
    assert!((state.timer_seconds - game_frames_to_seconds(100.0, fps)).abs() < 0.01);
}

#[test]
fn completed_target_timer_stays_at_zero_until_reset() {
    let mut state = FieldState::default();
    state.timer_kind = FieldTimerKind::Target;
    state.timer_finished = true;
    assert_eq!(
        state.preview_remaining(FieldTimerKind::Target, 59.8621),
        Some(0.0)
    );
    state.cancel_observation();
    assert_eq!(
        state.preview_remaining(FieldTimerKind::Target, 59.8621),
        None
    );
}

#[test]
fn field_target_timer_and_current_position_handle_max_frame_virtual_elapsed() {
    let fps = crate::domain::timing::DEFAULT_FPS;
    let start = Instant::now();
    let target_duration = game_frames_to_seconds(MAX_SFMT_FRAME as f64, fps);
    let virtual_now = start
        + std::time::Duration::from_secs_f64(game_frames_to_seconds(
            (MAX_SFMT_FRAME + 100) as f64,
            fps,
        ));
    let mut state = FieldState::default();
    state.observed.push(start);
    state.timer_kind = FieldTimerKind::Target;
    state.timer_start = Some(start);
    state.timer_seconds = target_duration;
    state.results.push(FieldResult {
        start_consumption: 486,
        first_wait_frame: 0,
        last_wait_frame: 486,
        first_consumption: 486,
        last_consumption: 486,
        intervals: Vec::new(),
        target_consumptions: Vec::new(),
        next_blink_wait_frame: 0,
        target_wait_frame: Some(MAX_SFMT_FRAME),
        target_exact: Some(true),
        target_frame_before: Some(MAX_SFMT_FRAME),
        target_frame_after: Some(MAX_SFMT_FRAME),
        observed_model: 0,
        models: 1,
        timeline: Vec::new(),
        candidate_blinks: Vec::new(),
        target_blink_count: Some(0),
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: None,
    });

    assert_eq!(state.timer_remaining_at(virtual_now), Some(0.0));
    assert!(state.finish_target_timer_if_elapsed(virtual_now));
    assert!(state.timer_is_running());
    assert!(state.timer_is_finished());
    assert!(state
        .current_timeline_frame_at(fps, virtual_now)
        .is_some_and(|frame| frame > MAX_SFMT_FRAME));
    assert!(state
        .remaining_from_observation_at(MAX_SFMT_FRAME, fps, virtual_now)
        .is_err());
}

#[test]
fn target_timing_distinguishes_exact_and_crossing_steps() {
    let values = SearchValues::new(0, generate_u64_stream(1, 32));
    let event = BlinkEvent {
        wait_frame: 0,
        consumption: 0,
        status: ModelStatus::new(2),
        stream_cursor: 0,
        model_consumption: None,
    };
    let crossing = find_target_timing(&event, 1, &values).unwrap();
    assert!(!crossing.exact);
    assert_eq!(crossing.absolute_wait_frame, 2);
    assert_eq!((crossing.frame_before, crossing.frame_after), (0, 2));

    let exact = find_target_timing(&event, 2, &values).unwrap();
    assert!(exact.exact);
    assert_eq!(exact.absolute_wait_frame, 2);
}

#[test]
fn timeleap_retry_restarts_shift_matching_after_frame1() {
    let mut state = FieldState::default();
    state.timeleap_locked = true;
    state.timeleap_stage = TimeleapStage::Failed;
    state.timeleap_frame1_wait_seconds = 1.25;
    let anchor = Instant::now() - Duration::from_secs(2);
    state.timeleap_wait_started = Some(anchor);
    state.timeleap_shift_times = vec![anchor + Duration::from_millis(500)];
    state.timer_start = Some(Instant::now());

    assert!(state.timeleap_retry(59.8621));
    assert!(state.timeleap_locked);
    assert_eq!(state.timeleap_stage, TimeleapStage::ShiftWaiting);
    assert!(state.timeleap_accepts_shift());
    assert!(state.timer_start.is_none());
    assert_eq!(state.timeleap_wait_started, Some(anchor));
    assert!(state.timeleap_shift_times.is_empty());
    assert!(!state.can_edit_general_settings());
}

#[test]
fn timeleap_recheck_requires_an_adopted_timeline() {
    let mut state = FieldState::default();
    assert!(state
        .begin_timeleap_recheck(false, crate::domain::timing::DEFAULT_FPS)
        .is_err());
    assert_eq!(state.timeleap_stage, TimeleapStage::Off);
}

#[test]
fn adopted_timeleap_timeline_continues_from_the_shared_sfmt_cache() {
    let seed = 0x1234_5678;
    let target = 1_500;
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=20_000,
        20_000,
        crate::domain::timing::DEFAULT_FPS,
    )
    .unwrap();
    let config = TimeleapConfig {
        seed,
        start_frame: 480,
        target_frame: target,
        models: 1,
        min_frame1_elapsed_ticks: 30,
        max_frame1_elapsed_ticks: 90,
        mode: TimeleapMode::Menu,
        is_boy: true,
        handoff_range_frames: 20,
        courier: CourierConfig::default(),
    };
    let initial_candidate = search_candidates_from_cache(&config, &cache)
        .unwrap()
        .into_iter()
        .flat_map(|group| group.candidates)
        .next()
        .unwrap();
    let predicted = predict_shift_times_until(
        &cache,
        &initial_candidate,
        0,
        initial_candidate.post_tick.saturating_mul(2) + 10_000,
    );
    let observed = *predicted.get(2).expect("three post-event blinks");
    let candidate =
        matching_shift_continuations(&cache, &initial_candidate, 0, &[observed], 0, 0, observed)
            .into_iter()
            .next()
            .expect("advance candidate to an observed blink");
    let shift_at = Instant::now();
    let post_position = candidate.post_sfmt_position;
    let post_tick = candidate.post_tick;
    let mut state = FieldState {
        seed: format!("{seed:08X}"),
        timeleap_source_inputs_initialized: true,
        timeleap_source_seed: format!("{seed:08X}"),
        timeleap_source_starting_frame: config.start_frame.to_string(),
        timeleap_source_npc_count: (config.models - 1).to_string(),
        results: vec![FieldResult {
            start_consumption: config.start_frame,
            first_wait_frame: 0,
            last_wait_frame: 0,
            first_consumption: config.start_frame,
            last_consumption: config.start_frame,
            intervals: Vec::new(),
            target_consumptions: Vec::new(),
            next_blink_wait_frame: 0,
            target_wait_frame: None,
            target_exact: None,
            target_frame_before: None,
            target_frame_after: None,
            observed_model: 0,
            models: config.models,
            timeline: Vec::new(),
            candidate_blinks: Vec::new(),
            target_blink_count: None,
            target_blink_count_offset_from_first_row: 0,
            timeline_cursor: None,
            target_cursor: None,
        }],
        timeleap_confirmed_candidate: Some(candidate),
        timeleap_shift_times: vec![shift_at],
        ..FieldState::default()
    };
    state.set_observation_cache(cache);

    assert!(state.adopt_timeleap_timeline());
    assert!(state.observation_cache_for_seed(seed).is_some());
    assert_eq!(state.timer_kind, FieldTimerKind::NextBlink);
    assert!(!state.timer_is_running());
    assert_eq!(state.results[0].last_wait_frame, post_tick * 2);
    assert_eq!(state.results[0].timeline[0].timeline_frame, post_tick * 2);
    state.prepare_shared_timeline_cache().unwrap();
    assert_eq!(
        state
            .resolved_timeline_cache
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .anchor_tick,
        post_tick
    );
    assert_eq!(
        state.current_sfmt_frame(crate::domain::timing::DEFAULT_FPS),
        Some(post_position)
    );
    // Frame1 is an absolute SFMT position. The display-frame clock is a
    // separate coordinate and must not be used as the search origin.
    state.timeleap_timeline_anchor = Some(Instant::now() + Duration::from_secs(60));
    state.timeleap_frame1_input = (post_position + 100).to_string();
    let (frame1_config, _, _, _) = state
        .prepare_timeleap_frame1(crate::domain::timing::DEFAULT_FPS)
        .expect("prepare Frame1 from the live SFMT position");
    assert_eq!(frame1_config.start_frame, post_position);

    state.target_consumption = (post_position + 500).to_string();
    let (candidate_key, _) = state
        .prepare_timeleap_candidate_search(crate::domain::timing::DEFAULT_FPS)
        .expect("prepare candidate search from the live SFMT position");
    assert_eq!(candidate_key.config.start_frame, config.start_frame);

    // UI does this extension on its background worker and retries timer startup
    // when the shared timeline has produced the first post-Timeleap blink.
    state
        .extend_timeline_if_needed(crate::domain::timing::DEFAULT_FPS)
        .unwrap();
    state
        .start_blink_timer(crate::domain::timing::DEFAULT_FPS)
        .unwrap();
    assert!(state.timer_is_running());
    assert_eq!(state.timer_kind, FieldTimerKind::NextBlink);

    let target_result = state
        .retarget_confirmed_result_from_cache(post_position + 100, None)
        .unwrap();
    assert!(target_result.target_wait_frame.is_some());
    state.results[0] = target_result;
    state.timeleap_enabled = true;
    state.timeleap_stage = TimeleapStage::Succeeded;
    state.timeleap_frame1_deadline = Some(Instant::now() + Duration::from_secs(60));
    let target_remaining = state
        .preview_remaining(FieldTimerKind::Target, crate::domain::timing::DEFAULT_FPS)
        .unwrap();
    assert!(target_remaining > 0.0 && target_remaining < 60.0);
}

#[test]
fn courier_validation_requires_fixed_pid_and_valid_iv_combination() {
    let mut state = FieldState::default();
    state.timeleap_mode = TimeleapMode::Courier;
    state.timeleap_courier_pid_type = 3;
    assert!(state.timeleap_courier_validation().is_err());
    state.timeleap_courier_pid = "12345678".into();
    state.timeleap_courier_iv_count = "7".into();
    assert!(state.timeleap_courier_validation().is_err());
    state.timeleap_courier_iv_count = "1".into();
    state.timeleap_courier_fixed_ivs[0] = 31;
    assert!(state.timeleap_courier_validation().is_ok());
}
