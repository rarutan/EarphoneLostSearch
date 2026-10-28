use super::*;

#[test]
fn shift_recheck_interval_display_starts_between_first_and_second_shift_and_resets_on_retry() {
    let first_recheck = Instant::now();
    let mut state = FieldState::default();
    state.timeleap_locked = true;
    state.timeleap_stage = TimeleapStage::Recheck;
    state.timeleap_wait_started = Some(first_recheck);

    assert!(state.record_timeleap_shift(first_recheck + Duration::from_millis(200)));
    assert_eq!(
        state.timeleap_recheck_intervals(crate::domain::timing::DEFAULT_FPS),
        Some(vec![]),
        "the first Shift anchors matching but is not an interval"
    );
    assert!(state.record_timeleap_shift(first_recheck + Duration::from_millis(1_200)));
    assert!(state.record_timeleap_shift(first_recheck + Duration::from_millis(2_700)));
    assert_eq!(
        state.timeleap_recheck_intervals(crate::domain::timing::DEFAULT_FPS),
        Some(vec![60, 90])
    );

    state.timeleap_stage = TimeleapStage::Failed;
    assert!(state.timeleap_retry(crate::domain::timing::DEFAULT_FPS));
    assert_eq!(
        state.timeleap_recheck_intervals(crate::domain::timing::DEFAULT_FPS),
        Some(vec![]),
        "retry starts a fresh Shift list"
    );
    assert!(state.record_timeleap_shift(first_recheck + Duration::from_secs(4)));
    assert_eq!(
        state.timeleap_recheck_intervals(crate::domain::timing::DEFAULT_FPS),
        Some(vec![]),
        "the first Shift after retry is again only a matching anchor"
    );
}

#[test]
fn timeleap_shift_matching_uses_nominal_tick_anchor_and_blink_correction() {
    let seed = 0x63D9_A401;
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=20_000,
        20_000,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate cache");
    let mut config = TimeleapConfig {
        seed,
        start_frame: 0,
        target_frame: 0,
        models: 15,
        min_frame1_elapsed_ticks: 0,
        max_frame1_elapsed_ticks: 0,
        mode: TimeleapMode::Menu,
        is_boy: false,
        handoff_range_frames: 1_000,
        courier: CourierConfig::default(),
    };
    let start = crate::domain::timeleap::TimeleapTimelineStart::initial(0, config.models);
    let broad_preparation = crate::domain::timeleap::run_timeleap_frame1_preparation(
        &config,
        0,
        &start,
        &cache,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .expect("prepare a broad reachable Frame1 window");
    let nominal_frame1 = broad_preparation
        .candidates
        .iter()
        .find(|candidate| candidate.frame1 >= 500)
        .expect("broad window reaches Frame 500")
        .frame1;
    config.target_frame = nominal_frame1;
    config.handoff_range_frames = 20;
    let preparation = crate::domain::timeleap::run_timeleap_frame1_preparation(
        &config,
        nominal_frame1,
        &start,
        &cache,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .expect("prepare Frame1 window");
    let nominal_time = preparation.nominal_tick * 2;
    let selected_candidate = preparation
        .candidates
        .iter()
        .find(|candidate| candidate.frame1 == nominal_frame1)
        .expect("window includes the requested Frame1")
        .clone();
    let selected_predictions =
        predict_shift_times_until(&cache, &selected_candidate, 0, nominal_time + 10_000);
    let (candidate, observed) = preparation
        .candidates
        .iter()
        .filter(|candidate| candidate.frame1 > nominal_frame1)
        .find_map(|candidate| {
            let predicted = predict_shift_times_until(&cache, candidate, 0, nominal_time + 10_000);
            predicted
                .into_iter()
                .find(|time| !selected_predictions.contains(time))
                .map(|time| (candidate.clone(), time))
        })
        .expect("another Frame1 branch has a distinct later blink");
    assert!(frame1_tick_is_in_research_range(
        candidate.frame1_tick,
        preparation.nominal_tick,
        20
    ));
    let adjustment = 3_i64;

    let anchor = Instant::now();
    let elapsed_frames = observed - nominal_time + adjustment;
    let at = anchor
        + Duration::from_secs_f64(elapsed_frames as f64 / crate::domain::timing::DEFAULT_FPS);
    let mut state = FieldState::default();
    state.seed = format!("{seed:08X}");
    state.timeleap_enabled = true;
    state.target_model = "1".into();
    state.tolerance = "0".into();
    state.timeleap_search_range = "20".into();
    state.adjust = adjustment;
    state.observation_cache = Some(cache);
    state.timeleap_locked = true;
    state.timeleap_stage = TimeleapStage::ShiftWaiting;
    state.timeleap_frame1_reachable = Some(true);
    state.timeleap_execution_candidates = vec![selected_candidate.clone(), candidate.clone()];
    state.timeleap_selected_frame = Some(nominal_frame1);
    state.timeleap_frame1_target = Some(nominal_frame1);
    state.timeleap_nominal_tick = Some(preparation.nominal_tick);
    state.timeleap_observation_anchor_frame = Some(nominal_time);
    state.timeleap_wait_started = Some(anchor);
    state.results.push(FieldResult {
        start_consumption: candidate.pre_sfmt_position,
        first_wait_frame: 0,
        last_wait_frame: 0,
        first_consumption: candidate.pre_sfmt_position,
        last_consumption: candidate.pre_sfmt_position,
        intervals: Vec::new(),
        target_consumptions: Vec::new(),
        next_blink_wait_frame: 0,
        target_wait_frame: None,
        target_exact: None,
        target_frame_before: None,
        target_frame_after: None,
        observed_model: 0,
        models: 15,
        timeline: Vec::new(),
        candidate_blinks: Vec::new(),
        target_blink_count: None,
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: None,
    });
    assert_eq!(
        state.resolve_timeleap_shift(at, crate::domain::timing::DEFAULT_FPS),
        TimeleapStage::Succeeded
    );
    assert_eq!(
        state.target_model, "1",
        "recheck does not rewrite the input"
    );
    assert_eq!(state.timeleap_confirmed_frame1, Some(candidate.frame1));
    assert_ne!(
        state.timeleap_confirmed_frame1,
        state.timeleap_selected_frame
    );
    assert_eq!(
        state.timeleap_frame1_indicator(),
        Some(TimeleapFrame1Indicator::Reachable),
        "a different observed Frame1 must not rewrite the pre-Ready reachability result"
    );
    let confirmed = state
        .timeleap_confirmed_candidate
        .as_ref()
        .expect("confirmed candidate continues at matched blink");
    assert_eq!(confirmed.post_tick.saturating_mul(2), observed);
    assert!(confirmed.post_tick > candidate.post_tick);
    assert!(state.adopt_timeleap_timeline());
    assert!(state.can_edit_target());
    assert_eq!(state.timeleap_timeline_anchor, Some(at));
    assert_eq!(state.results[0].last_wait_frame, observed);
    let mut expected = state.results[0]
        .target_cursor
        .clone()
        .expect("adopted cursor");
    assert!(expected.timeleap_runtime.is_some());
    let mut expected_sfmt = state
        .observation_cache
        .as_ref()
        .unwrap()
        .sfmt_at_checked(expected.stream_cursor)
        .unwrap()
        .unwrap();
    let used = advance_timeleap_runtime_tick(
        &mut expected.status,
        expected.timeleap_runtime.as_mut().unwrap(),
        &mut expected.stream_cursor,
        &mut expected_sfmt,
    )
    .0;
    expected.consumption += used;
    expected.tick += 1;
    state.prepare_shared_timeline_cache().unwrap();
    let shared_timeline = state.resolved_timeline_cache.as_ref().unwrap().clone();
    let mut shared = shared_timeline.lock().unwrap();
    assert_eq!(shared.append_chunk(1), 1);
    assert_eq!(shared.cursor, expected);
    drop(shared);

    let recheck_now = at + Duration::from_secs(40);
    let recheck_frame = state
        .current_timeline_frame_at(crate::domain::timing::DEFAULT_FPS, recheck_now)
        .expect("adopted Timeline has a live clock");
    let recheck_tick = recheck_frame.div_euclid(2);
    let observation_cache = state.observation_cache.clone().unwrap();
    let recheck_cursor = {
        let mut shared = shared_timeline.lock().unwrap();
        let ticks_needed = recheck_tick
            .saturating_add(16)
            .saturating_sub(shared.cursor.tick)
            .max(0) as usize;
        assert_eq!(shared.append_chunk(ticks_needed), ticks_needed);
        shared
            .cursor_at_tick(recheck_tick, &observation_cache)
            .unwrap()
            .expect("recheck point is in the shared Timeline")
    };
    let timeline_generation_before_recheck = state.confirmed_target_timeline_generation;
    state.timeleap_frame1_reachable = Some(false);
    state
        .begin_timeleap_recheck_at(false, crate::domain::timing::DEFAULT_FPS, recheck_now)
        .unwrap();
    assert!(
        state.confirmed_target_timeline_generation > timeline_generation_before_recheck,
        "recheck must invalidate snapshots rooted at the adopted Timeline"
    );
    assert_eq!(state.timeleap_stage, TimeleapStage::Recheck);
    assert_eq!(state.timeleap_wait_started, Some(recheck_now));
    assert_eq!(state.timeleap_observation_anchor_frame, Some(recheck_frame));
    assert_eq!(state.timeleap_recheck_pause_frame, Some(recheck_frame));
    assert_eq!(
        state.timeleap_recheck_intervals(crate::domain::timing::DEFAULT_FPS),
        Some(vec![])
    );
    assert_eq!(
        state.current_timeline_frame_at(
            crate::domain::timing::DEFAULT_FPS,
            recheck_now + Duration::from_secs(30),
        ),
        Some(recheck_frame),
        "blink recheck freezes the displayed Timeline position",
    );
    assert_eq!(state.timeleap_nominal_tick, Some(preparation.nominal_tick));
    assert_eq!(state.timeleap_execution_candidates.len(), 1);
    let recheck_candidate = &state.timeleap_execution_candidates[0];
    assert_eq!(recheck_candidate.post_tick, recheck_cursor.tick);
    assert_eq!(
        recheck_candidate.post_sfmt_position,
        recheck_cursor.stream_cursor
    );
    assert_eq!(recheck_candidate.post_frame, recheck_cursor.consumption);
    assert_eq!(recheck_candidate.post_status.model, recheck_cursor.status);
    assert_eq!(
        recheck_candidate.post_status.phase,
        recheck_cursor.timeleap_runtime.as_ref().unwrap().phase
    );
    assert_eq!(
        recheck_candidate.post_status.fidget_cd,
        recheck_cursor.timeleap_runtime.as_ref().unwrap().fidget_cd
    );
    assert_eq!(
        recheck_candidate.post_status.is_boy,
        recheck_cursor.timeleap_runtime.as_ref().unwrap().is_boy
    );
    assert_eq!(
        recheck_candidate.post_status.raining,
        recheck_cursor.timeleap_runtime.as_ref().unwrap().raining
    );

    // A failed Shift attempt followed by a retry must keep the exact recheck
    // anchor; it must not restart from the original Frame1 deadline.
    state.adjust = 0;
    state.tolerance = "0".to_owned();
    let predicted_shifts = predict_shift_times_until(
        &observation_cache,
        recheck_candidate,
        0,
        recheck_frame + 10_000,
    );
    let predicted = *predicted_shifts
        .iter()
        .find(|frame| **frame > recheck_frame)
        .expect("the adopted Timeline has a future blink");

    let failed_observation = predicted.saturating_add(2);
    assert!(!predicted_shifts.contains(&failed_observation));
    let failed_elapsed_frames = failed_observation - recheck_frame;
    let failed_shift_at = recheck_now
        + Duration::from_secs_f64(
            failed_elapsed_frames as f64 / crate::domain::timing::DEFAULT_FPS,
        );
    assert_eq!(
        state.resolve_timeleap_shift(failed_shift_at, crate::domain::timing::DEFAULT_FPS),
        TimeleapStage::Failed
    );
    assert!(state.timeleap_retry(crate::domain::timing::DEFAULT_FPS));
    assert_eq!(state.timeleap_stage, TimeleapStage::ShiftWaiting);
    assert_eq!(state.timeleap_wait_started, Some(recheck_now));
    assert_eq!(state.timeleap_observation_anchor_frame, Some(recheck_frame));

    let elapsed_frames = predicted - recheck_frame - state.adjust;
    let shift_at = recheck_now
        + Duration::from_secs_f64(elapsed_frames as f64 / crate::domain::timing::DEFAULT_FPS);
    assert_eq!(
        state.resolve_timeleap_shift(shift_at, crate::domain::timing::DEFAULT_FPS),
        TimeleapStage::Succeeded
    );
    assert_eq!(
        state
            .timeleap_confirmed_candidate
            .as_ref()
            .unwrap()
            .post_tick
            * 2,
        predicted
    );
    assert_eq!(
        state.timeleap_frame1_indicator(),
        Some(TimeleapFrame1Indicator::Unreachable),
        "a later blink recheck must preserve the frozen ordinary Timeline result"
    );
}

#[test]
fn shift_recheck_rejects_invalid_position_radius_instead_of_using_a_default() {
    let at = Instant::now();
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_locked = true;
    state.timeleap_stage = TimeleapStage::ShiftWaiting;
    state.timeleap_wait_started = Some(at - Duration::from_secs(1));
    state.target_model = "0".into();
    state.tolerance = "not-a-number".into();

    let result = state.prepare_timeleap_shift_resolution(at, 60.0);

    assert!(result.is_err());
    assert!(!state.timeleap_shift_resolution_pending());
    assert_eq!(state.timeleap_stage, TimeleapStage::Failed);
    assert!(state
        .notice
        .as_deref()
        .is_some_and(|notice| notice.contains("範囲")));
}

#[test]
fn shift_recheck_uses_adopted_observed_model_without_changing_npc_input() {
    let at = Instant::now();
    let seed = 0xA02D_5C73;
    let candidate_status = crate::domain::timeleap::TimeleapStatus {
        model: crate::domain::rng::ModelStatus::new(2),
        phase: 0,
        fidget_cd: -1,
        is_boy: true,
        raining: false,
    };
    let candidate = crate::domain::timeleap::TimeleapCandidate {
        frame1: 100,
        frame1_tick: 50,
        post_tick: 60,
        grace_frames: 0,
        pre_frame: 100,
        pre_sfmt_position: 100,
        pre_status: candidate_status.clone(),
        post_frame: 120,
        post_sfmt_position: 120,
        post_status: candidate_status,
        target_inputs: vec![crate::domain::timeleap::TimeleapTargetInput {
            frame: 120,
            tick: 60,
        }]
        .into(),
    };
    let mut state = FieldState {
        seed: format!("{seed:08X}"),
        target_model: "1".into(),
        timeleap_locked: true,
        timeleap_stage: TimeleapStage::ShiftWaiting,
        timeleap_wait_started: Some(at - Duration::from_secs(1)),
        timeleap_observation_anchor_frame: Some(100),
        timeleap_nominal_tick: Some(50),
        timeleap_search_range: "20".into(),
        timeleap_execution_candidates: vec![candidate],
        tolerance: "0".into(),
        results: vec![FieldResult {
            start_consumption: 100,
            first_wait_frame: 0,
            last_wait_frame: 0,
            first_consumption: 100,
            last_consumption: 100,
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
    state.set_observation_cache(
        crate::domain::rng::generate_observation_cache(
            seed,
            0..=2_000,
            2_000,
            crate::domain::timing::DEFAULT_FPS,
        )
        .expect("generate observation cache"),
    );

    let request = state
        .prepare_timeleap_shift_resolution(at, crate::domain::timing::DEFAULT_FPS)
        .expect("prepare Shift resolution")
        .expect("Shift resolution request");

    assert_eq!(request.model, 0, "use the adopted result's observed NPC");
    assert_eq!(state.target_model, "1", "keep the user's input unchanged");
}

#[test]
fn stale_shift_resolution_generation_cannot_clear_or_apply_current_request() {
    let mut state = FieldState::default();
    state.timeleap_shift_resolution_generation = 2;
    state.timeleap_shift_resolution_pending = true;
    state.timeleap_stage = TimeleapStage::Recheck;

    assert!(state
        .apply_timeleap_shift_resolution(1, TimeleapShiftResolutionSummary::default())
        .is_none());

    assert!(state.timeleap_shift_resolution_pending());
    assert_eq!(state.timeleap_stage, TimeleapStage::Recheck);
}

#[test]
fn only_an_exhaustive_no_match_summary_is_a_final_recheck_failure() {
    assert!(TimeleapShiftResolutionSummary::default().has_no_matches());
    assert!(!TimeleapShiftResolutionSummary {
        matches_found: true,
        multiple_future_states: false,
        ..TimeleapShiftResolutionSummary::default()
    }
    .has_no_matches());
    assert!(!TimeleapShiftResolutionSummary {
        matches_found: true,
        multiple_future_states: true,
        ..TimeleapShiftResolutionSummary::default()
    }
    .has_no_matches());
}
