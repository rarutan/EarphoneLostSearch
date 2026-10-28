use super::*;

#[test]
fn confirmed_target_search_requires_adoption_and_rejects_stale_or_changes_only_target() {
    let mut unconfirmed = FieldState::default();
    assert!(unconfirmed.begin_confirmed_target_search().is_err());

    let mut state = confirmed_target_search_state();
    state.timeleap_frame1_input = "481".into();
    assert!(state.can_edit_target());
    state.confirmed_target_search_form.drift_search_enabled = true;
    state
        .confirmed_target_search_form
        .drift_search_generation_start_frame = "950".into();
    state.confirmed_target_search_form.drift_range_ticks = "4".into();
    let confirmed_frame1 = state.timeleap_confirmed_frame1;
    let generation_start = state.timeleap_generation_start_frame.clone();
    let (generation, request, _) = state
        .begin_confirmed_target_search()
        .expect("adopted Timeleap state permits search");
    assert_eq!(request.start.frame, 500);
    assert_eq!(request.confirmed_frame1, 480);
    assert_eq!(request.generation_start_frame, 900);
    assert_eq!(request.drift_search_generation_start_frame, Some(950));
    assert_eq!(
        state
            .confirmed_target_search_form
            .result_generation_start_frame,
        None,
        "the old display snapshot is not replaced until the new result arrives"
    );
    assert_eq!(
        state
            .confirmed_target_search_form
            .result_drift_search_generation_start_frame,
        None,
        "an in-flight search does not replace the previous result decoration snapshot"
    );
    assert_eq!(
        request.timeline_generation,
        state.confirmed_target_timeline_generation
    );

    let row = crate::domain::timeleap::ConfirmedTargetSearchRow {
        frame1: 480,
        target_frame: 998,
        drift_frames: -2,
        grace_frames: 2,
        generation_start_frame: Some(900),
    };
    assert!(state.apply_confirmed_target_search_result(
        generation,
        &request,
        Ok(crate::domain::timeleap::ConfirmedTargetSearchResult::from_rows(vec![row], Some(950),)),
    ));
    assert_eq!(
        state
            .confirmed_target_search_form
            .result_drift_marker_boundary,
        Some(1)
    );
    assert_eq!(state.committed_target_frame(), Some(998));
    assert_eq!(state.timeleap_confirmed_frame1, confirmed_frame1);
    assert_eq!(state.timeleap_frame1_input, "481");
    assert_eq!(state.timeleap_source_frame1, "480");
    assert_eq!(state.timeleap_generation_start_frame, generation_start);

    let mut manually_set = confirmed_target_search_state();
    manually_set.timeleap_frame1_input = "481".into();
    manually_set.timeleap_frame1_reachable = Some(false);
    manually_set.confirmed_target_search_form.status = ConfirmedTargetSearchStatus::Results(vec![
        crate::domain::timeleap::ConfirmedTargetSearchRow {
            frame1: 480,
            target_frame: 998,
            drift_frames: -2,
            grace_frames: 2,
            generation_start_frame: Some(900),
        },
    ]);
    let row = match &manually_set.confirmed_target_search_form.status {
        ConfirmedTargetSearchStatus::Results(rows) => rows[0].clone(),
        _ => unreachable!(),
    };
    assert!(manually_set.set_confirmed_target_frame_from_search_row(&row));
    assert_eq!(manually_set.timeleap_frame1_input, "481");
    assert_eq!(
        manually_set.timeleap_frame1_indicator(),
        Some(TimeleapFrame1Indicator::Unreachable),
        "setting only TargetFrame must not rewrite Frame1 or its frozen reachability verdict"
    );
    assert_eq!(manually_set.committed_target_frame(), Some(998));

    let (multiple_generation, multiple_request, _) = state
        .begin_confirmed_target_search()
        .expect("second search starts from the same adopted state");
    assert!(
        matches!(
            &state.confirmed_target_search_form.status,
            ConfirmedTargetSearchStatus::Running
        ),
        "re-searching from displayed results immediately enters the running state"
    );
    assert!(!state.apply_confirmed_target_search_result(
        multiple_generation,
        &multiple_request,
        Ok(
            crate::domain::timeleap::ConfirmedTargetSearchResult::from_rows(
                vec![
                    crate::domain::timeleap::ConfirmedTargetSearchRow {
                        frame1: 480,
                        target_frame: 997,
                        drift_frames: -4,
                        grace_frames: 2,
                        generation_start_frame: Some(900),
                    },
                    crate::domain::timeleap::ConfirmedTargetSearchRow {
                        frame1: 480,
                        target_frame: 999,
                        drift_frames: 0,
                        grace_frames: 2,
                        generation_start_frame: Some(900),
                    },
                ],
                Some(950),
            )
        ),
    ));
    assert_eq!(state.committed_target_frame(), Some(998));

    let (stale_generation, stale_request, _) = state
        .begin_confirmed_target_search()
        .expect("third search starts from the same adopted state");
    let stale_row = crate::domain::timeleap::ConfirmedTargetSearchRow {
        frame1: 480,
        target_frame: 996,
        drift_frames: -4,
        grace_frames: 2,
        generation_start_frame: Some(900),
    };
    state.invalidate_confirmed_target_search();
    assert!(!state.set_confirmed_target_frame_from_search_row(&stale_row));
    assert!(!state.apply_confirmed_target_search_result(
        stale_generation,
        &stale_request,
        Ok(crate::domain::timeleap::ConfirmedTargetSearchResult::from_rows(Vec::new(), Some(950),)),
    ));
    assert_eq!(state.committed_target_frame(), Some(998));
    let displayed_rows = match &state.confirmed_target_search_form.status {
        ConfirmedTargetSearchStatus::Results(rows) => rows.clone(),
        status => panic!("completed ambiguous search should keep its visible rows: {status:?}"),
    };
    let timeline_generation = state.confirmed_target_timeline_generation;
    state.start_observation();
    assert!(state.confirmed_target_timeline_generation > timeline_generation);
    assert!(matches!(
        &state.confirmed_target_search_form.status,
        ConfirmedTargetSearchStatus::Results(rows)
            if rows == &displayed_rows
    ));
}

#[test]
fn unique_target_result_preserves_planned_frame1_and_actual_frame1_display() {
    let status = crate::domain::timeleap::TimeleapStatus {
        model: crate::domain::rng::ModelStatus::new(1),
        phase: 0,
        fidget_cd: 0,
        is_boy: true,
        raining: false,
    };
    let request = crate::domain::timeleap::ConfirmedTargetSearchRequest {
        seed: 0xA02D_5C73,
        confirmed_frame1: 480,
        timeline_generation: 7,
        source_config: None,
        start: crate::domain::timeleap::ConfirmedTimeleapStart {
            frame: 500,
            sfmt_position: 500,
            tick: 250,
            status,
        },
        baseline_target_frame: 900,
        generation_start_frame: 900,
        drift_search_generation_start_frame: None,
        drift_range_ticks: 0,
        grace_min_frames: 0,
        consider_delay: None,
    };
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_stage = TimeleapStage::Succeeded;
    state.set_timeleap_timeline_anchor_for_test(Some(Instant::now()), Some(490));
    state.timeleap_frame1_input = "481".into();
    state.timeleap_confirmed_frame1 = Some(480);
    state.timeleap_source_frame1 = "480".into();
    state.confirmed_target_timeline_generation = request.timeline_generation;
    state.confirmed_target_search_form.generation = 1;
    state.confirmed_target_search_form.status = ConfirmedTargetSearchStatus::Running;
    state.confirmed_target_search_form.request = Some(request.clone());

    let row = crate::domain::timeleap::ConfirmedTargetSearchRow {
        frame1: 480,
        target_frame: 998,
        drift_frames: -2,
        grace_frames: 2,
        generation_start_frame: Some(900),
    };
    assert!(state.apply_confirmed_target_search_result(
        1,
        &request,
        Ok(crate::domain::timeleap::ConfirmedTargetSearchResult::from_rows(vec![row], None,)),
    ));

    assert_eq!(state.committed_target_frame(), Some(998));
    assert_eq!(state.timeleap_frame1_input, "481");
    assert_eq!(state.timeleap_source_frame1, "480");
    assert_eq!(state.timeleap_lifecycle_policy().summary_frame1, Some(480));
}

#[test]
fn correction_timeline_cursor_stays_fixed_as_wall_clock_advances() {
    let mut state = FieldState::empty_timeline_for_test();
    state.mode = FieldMode::Correction;
    state.correction_timeline_frame = Some(321);
    let fps = crate::domain::timing::DEFAULT_FPS;
    let now = Instant::now();

    assert_eq!(state.current_timeline_frame_at(fps, now), Some(321));
    assert_eq!(
        state.current_timeline_frame_at(fps, now + std::time::Duration::from_secs(30)),
        Some(321)
    );
}

#[test]
fn confirmed_target_search_preserves_frame1_unreachable_as_distinct_status() {
    let status = super::forms::confirmed_target_search_failure_status(
        crate::domain::timeleap::TimeleapSearchFailure::Frame1Unreachable,
    )
    .expect("Frame1-unreachable is a displayable search outcome");
    assert!(matches!(
        status,
        ConfirmedTargetSearchStatus::Frame1Unreachable
    ));
    assert!(matches!(
        super::forms::confirmed_target_search_failure_status(
            crate::domain::timeleap::TimeleapSearchFailure::GenerationFrameUnreachable
        ),
        Some(ConfirmedTargetSearchStatus::GenerationFrameUnreachable)
    ));
    assert!(super::forms::confirmed_target_search_failure_status(
        crate::domain::timeleap::TimeleapSearchFailure::Cancelled
    )
    .is_none());
}

#[test]
fn target_search_set_action_changes_only_target_and_offset_math_preserves_base() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_stage = TimeleapStage::Succeeded;
    state.timeleap_timeline_anchor = Some(Instant::now());
    state.target_consumption = "1000".into();
    state.confirmed_target_search_form.status = ConfirmedTargetSearchStatus::Results(vec![
        crate::domain::timeleap::ConfirmedTargetSearchRow {
            frame1: 480,
            target_frame: 998,
            drift_frames: -2,
            grace_frames: 2,
            generation_start_frame: Some(900),
        },
    ]);
    let row = match &state.confirmed_target_search_form.status {
        ConfirmedTargetSearchStatus::Results(rows) => rows[0].clone(),
        _ => unreachable!(),
    };
    state.timeleap_frame1_input = "481".into();
    assert!(state.set_confirmed_target_frame_from_search_row(&row));
    assert_eq!(state.timeleap_frame1_input, "481");
    assert_eq!(state.committed_target_frame(), Some(998));

    state.encounter_offset = "20".into();
    assert_eq!(state.preview_timeleap_offset_drift(-2, false), Some(22));
    assert_eq!(state.preview_timeleap_offset_drift(-2, true), Some(22));
    assert_eq!(state.preview_timeleap_offset_drift(1, false), None);
    assert_eq!(state.preview_timeleap_offset_drift(2, false), Some(18));
    assert_eq!(state.preview_timeleap_offset_drift(2, true), Some(18));
    assert_eq!(state.apply_timeleap_offset_drift(-2, false), Some(22));
    assert_eq!(state.encounter_offset, "22");
    assert!(state.use_offset);
    assert_eq!(state.apply_timeleap_offset_drift(4, true), Some(18));
    assert_eq!(state.apply_timeleap_offset_drift(1, false), None);
    assert_eq!(state.encounter_offset, "18");

    state.enter_correction_mode(crate::domain::timing::DEFAULT_FPS);
    assert!(!state.can_set_confirmed_target_frame_from_search());
    assert!(!state.set_confirmed_target_frame_from_search_row(&row));
    assert_eq!(state.committed_target_frame(), Some(998));
    assert_eq!(state.preview_timeleap_offset_drift(2, false), Some(16));
}

#[test]
fn frame1_offset_uses_candidate_ticks_across_frame_parity() {
    let mut state = FieldState::default();
    state.encounter_offset = "0".into();
    state.timeleap_frame1_input = "10000".into();
    let status = TimeleapStatus {
        model: crate::domain::rng::ModelStatus::new(1),
        phase: 0,
        fidget_cd: -1,
        is_boy: true,
        raining: false,
    };
    let candidate = |frame1, frame1_tick| TimeleapCandidate {
        frame1,
        frame1_tick,
        post_tick: frame1_tick,
        grace_frames: 2,
        pre_frame: frame1,
        pre_sfmt_position: frame1,
        pre_status: status.clone(),
        post_frame: frame1,
        post_sfmt_position: frame1,
        post_status: status.clone(),
        target_inputs: Vec::new().into(),
    };
    state.timeleap_candidate_groups = vec![TimeleapCandidateGroup {
        frame1_start: 10_000,
        frame1_end: 10_001,
        frame1_tick_start: 5_000,
        frame1_tick_end: 5_001,
        representative_frame1: 10_000,
        frame2_ticks: 0,
        grace_frames: 4,
        target_frame_start: 0,
        target_frame_end: 0,
        target_tick_start: 0,
        target_tick_end: 0,
        representative_target_frame: 0,
        target_grace_frames: 0,
        candidates: smallvec::smallvec![candidate(10_000, 5_000), candidate(10_001, 5_001)],
    }];

    assert_eq!(state.preview_timeleap_frame1_offset(10_000, false), Some(0));
    assert_eq!(
        state.preview_timeleap_frame1_offset(10_001, false),
        Some(-2)
    );
    assert_eq!(state.preview_timeleap_frame1_offset(10_001, true), Some(-2));
    assert_eq!(state.apply_timeleap_frame1_offset(10_001, false), Some(-2));
    assert_eq!(state.encounter_offset, "-2");
    assert!(state.use_offset);
}

#[test]
fn target_frame_search_row_can_set_target_without_adopting_timeline() {
    let mut state = FieldState::default();
    let row = crate::domain::timeleap::ConfirmedTargetSearchRow {
        frame1: 480,
        target_frame: 998,
        drift_frames: -2,
        grace_frames: 2,
        generation_start_frame: Some(900),
    };
    state.confirmed_target_search_form.status =
        ConfirmedTargetSearchStatus::Results(vec![row.clone()]);

    assert!(!state.timeleap_timeline_adopted());
    assert!(state.set_confirmed_target_frame_from_search_row(&row));
    assert_eq!(state.committed_target_frame(), Some(998));
}

#[test]
fn standalone_target_search_snapshot_uses_editable_source_without_adoption() {
    let mut state = FieldState::default();
    state.timeleap_source_inputs_initialized = true;
    state.timeleap_source_seed = "12345678".into();
    state.timeleap_source_starting_frame = "480".into();
    state.timeleap_source_npc_count = "1".into();
    state.timeleap_source_frame1 = "500".into();
    state.target_consumption = "1000".into();
    state
        .commit_target_input()
        .expect("baseline TargetFrame input is valid");
    state.confirmed_target_search_form.initialized = true;
    state.confirmed_target_search_form.drift_range_ticks = "20".into();
    state.confirmed_target_search_form.grace_min_frames = "0".into();
    state.confirmed_target_search_form.status = ConfirmedTargetSearchStatus::Results(vec![
        crate::domain::timeleap::ConfirmedTargetSearchRow {
            frame1: 480,
            target_frame: 998,
            drift_frames: -2,
            grace_frames: 2,
            generation_start_frame: Some(900),
        },
    ]);

    let (_, request, cache) = state
        .begin_confirmed_target_search()
        .expect("explicit inputs do not require an adopted internal Timeline");
    assert!(cache.is_none());
    assert!(
        matches!(
            &state.confirmed_target_search_form.status,
            ConfirmedTargetSearchStatus::Running
        ),
        "an explicit re-search immediately replaces the previous results with Running"
    );
    assert_eq!(request.seed, 0x1234_5678);
    assert_eq!(request.confirmed_frame1, 500);
    assert_eq!(request.source_config.as_ref().unwrap().start_frame, 480);
    assert_eq!(request.source_config.as_ref().unwrap().models, 2);
    assert_eq!(
        state
            .confirmed_target_search_form
            .result_generation_start_frame,
        None,
        "starting a request keeps the last completed result snapshot untouched"
    );
    assert_eq!(
        state
            .confirmed_target_search_form
            .result_drift_search_generation_start_frame,
        None
    );
}

#[test]
fn standalone_target_search_snapshots_shared_delay_and_courier_no_dex() {
    let mut state = FieldState::default();
    state.timeleap_source_inputs_initialized = true;
    state.timeleap_source_seed = "12345678".into();
    state.timeleap_source_starting_frame = "480".into();
    state.timeleap_source_npc_count = "1".into();
    state.timeleap_source_frame1 = "500".into();
    state.target_consumption = "1000".into();
    state
        .commit_target_input()
        .expect("baseline TargetFrame input is valid");
    state.timeleap_mode = TimeleapMode::Courier;
    state.timeleap_consider_delay_enabled = true;
    state.timeleap_consider_delay_frames = "20".into();
    state.timeleap_no_dex = true;

    let (_, request, _) = state
        .begin_confirmed_target_search()
        .expect("shared delay and courier options should be valid search inputs");
    let delay = request
        .consider_delay
        .as_ref()
        .expect("Consider Delay should be included in the request snapshot");
    assert_eq!(delay.delay_frames, 20);
    assert!(delay.courier.no_dex);
    assert!(request.source_config.as_ref().unwrap().courier.no_dex);
}

#[test]
fn explicit_target_search_cancel_restores_results_and_rejects_late_worker_output() {
    let mut state = confirmed_target_search_state();
    let row = crate::domain::timeleap::ConfirmedTargetSearchRow {
        frame1: 480,
        target_frame: 998,
        drift_frames: -2,
        grace_frames: 2,
        generation_start_frame: Some(900),
    };
    state.confirmed_target_search_form.status =
        ConfirmedTargetSearchStatus::Results(vec![row.clone()]);
    state
        .confirmed_target_search_form
        .result_generation_start_frame = Some(900);
    state
        .confirmed_target_search_form
        .result_drift_search_generation_start_frame = Some(880);
    state
        .confirmed_target_search_form
        .result_drift_marker_boundary = Some(1);

    let (generation, request, _) = state
        .begin_confirmed_target_search()
        .expect("re-search starts from the existing result state");
    assert!(state.confirmed_target_search_is_running());

    state.cancel_confirmed_target_search();

    assert!(!state.confirmed_target_search_is_running());
    assert!(matches!(
        &state.confirmed_target_search_form.status,
        ConfirmedTargetSearchStatus::Results(rows) if rows == &[row]
    ));
    assert_eq!(
        state
            .confirmed_target_search_form
            .result_generation_start_frame,
        Some(900)
    );
    assert_eq!(
        state
            .confirmed_target_search_form
            .result_drift_search_generation_start_frame,
        Some(880)
    );
    assert_eq!(
        state
            .confirmed_target_search_form
            .result_drift_marker_boundary,
        Some(1)
    );
    assert!(!state.apply_confirmed_target_search_result(
        generation,
        &request,
        Ok(crate::domain::timeleap::ConfirmedTargetSearchResult::from_rows(Vec::new(), None,)),
    ));

    state.confirmed_target_search_form.status = ConfirmedTargetSearchStatus::Idle;
    let (generation, request, _) = state
        .begin_confirmed_target_search()
        .expect("another search can start after cancellation");
    state.cancel_confirmed_target_search();
    assert!(matches!(
        state.confirmed_target_search_form.status,
        ConfirmedTargetSearchStatus::Idle
    ));
    assert!(!state.apply_confirmed_target_search_result(
        generation,
        &request,
        Ok(crate::domain::timeleap::ConfirmedTargetSearchResult::from_rows(Vec::new(), None,)),
    ));
}

#[test]
fn failed_target_worker_closes_its_request_and_allows_retry() {
    let mut state = confirmed_target_search_state();
    state
        .begin_confirmed_target_search()
        .expect("first TargetFrame search starts");

    state.fail_confirmed_target_search("worker failed".into());

    assert!(!state.confirmed_target_search_is_running());
    assert!(matches!(
        &state.confirmed_target_search_form.status,
        ConfirmedTargetSearchStatus::Failed(message) if message == "worker failed"
    ));
    state
        .begin_confirmed_target_search()
        .expect("retry should be possible after worker failure");
    assert!(state.confirmed_target_search_is_running());
}

#[test]
fn confirmed_target_drift_search_snapshot_requires_a_valid_frame_only_when_enabled() {
    let mut state = confirmed_target_search_state();
    state.timeleap_generation_start_frame = "900".into();
    state
        .confirmed_target_search_form
        .drift_search_generation_start_frame = "1012".into();
    let (_, request, _) = state
        .begin_confirmed_target_search()
        .expect("disabled optional drift search does not require its input");
    assert_eq!(request.drift_search_generation_start_frame, None);

    state.confirmed_target_search_form.drift_search_enabled = true;
    let (_, request, _) = state
        .begin_confirmed_target_search()
        .expect("enabled drift search snapshots its actual generation frame");
    assert_eq!(request.drift_search_generation_start_frame, Some(1012));

    state
        .confirmed_target_search_form
        .drift_search_generation_start_frame
        .clear();
    assert!(state.begin_confirmed_target_search().is_err());
    state
        .confirmed_target_search_form
        .drift_search_generation_start_frame = (crate::domain::rng::MAX_SFMT_FRAME + 1).to_string();
    assert!(state.begin_confirmed_target_search().is_err());
}
