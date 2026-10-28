use super::*;

#[test]
fn resolved_timeline_cache_picks_the_first_safe_frame_after_adoption() {
    let cursor = TimelineCursor {
        status: crate::domain::rng::ModelStatus { remain: vec![2, 2] },
        stream_cursor: 100,
        consumption: 100,
        tick: 0,
        timeleap_runtime: Some(TimeleapContinuation {
            phase: 0,
            fidget_cd: -1,
            is_boy: true,
            raining: false,
        }),
    };
    let identity = TimelineIdentity {
        seed: 1,
        start_consumption: 100,
        first_wait_frame: 0,
        last_wait_frame: 0,
        first_consumption: 100,
        last_consumption: 100,
        models: 2,
        observed_model: 0,
        intervals: Vec::new(),
        target_consumptions: Vec::new(),
    };
    let mut cache =
        ResolvedTimelineCache::new(identity, cursor, crate::domain::rng::Sfmt::new(1), 0);

    assert_eq!(cache.safe_frame, None);
    assert_eq!(cache.append_chunk(1), 1);
    // remain 2 -> 1 is safe without consuming an SFMT output, so the frame stays 100.
    assert_eq!(cache.safe_frame, Some(100));

    let mut immediately_safe = cache.cursor.clone();
    immediately_safe.status.remain = vec![0, 1];
    assert!(is_safe_frame_cursor(&immediately_safe));
    immediately_safe.status.remain[1] = -5;
    assert!(!is_safe_frame_cursor(&immediately_safe));
}

#[test]
fn starting_frame_display_keeps_original_start_after_timeleap_adoption() {
    let result = FieldResult {
        start_consumption: 480,
        first_wait_frame: 0,
        last_wait_frame: 0,
        first_consumption: 480,
        last_consumption: 480,
        intervals: Vec::new(),
        target_consumptions: Vec::new(),
        next_blink_wait_frame: 0,
        target_wait_frame: Some(100),
        target_exact: Some(true),
        target_frame_before: Some(100),
        target_frame_after: Some(100),
        observed_model: 0,
        models: 2,
        timeline: vec![FieldTimelineRow {
            frame: 480,
            timeline_frame: 0,
            duration_frames: 0,
        }],
        candidate_blinks: Vec::new(),
        target_blink_count: Some(1),
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: None,
    };
    let mut state = FieldState::default();
    assert_eq!(state.starting_frame_for_display(&result), Some(480));

    state.timeleap_enabled = true;
    assert_eq!(state.starting_frame_for_display(&result), Some(480));
    state.timeleap_original_starting_frame = Some(480);
    state.timeleap_timeline_anchor = Some(Instant::now());
    assert_eq!(state.starting_frame_for_display(&result), Some(480));
    state.timeleap_stage = TimeleapStage::Succeeded;
    state.timeleap_safe_frame_display = Some(512);
    assert_eq!(state.starting_frame_for_display(&result), Some(480));
    state.timeleap_stage = TimeleapStage::Recheck;
    assert_eq!(state.starting_frame_for_display(&result), Some(480));
    // The value used to drive the adopted Timeline remains untouched.
    assert_eq!(result.start_consumption, 480);
}

#[test]
fn completed_empty_timeleap_search_is_distinct_from_not_searched() {
    let mut state = FieldState::default();
    assert!(!state.timeleap_search_completed_without_candidates());

    state.begin_timeleap_candidate_search();
    assert!(!state.timeleap_search_completed_without_candidates());

    state.apply_timeleap_candidate_groups(Vec::new());
    assert!(state.timeleap_search_completed_without_candidates());

    state.clear_timeleap_candidates();
    assert!(!state.timeleap_search_completed_without_candidates());
}

#[test]
fn enabling_timeleap_or_opening_search_does_not_auto_start_candidate_search() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_search_window_open = true;

    assert_eq!(
        state.timeleap_search_status,
        TimeleapSearchStatus::NotRequested
    );
    assert!(!state.timeleap_candidate_search_started);
}

#[test]
fn cancelling_candidate_search_resets_only_its_search_lifecycle() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_locked = true;
    state.timeleap_stage = TimeleapStage::Succeeded;
    state.timeleap_frame1_input = "1234".into();
    state.timeleap_frame1_wait_seconds = 12.5;
    state.begin_timeleap_candidate_search();
    assert_eq!(state.timeleap_search_status, TimeleapSearchStatus::Running);

    state.cancel_timeleap_candidate_search();

    assert_eq!(
        state.timeleap_search_status,
        TimeleapSearchStatus::NotRequested
    );
    assert!(!state.timeleap_candidate_search_started);
    assert!(state.timeleap_candidate_search_key.is_none());
    assert!(state.timeleap_candidate_search_started_at.is_none());
    assert!(state.timeleap_enabled);
    assert!(state.timeleap_locked);
    assert_eq!(state.timeleap_stage, TimeleapStage::Succeeded);
    assert_eq!(state.timeleap_frame1_input, "1234");
    assert_eq!(state.timeleap_frame1_wait_seconds, 12.5);
}

#[test]
fn timeleap_candidate_search_uses_explicit_source_inputs_without_a_live_timeline() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_source_inputs_initialized = true;
    state.timeleap_source_seed = "12345678".into();
    state.timeleap_source_starting_frame = "480".into();
    state.timeleap_source_npc_count = "1".into();
    state.timeleap_search_category = crate::domain::fixed_rng_catalog::CATEGORIES
        .iter()
        .position(|category| category.name == "Poke Pelago")
        .expect("カタログにTimeleap方式未対応カテゴリが必要です");
    state.timeleap_mode = TimeleapMode::Dialogue;
    state.target_consumption = "1000".into();

    assert!(state.timeleap_candidate_search_inputs_ready(60.0));
    state.timeleap_locked = true;
    state.timeleap_stage = TimeleapStage::Succeeded;
    state.timeleap_timeline_anchor = Some(Instant::now());
    assert!(
        state.timeleap_candidate_search_inputs_ready(60.0),
        "the post-Frame1 search action remains available while setup is locked"
    );
    state.timeleap_source_npc_count = String::new();
    assert!(!state.timeleap_candidate_search_inputs_ready(60.0));
    state.timeleap_source_npc_count = "1".into();
    state.timeleap_generation_start_frame.clear();
    assert!(!state.timeleap_candidate_search_inputs_ready(60.0));
}

#[test]
fn candidate_search_builds_its_start_from_shared_inputs() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_consider_delay_enabled = false;
    state.timeleap_source_inputs_initialized = true;
    state.timeleap_source_seed = "12345678".into();
    state.timeleap_source_starting_frame = "480".into();
    state.timeleap_source_npc_count = "3".into();
    state.timeleap_generation_start_frame = "10000".into();

    let (key, cache) = state
        .prepare_timeleap_candidate_search(crate::domain::timing::DEFAULT_FPS)
        .expect("standalone source inputs should prepare a search");
    assert!(cache.is_none());
    assert_eq!(key.config.seed, 0x1234_5678);
    assert_eq!(key.config.start_frame, 480);
    assert_eq!(key.config.models, 4);
    assert_eq!(key.timeline_start.frame, 480);
    assert_eq!(key.timeline_start.sfmt_position, 480);
    assert_eq!(key.timeline_start.tick, 0);
    assert_eq!(key.current_tick, 0);
}

#[test]
fn candidate_search_upper_seconds_bound_includes_the_full_display_bucket() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_consider_delay_enabled = false;
    state.timeleap_source_inputs_initialized = true;
    state.timeleap_source_seed = "b248ea3b".into();
    state.timeleap_source_starting_frame = "490".into();
    state.timeleap_source_npc_count = "15".into();
    state.timeleap_generation_start_frame = "69846".into();
    state.timeleap_seconds_min = "30".into();
    state.timeleap_seconds_max = "109".into();

    let (key, _) = state
        .prepare_timeleap_candidate_search(crate::domain::timing::DEFAULT_FPS)
        .expect("the displayed seconds range should prepare");

    assert_eq!(key.config.min_frame1_elapsed_ticks, 900);
    assert_eq!(key.config.max_frame1_elapsed_ticks, 3_299);
}

#[test]
fn candidate_search_frame1_mode_uses_absolute_frame_bounds() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_consider_delay_enabled = false;
    state.timeleap_filter_mode = 1;
    state.timeleap_source_inputs_initialized = true;
    state.timeleap_source_seed = "b248ea3b".into();
    state.timeleap_source_starting_frame = "490".into();
    state.timeleap_source_npc_count = "15".into();
    state.timeleap_generation_start_frame = "69846".into();
    state.timeleap_seconds_min = "30".into();
    state.timeleap_seconds_max = "120".into();
    state.timeleap_frame1_min = "10000".into();
    state.timeleap_frame1_max = "12000".into();

    let (key, _) = state
        .prepare_timeleap_candidate_search(crate::domain::timing::DEFAULT_FPS)
        .expect("Frame1 bounds should prepare a candidate search");

    assert_eq!(key.config.min_frame1_elapsed_ticks, 0);
    assert_eq!(key.config.max_frame1_elapsed_ticks, 5_756);
    assert_eq!(key.frame1_filter, Some((10_000, 12_000)));
}

#[test]
fn seconds_and_frame1_filter_ranges_remain_independent_across_mode_changes() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_consider_delay_enabled = false;
    state.timeleap_source_inputs_initialized = true;
    state.timeleap_source_seed = "b248ea3b".into();
    state.timeleap_source_starting_frame = "490".into();
    state.timeleap_source_npc_count = "15".into();
    state.timeleap_generation_start_frame = "69846".into();
    state.timeleap_seconds_min = "30".into();
    state.timeleap_seconds_max = "120".into();
    state.timeleap_frame1_min = "10000".into();
    state.timeleap_frame1_max = "12000".into();
    state.timeleap_filter_mode = 1;
    let (frame1_key, _) = state
        .prepare_timeleap_candidate_search(crate::domain::timing::DEFAULT_FPS)
        .unwrap();
    assert_eq!(frame1_key.frame1_filter, Some((10_000, 12_000)));
    state.timeleap_filter_mode = 0;
    let (seconds_key, _) = state
        .prepare_timeleap_candidate_search(crate::domain::timing::DEFAULT_FPS)
        .unwrap();
    assert_eq!(seconds_key.config.min_frame1_elapsed_ticks, 900);
    assert_eq!(seconds_key.config.max_frame1_elapsed_ticks, 3_629);
    assert_eq!(state.timeleap_seconds_min, "30");
    assert_eq!(state.timeleap_seconds_max, "120");
    assert_eq!(state.timeleap_frame1_min, "10000");
    assert_eq!(state.timeleap_frame1_max, "12000");
}

#[test]
fn restored_timeleap_source_inputs_skip_first_open_initialization() {
    let mut state = FieldState::default();
    state.timeleap_source_inputs_initialized = true;
    state.timeleap_source_starting_frame = "490".into();
    state.timeleap_source_npc_count = "15".into();

    state.initialize_timeleap_source_inputs();

    assert_eq!(state.timeleap_source_starting_frame, "490");
    assert_eq!(state.timeleap_source_npc_count, "15");
}

#[test]
fn reset_timeleap_source_inputs_is_blank_without_an_internal_timeline() {
    let mut state = FieldState::default();
    state.seed.clear();
    state.timeleap_frame1_input.clear();
    state.timeleap_source_inputs_initialized = true;
    state.timeleap_source_seed = "12345678".into();
    state.timeleap_source_starting_frame = "480".into();
    state.timeleap_source_npc_count = "3".into();
    state.timeleap_source_frame1 = "500".into();

    state.reset_timeleap_source_inputs();

    assert!(state.timeleap_source_seed.is_empty());
    assert!(state.timeleap_source_starting_frame.is_empty());
    assert!(state.timeleap_source_npc_count.is_empty());
    assert!(state.timeleap_source_frame1.is_empty());
}

#[test]
fn reset_timeleap_source_inputs_uses_main_seed_without_an_internal_timeline() {
    let mut state = FieldState::default();
    state.seed = "ABCDEF12".into();
    state.timeleap_source_inputs_initialized = true;
    state.timeleap_source_seed = "12345678".into();

    state.reset_timeleap_source_inputs();

    assert_eq!(state.timeleap_source_seed, "ABCDEF12");
    assert!(state.timeleap_source_starting_frame.is_empty());
    assert!(state.timeleap_source_npc_count.is_empty());
}

#[test]
fn timeleap_source_seed_syncs_only_when_main_seed_is_editable() {
    let mut state = FieldState::default();
    state.seed = "AAAAAAAA".into();
    state.timeleap_source_inputs_initialized = true;

    state.set_timeleap_source_seed("BBBBBBBB".into(), false);
    assert_eq!(state.timeleap_source_seed, "BBBBBBBB");
    assert_eq!(state.seed, "AAAAAAAA");

    state.set_timeleap_source_seed("CCCCCCCC".into(), true);
    assert_eq!(state.timeleap_source_seed, "CCCCCCCC");
    assert_eq!(state.seed, "CCCCCCCC");
}

#[test]
fn main_seed_changes_sync_to_initialized_timeleap_source() {
    let mut state = FieldState::default();
    state.timeleap_source_inputs_initialized = true;
    state.timeleap_source_seed = "AAAAAAAA".into();
    state.seed = "BBBBBBBB".into();

    state.sync_timeleap_source_seed_from_main();

    assert_eq!(state.timeleap_source_seed, "BBBBBBBB");
}

#[test]
fn restored_main_seed_initializes_timeleap_source_even_when_inputs_are_initialized() {
    let mut state = FieldState::default();
    state.seed = "B248EA3B".into();
    state.timeleap_source_inputs_initialized = true;
    state.timeleap_source_seed.clear();

    state.restore_timeleap_source_seed_from_main();

    assert_eq!(state.timeleap_source_seed, "B248EA3B");
}

#[test]
fn timeleap_frame1_sync_obeys_main_editability_and_restore_priority() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_source_inputs_initialized = true;
    state.timeleap_frame1_input = "1200".into();
    state.timeleap_source_frame1 = "900".into();
    assert!(state.can_edit_timeleap_frame1_input());

    state.set_timeleap_source_frame1("1300".into(), false);
    assert_eq!(state.timeleap_frame1_input, "1200");
    assert_eq!(state.timeleap_source_frame1, "1300");

    state.set_timeleap_source_frame1("1400".into(), true);
    assert_eq!(state.timeleap_frame1_input, "1400");
    assert_eq!(state.timeleap_source_frame1, "1400");
    state.timeleap_locked = true;
    assert!(!state.can_edit_timeleap_frame1_input());

    state.timeleap_source_frame1 = "800".into();
    state.timeleap_frame1_input = "1500".into();
    state.restore_timeleap_source_frame1_from_main();
    assert_eq!(state.timeleap_source_frame1, "1500");

    state.timeleap_frame1_input = "invalid".into();
    state.timeleap_source_frame1 = "777".into();
    state.restore_timeleap_source_frame1_from_main();
    assert_eq!(state.timeleap_source_frame1, "777");
}

#[test]
fn initializing_and_resetting_timeleap_inputs_prefer_confirmed_actual_frame1() {
    let mut state = FieldState::default();
    state.timeleap_frame1_input = "1234".into();
    state.timeleap_confirmed_frame1 = Some(999);
    state.initialize_timeleap_source_inputs();
    assert_eq!(state.timeleap_source_frame1, "999");

    state.timeleap_source_frame1 = "456".into();
    state.reset_timeleap_source_inputs();
    assert_eq!(state.timeleap_source_frame1, "999");
}

#[test]
fn target_search_results_and_decoration_snapshot_survive_form_edits() {
    let mut state = FieldState::default();
    state.confirmed_target_search_form.generation = 7;
    state
        .confirmed_target_search_form
        .result_generation_start_frame = Some(900);
    state
        .confirmed_target_search_form
        .result_drift_search_generation_start_frame = Some(880);
    let old_rows = vec![crate::domain::timeleap::ConfirmedTargetSearchRow {
        frame1: 480,
        target_frame: 998,
        drift_frames: -2,
        grace_frames: 2,
        generation_start_frame: Some(900),
    }];
    state.confirmed_target_search_form.status =
        ConfirmedTargetSearchStatus::Results(old_rows.clone());

    state.timeleap_generation_start_frame = "1200".into();
    state
        .confirmed_target_search_form
        .drift_search_generation_start_frame = "1100".into();
    state.confirmed_target_search_form.drift_search_enabled = false;

    assert_eq!(state.confirmed_target_search_form.generation, 7);
    assert_eq!(state.timeleap_generation_start_frame, "1200");
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
    assert!(matches!(
        &state.confirmed_target_search_form.status,
        ConfirmedTargetSearchStatus::Results(rows) if rows == &old_rows
    ));
}

#[test]
fn main_timeline_reset_preserves_target_search_results_and_invalidates_only_worker() {
    let mut state = FieldState {
        seed: "A02D5C73".into(),
        timeleap_enabled: true,
        timeleap_stage: TimeleapStage::Succeeded,
        timeleap_source_inputs_initialized: true,
        timeleap_source_seed: "A02D5C73".into(),
        timeleap_source_starting_frame: "480".into(),
        timeleap_source_npc_count: "1".into(),
        timeleap_source_frame1: "480".into(),
        timeleap_confirmed_frame1: Some(480),
        timeleap_generation_start_frame: "900".into(),
        target_consumption: "1000".into(),
        ..FieldState::default()
    };
    state
        .commit_target_input()
        .expect("valid baseline TargetFrame");
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

    let (stale_generation, stale_request, _) = state
        .begin_confirmed_target_search()
        .expect("a valid current Timeline can start a search");
    assert!(state.confirmed_target_search_is_running());
    let (generation, request, _) = state
        .begin_confirmed_target_search()
        .expect("a running re-search can be replaced without losing its prior result");
    assert!(!state.apply_confirmed_target_search_result(
        stale_generation,
        &stale_request,
        Ok(crate::domain::timeleap::ConfirmedTargetSearchResult::from_rows(Vec::new(), None,)),
    ));
    state.clear_timeleap_candidates();

    assert!(!state.confirmed_target_search_is_running());
    assert!(!state.apply_confirmed_target_search_result(
        generation,
        &request,
        Ok(crate::domain::timeleap::ConfirmedTargetSearchResult::from_rows(Vec::new(), None,)),
    ));
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
}

#[test]
fn cancelled_target_search_restores_previous_results_but_initial_cancel_returns_idle() {
    let mut state = confirmed_target_search_state();
    let (initial_generation, initial_request, _) = state
        .begin_confirmed_target_search()
        .expect("initial search can start");
    assert!(!state.apply_confirmed_target_search_result(
        initial_generation,
        &initial_request,
        Err(crate::domain::timeleap::TimeleapSearchFailure::Cancelled),
    ));
    assert!(matches!(
        &state.confirmed_target_search_form.status,
        ConfirmedTargetSearchStatus::Idle
    ));
    assert!(!state.confirmed_target_search_is_running());

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

    let (stale_generation, stale_request, _) = state
        .begin_confirmed_target_search()
        .expect("first re-search can start");
    let (generation, request, _) = state
        .begin_confirmed_target_search()
        .expect("replacing a running request preserves the original completed result");
    assert!(!state.apply_confirmed_target_search_result(
        stale_generation,
        &stale_request,
        Ok(crate::domain::timeleap::ConfirmedTargetSearchResult::from_rows(Vec::new(), None,)),
    ));
    assert!(!state.apply_confirmed_target_search_result(
        generation,
        &request,
        Err(crate::domain::timeleap::TimeleapSearchFailure::Cancelled),
    ));

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
    assert!(!state.confirmed_target_search_is_running());
}

#[test]
fn confirmed_timeline_overwrites_source_inputs_but_keeps_frame1_search_value() {
    let mut state = FieldState::default();
    state.seed = "ABCDEF12".into();
    state.timeleap_enabled = true;
    state.timeleap_source_inputs_initialized = true;
    state.timeleap_source_seed = "11111111".into();
    state.timeleap_source_starting_frame = "111".into();
    state.timeleap_source_npc_count = "8".into();
    state.timeleap_source_frame1 = "777".into();
    state.results.push(FieldResult {
        start_consumption: 512,
        first_wait_frame: 0,
        last_wait_frame: 0,
        first_consumption: 512,
        last_consumption: 512,
        intervals: Vec::new(),
        target_consumptions: Vec::new(),
        next_blink_wait_frame: 0,
        target_wait_frame: None,
        target_exact: None,
        target_frame_before: None,
        target_frame_after: None,
        target_blink_count: None,
        observed_model: 0,
        models: 4,
        timeline: vec![FieldTimelineRow {
            frame: 512,
            timeline_frame: 0,
            duration_frames: 0,
        }],
        candidate_blinks: Vec::new(),
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: None,
    });

    state.sync_timeleap_source_inputs_from_internal_timeline();
    state.mark_timeline_ready();

    assert_eq!(state.timeleap_source_seed, "ABCDEF12");
    assert_eq!(state.timeleap_source_starting_frame, "512");
    assert_eq!(state.timeleap_source_npc_count, "3");
    assert_eq!(state.timeleap_source_frame1, "777");
    assert_eq!(state.seed, "ABCDEF12");

    state.timeleap_source_starting_frame = "900".into();
    state.timeleap_source_npc_count = "12".into();
    state.mark_timeline_ready();
    assert_eq!(state.timeleap_source_starting_frame, "900");
    assert_eq!(state.timeleap_source_npc_count, "12");
}

#[test]
fn unresolved_timeline_does_not_overwrite_source_inputs() {
    let mut state = FieldState::default();
    state.timeleap_source_inputs_initialized = true;
    state.timeleap_source_seed = "11111111".into();
    state.timeleap_source_starting_frame = "111".into();
    state.timeleap_source_npc_count = "8".into();
    state.timeleap_source_frame1 = "777".into();
    state.results.resize_with(2, || FieldResult {
        start_consumption: 512,
        first_wait_frame: 0,
        last_wait_frame: 0,
        first_consumption: 512,
        last_consumption: 512,
        intervals: Vec::new(),
        target_consumptions: Vec::new(),
        next_blink_wait_frame: 0,
        target_wait_frame: None,
        target_exact: None,
        target_frame_before: None,
        target_frame_after: None,
        target_blink_count: None,
        observed_model: 0,
        models: 4,
        timeline: Vec::new(),
        candidate_blinks: Vec::new(),
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: None,
    });

    state.sync_timeleap_source_inputs_from_internal_timeline();
    state.mark_timeline_ready();

    assert_eq!(state.timeleap_source_seed, "11111111");
    assert_eq!(state.timeleap_source_starting_frame, "111");
    assert_eq!(state.timeleap_source_npc_count, "8");
    assert_eq!(state.timeleap_source_frame1, "777");
}

#[test]
fn first_safe_frame_after_adoption_does_not_overwrite_original_starting_frame() {
    let mut state = FieldState::default();
    state.seed = "ABCDEF12".into();
    state.timeleap_enabled = true;
    state.timeleap_locked = true;
    state.timeleap_stage = TimeleapStage::Assumed;
    state.timeleap_timeline_anchor = Some(Instant::now());
    state.timeleap_original_starting_frame = Some(111);
    state.timeleap_source_inputs_initialized = true;
    state.timeleap_source_seed = "11111111".into();
    state.timeleap_source_starting_frame = "111".into();
    state.timeleap_source_npc_count = "8".into();
    state.timeleap_source_frame1 = "777".into();
    state.results.push(FieldResult {
        start_consumption: 512,
        first_wait_frame: 0,
        last_wait_frame: 0,
        first_consumption: 512,
        last_consumption: 512,
        intervals: Vec::new(),
        target_consumptions: Vec::new(),
        next_blink_wait_frame: 0,
        target_wait_frame: None,
        target_exact: None,
        target_frame_before: None,
        target_frame_after: None,
        target_blink_count: None,
        observed_model: 0,
        models: 4,
        timeline: vec![FieldTimelineRow {
            frame: 512,
            timeline_frame: 0,
            duration_frames: 0,
        }],
        candidate_blinks: Vec::new(),
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: None,
    });
    let extension = state.timeline_extension_result_for_test(Some(640));

    assert_eq!(state.apply_timeline_extension(extension), Some(true));

    assert_eq!(state.timeleap_source_seed, "ABCDEF12");
    assert_eq!(state.timeleap_source_starting_frame, "111");
    assert_eq!(state.timeleap_source_npc_count, "3");
    assert_eq!(state.timeleap_source_frame1, "777");
}

#[test]
fn timeline_extension_result_with_changed_search_identity_is_ignored() {
    let mut state = FieldState::default();
    state.seed = "ABCDEF12".into();
    state.results.push(FieldResult {
        start_consumption: 512,
        first_wait_frame: 20,
        last_wait_frame: 44,
        first_consumption: 512,
        last_consumption: 520,
        intervals: vec![24],
        target_consumptions: vec![512, 520],
        next_blink_wait_frame: 0,
        target_wait_frame: None,
        target_exact: None,
        target_frame_before: None,
        target_frame_after: None,
        observed_model: 0,
        models: 3,
        timeline: vec![FieldTimelineRow {
            frame: 520,
            timeline_frame: 44,
            duration_frames: 0,
        }],
        candidate_blinks: Vec::new(),
        target_blink_count: None,
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: None,
    });
    let mut extension = state.timeline_extension_result_for_test(Some(640));
    extension.identity.intervals[0] += 2;

    assert!(state.apply_timeline_extension(extension).is_none());

    assert_eq!(state.results[0].intervals, vec![24]);
    assert!(state.timeleap_safe_frame_display.is_none());
}

#[test]
fn candidate_seconds_use_live_position_only_for_matching_running_timeline() {
    let mut state = FieldState::default();
    state.seed = "12345678".into();
    state.timeleap_source_inputs_initialized = true;
    state.timeleap_source_seed = "12345678".into();
    state.timeleap_source_starting_frame = "480".into();
    state.timeleap_source_npc_count = "1".into();
    state.timeleap_enabled = true;
    state.timeleap_generation_start_frame = "1000".into();
    state.results.push(FieldResult {
        start_consumption: 480,
        first_wait_frame: 0,
        last_wait_frame: 100,
        first_consumption: 480,
        last_consumption: 480,
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
    });
    state.observed.push(Instant::now());
    let now = Instant::now();
    state.timer_start = Some(now);
    state.timeleap_candidate_search_started_at = Some(now);
    assert_eq!(
        state.starting_frame_for_display(&state.results[0]),
        Some(480)
    );
    assert!(state.timeleap_source_matches_running_timeline(60.0));
    let (search_key, _) = state
        .prepare_timeleap_candidate_search(60.0)
        .expect("matching running Timeline should seed search at its live position");
    assert_eq!(search_key.current_tick, 50);
    // The live Timeline is already at absolute tick 50 (wait frame 100),
    // but candidate ticks are relative to this search snapshot.
    let live_seconds = state.timeleap_candidate_seconds_remaining(80, 60.0, now);
    assert_eq!(
        live_seconds,
        crate::domain::timing::blink_ticks_to_seconds(80, 60.0).floor() as i64
    );
    let live_seconds_after_one_second =
        state.timeleap_candidate_seconds_remaining(80, 60.0, now + Duration::from_secs(1));
    assert_eq!(live_seconds_after_one_second, 1);

    state.timer_start = None;
    let static_seconds = state.timeleap_candidate_seconds_remaining(80, 60.0, now);
    assert_eq!(
        static_seconds,
        crate::domain::timing::blink_ticks_to_seconds(80, 60.0).floor() as i64
    );

    state.timeleap_source_starting_frame = "482".into();
    state.timer_start = Some(now);
    assert!(!state.timeleap_source_matches_running_timeline(60.0));
    let mismatched_seconds = state.timeleap_candidate_seconds_remaining(80, 60.0, now);
    assert_eq!(mismatched_seconds, static_seconds);

    state.timeleap_source_starting_frame = "480".into();
    state.timeleap_source_seed = "87654321".into();
    assert!(!state.timeleap_source_matches_running_timeline(60.0));
    assert_eq!(
        state.timeleap_candidate_seconds_remaining(80, 60.0, now),
        static_seconds
    );

    state.timeleap_source_seed = "12345678".into();
    state.timeleap_source_npc_count = "2".into();
    assert!(!state.timeleap_source_matches_running_timeline(60.0));
    assert_eq!(
        state.timeleap_candidate_seconds_remaining(80, 60.0, now),
        static_seconds
    );
}

#[test]
fn timeleap_method_choices_round_trip_and_default_to_male_menu_close() {
    for method in [
        TimeleapMethodSelection::Dialogue,
        TimeleapMethodSelection::MenuBoy,
        TimeleapMethodSelection::MenuGirl,
        TimeleapMethodSelection::Courier,
    ] {
        let (mode, is_boy) = method.state_values();
        assert_eq!(TimeleapMethodSelection::from_state(mode, is_boy), method);
    }

    let state = FieldState::default();
    assert_eq!(
        TimeleapMethodSelection::from_state(state.timeleap_mode, state.timeleap_is_boy),
        TimeleapMethodSelection::MenuBoy
    );
}

#[test]
fn selected_pokemon_preset_uses_configured_gender_and_preserves_manual_unsupported_mode() {
    use crate::domain::fixed_rng_catalog::{FixedRngPreset, PresetMode};

    let menu = FixedRngPreset {
        species_id: 0,
        suffix: "",
        delay_frames: 156,
        mode: PresetMode::Menu,
        delay_type: crate::domain::fixed_rng_catalog::StationaryDelayType::CommonLegendary,
    };
    let dialogue = FixedRngPreset {
        mode: PresetMode::Dialogue,
        ..menu
    };
    let unsupported = FixedRngPreset {
        mode: PresetMode::Unsupported,
        delay_frames: 8,
        ..menu
    };

    let mut state = FieldState::default();
    assert!(state.timeleap_default_is_boy);
    state.timeleap_default_is_boy = false;
    state.timeleap_is_boy = false;
    state.apply_timeleap_preset(&menu);
    assert_eq!(
        (state.timeleap_mode, state.timeleap_is_boy),
        (TimeleapMode::Menu, false)
    );
    assert_eq!(state.timeleap_consider_delay_frames, "156");

    state.timeleap_default_is_boy = true;
    state.apply_timeleap_preset(&menu);
    assert_eq!(
        (state.timeleap_mode, state.timeleap_is_boy),
        (TimeleapMode::Menu, true)
    );

    state.apply_timeleap_preset(&dialogue);
    assert_eq!(state.timeleap_mode, TimeleapMode::Dialogue);

    state.timeleap_mode = TimeleapMode::Courier;
    state.apply_timeleap_preset(&unsupported);
    assert_eq!(state.timeleap_mode, TimeleapMode::Courier);
    assert_eq!(state.timeleap_consider_delay_frames, "8");
}

#[test]
fn consider_delay_generator_comes_from_the_selected_encounter_class() {
    let mut state = FieldState::default();
    state.timeleap_search_category = 2;
    state.timeleap_search_preset = 0;
    state.timeleap_mode = TimeleapMode::Courier;
    assert_eq!(
        state.timeleap_consider_delay_generator(),
        crate::domain::timeleap::ConsiderDelayGenerator::FixedSymbol(
            crate::domain::fixed_rng_catalog::StationaryDelayType::CommonLegendary
        )
    );

    state.timeleap_search_category = crate::domain::fixed_rng_catalog::CATEGORIES.len();
    assert_eq!(
        state.timeleap_consider_delay_generator(),
        crate::domain::timeleap::ConsiderDelayGenerator::Event
    );
}

#[test]
fn candidate_search_completion_does_not_automatically_select_frame1() {
    let mut state = FieldState::default();
    state.timeleap_frame1_input = "12345".into();
    state.timeleap_selected_frame = None;
    state.begin_timeleap_candidate_search();
    state.apply_timeleap_candidate_groups(Vec::new());
    assert_eq!(state.timeleap_frame1_input, "12345");
    assert_eq!(state.timeleap_selected_frame, None);
}

#[test]
fn applying_candidate_results_keeps_manual_frame1_selection() {
    let now = Instant::now();
    let mut state = FieldState::default();
    state.target_consumption = "1000".into();
    state.timeleap_grace_min = "4".into();
    state.seed = "12345678".into();
    state.timeleap_source_inputs_initialized = true;
    state.timeleap_source_seed = "12345678".into();
    state.timeleap_source_starting_frame = "480".into();
    state.timeleap_source_npc_count = "1".into();
    state.results.push(FieldResult {
        start_consumption: 480,
        first_wait_frame: 0,
        last_wait_frame: 100,
        first_consumption: 480,
        last_consumption: 480,
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
    });
    state.observed.push(now);
    state.timer_start = Some(now);
    state.timeleap_candidate_search_key = Some(TimeleapSearchKey {
        config: TimeleapConfig {
            seed: 1,
            start_frame: 0,
            target_frame: 1000,
            models: 1,
            min_frame1_elapsed_ticks: 0,
            max_frame1_elapsed_ticks: 1_000,
            mode: TimeleapMode::Menu,
            is_boy: false,
            handoff_range_frames: 20,
            courier: CourierConfig::default(),
        },
        consider_delay: None,
        grace_min: 4,
        target_grace_min: 0,
        frame1_filter: None,
        timeline_start: crate::domain::timeleap::TimeleapTimelineStart::initial(0, 1),
        current_tick: 0,
    });
    state.timeleap_candidate_search_started_at = Some(now);
    state.timeleap_search_status = TimeleapSearchStatus::Completed;
    let status = crate::domain::timeleap::TimeleapStatus {
        model: crate::domain::rng::ModelStatus::new(1),
        phase: 0,
        fidget_cd: -1,
        is_boy: false,
        raining: false,
    };
    state.timeleap_candidate_groups = vec![
        TimeleapCandidateGroup {
            frame1_start: 100,
            frame1_end: 100,
            frame1_tick_start: 300,
            frame1_tick_end: 300,
            representative_frame1: 100,
            frame2_ticks: 60,
            grace_frames: 4,
            target_frame_start: 100,
            target_frame_end: 100,
            target_tick_start: 360,
            target_tick_end: 360,
            representative_target_frame: 100,
            target_grace_frames: 2,
            candidates: smallvec::smallvec![TimeleapCandidate {
                frame1: 100,
                frame1_tick: 300,
                post_tick: 360,
                grace_frames: 4,
                pre_frame: 100,
                pre_sfmt_position: 100,
                pre_status: status.clone(),
                post_frame: 100,
                post_sfmt_position: 100,
                post_status: status.clone(),
                target_inputs: vec![crate::domain::timeleap::TimeleapTargetInput {
                    frame: 100,
                    tick: 360,
                }]
                .into(),
            }],
        },
        TimeleapCandidateGroup {
            frame1_start: 200,
            frame1_end: 200,
            frame1_tick_start: 120,
            frame1_tick_end: 120,
            representative_frame1: 200,
            frame2_ticks: 90,
            grace_frames: 4,
            target_frame_start: 200,
            target_frame_end: 200,
            target_tick_start: 210,
            target_tick_end: 210,
            representative_target_frame: 200,
            target_grace_frames: 2,
            candidates: smallvec::smallvec![TimeleapCandidate {
                frame1: 200,
                frame1_tick: 120,
                post_tick: 210,
                grace_frames: 4,
                pre_frame: 200,
                pre_sfmt_position: 200,
                pre_status: status.clone(),
                post_frame: 200,
                post_sfmt_position: 200,
                post_status: status,
                target_inputs: vec![crate::domain::timeleap::TimeleapTargetInput {
                    frame: 200,
                    tick: 210,
                }]
                .into(),
            }],
        },
    ];

    let original_frame1 = state.timeleap_frame1_input.clone();
    state.apply_timeleap_candidate_groups(state.timeleap_candidate_groups.clone());
    assert_eq!(state.timeleap_frame1_input, original_frame1);
    assert_eq!(state.timeleap_selected_frame, None);
}

#[test]
fn editing_timeleap_inputs_does_not_invalidate_an_explicit_search_snapshot() {
    let seed = 0x436F_7E6C;
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=1_000,
        1_000,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate cache for candidate-search snapshot");
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_search_category = crate::domain::fixed_rng_catalog::CATEGORIES.len();
    state.timeleap_courier_your_id = true;
    state.timeleap_courier_is_egg = true;
    state.seed = format!("{seed:08X}");
    state.timeleap_source_inputs_initialized = true;
    state.timeleap_source_seed = state.seed.clone();
    state.timeleap_source_starting_frame = "0".into();
    state.timeleap_source_npc_count = "1".into();
    state.range_start = "0".into();
    state.target_consumption = "900".into();
    state.timeleap_target_grace_min = "6".into();
    state.npc_count = "1".into();
    state.observed.push(Instant::now());
    state.results.push(FieldResult {
        start_consumption: 0,
        first_wait_frame: 0,
        last_wait_frame: 0,
        first_consumption: 0,
        last_consumption: 0,
        intervals: Vec::new(),
        target_consumptions: vec![0],
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
        target_cursor: Some(TimelineCursor {
            status: crate::domain::rng::ModelStatus::new(2),
            stream_cursor: 0,
            consumption: 0,
            tick: 0,
            timeleap_runtime: None,
        }),
    });
    state.set_observation_cache(cache);

    let (key, _) = state
        .prepare_timeleap_candidate_search(crate::domain::timing::DEFAULT_FPS)
        .expect("prepare candidate-search snapshot");
    let courier_delay = key.consider_delay.as_ref().unwrap();
    assert_eq!(key.config.target_frame, 10_000);
    assert_eq!(key.target_grace_min, 6);
    assert_eq!(
        courier_delay.generator,
        crate::domain::timeleap::ConsiderDelayGenerator::Event
    );
    assert!(courier_delay.courier.your_id);
    assert!(courier_delay.courier.is_egg);
    state.begin_timeleap_candidate_search();
    state.set_timeleap_candidate_search_key(key.clone());
    assert!(state.timeleap_search_key_matches(&key));

    state.timeleap_generation_start_frame = "900".into();
    assert_eq!(state.timeleap_candidate_search_target_frame(), Some(10_000));
    assert!(state.timeleap_search_key_matches(&key));
}

#[test]
fn courier_delay_policy_uses_edition_and_all_generation_flags_without_toggling_option() {
    use crate::domain::fixed_rng_catalog::GameEdition;

    for edition in GameEdition::ALL {
        let expected_event_delay = edition.courier_event_delay_frames().to_string();
        let mut state = FieldState {
            timeleap_game_edition: edition,
            timeleap_consider_delay_enabled: false,
            ..FieldState::default()
        };
        state.timeleap_search_category = state.timeleap_courier_category();

        let cases = [
            (false, false, false, "0"),
            (false, false, true, "0"),
            (false, true, false, expected_event_delay.as_str()),
            (false, true, true, "0"),
            (true, false, false, expected_event_delay.as_str()),
            (true, false, true, expected_event_delay.as_str()),
            (true, true, false, expected_event_delay.as_str()),
            (true, true, true, expected_event_delay.as_str()),
        ];
        for (no_dex, your_id, is_egg, expected) in cases {
            state.timeleap_no_dex = no_dex;
            state.timeleap_courier_your_id = your_id;
            state.timeleap_courier_is_egg = is_egg;
            state.timeleap_consider_delay_frames = "999".into();
            state.refresh_timeleap_courier_delay();
            assert_eq!(state.timeleap_consider_delay_frames, expected);
            assert!(!state.timeleap_consider_delay_enabled);
        }
    }
}

#[test]
fn changing_edition_maps_a_unique_encounter_and_preserves_courier_selection() {
    use crate::domain::fixed_rng_catalog::{
        categories, find_unique_preset, selected_preset_for, GameEdition,
    };

    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_search_category = 1;
    state.timeleap_search_preset = find_unique_preset(GameEdition::UltraSunUltraMoon, 791, "")
        .expect("USUM Solgaleo preset")
        .1;
    assert!(state.set_timeleap_game_edition(GameEdition::SunMoon));
    assert_eq!(state.timeleap_game_edition, GameEdition::SunMoon);
    let selected = selected_preset_for(
        state.timeleap_game_edition,
        state.timeleap_search_category,
        state.timeleap_search_preset,
    )
    .unwrap();
    assert_eq!(selected.species_id, 791);
    assert_eq!(state.timeleap_consider_delay_frames, "288");

    state.timeleap_search_category = categories(GameEdition::SunMoon).len();
    state.timeleap_no_dex = true;
    assert!(state.set_timeleap_game_edition(GameEdition::UltraSunUltraMoon));
    assert_eq!(
        state.timeleap_search_category,
        categories(GameEdition::UltraSunUltraMoon).len()
    );
    assert_eq!(state.timeleap_mode, TimeleapMode::Courier);
    assert_eq!(state.timeleap_consider_delay_frames, "42");
}

#[test]
fn invalid_edition_selection_falls_back_without_becoming_courier() {
    use crate::domain::fixed_rng_catalog::{normalize_selection, GameEdition};

    assert_eq!(
        normalize_selection(GameEdition::SunMoon, usize::MAX, 99),
        (0, 0)
    );
    assert_eq!(
        normalize_selection(GameEdition::SunMoon, 1, usize::MAX),
        (1, 0)
    );
    assert_eq!(normalize_selection(GameEdition::SunMoon, 8, 99), (8, 0));
}

#[test]
fn switching_to_an_edition_without_the_selected_encounter_uses_its_neutral_preset() {
    use crate::domain::fixed_rng_catalog::GameEdition;

    let mut state = FieldState {
        timeleap_enabled: true,
        timeleap_search_category: 15,
        timeleap_search_preset: 0,
        ..FieldState::default()
    };

    assert!(state.set_timeleap_game_edition(GameEdition::SunMoon));
    assert_eq!(state.timeleap_search_category, 0);
    assert_eq!(state.timeleap_search_preset, 0);
    assert_eq!(state.timeleap_consider_delay_frames, "0");
}

#[test]
fn setting_a_candidate_row_updates_frame1_and_a_input_target_only() {
    let mut state = FieldState::default();
    state.timeleap_generation_start_frame = "123456".into();

    assert!(state.set_timeleap_candidate_inputs(500, 600));

    assert_eq!(state.timeleap_frame1_input, "500");
    assert_eq!(state.target_consumption, "600");
    assert_eq!(state.target_last_committed, "600");
    assert_eq!(state.timeleap_selected_frame, Some(500));
    assert_eq!(state.timeleap_generation_start_frame, "123456");
}

#[test]
fn setting_a_candidate_row_is_rejected_while_locked_or_out_of_range() {
    let mut state = FieldState::default();
    state.timeleap_locked = true;
    assert!(!state.set_timeleap_candidate_inputs(500, 600));
    state.timeleap_locked = false;
    assert!(!state.set_timeleap_candidate_inputs(500, crate::domain::rng::MAX_SFMT_FRAME + 1));
    assert_eq!(state.timeleap_frame1_input, defaults::FIELD_TIMELEAP_FRAME1);
    assert!(state.target_consumption.is_empty());
}

#[test]
fn candidate_seconds_from_start_do_not_drift_with_search_wall_time() {
    let now = Instant::now();
    let mut state = FieldState::default();
    state.timeleap_candidate_search_started_at = Some(now - Duration::from_secs(3));

    assert_eq!(
        state.timeleap_candidate_seconds_remaining(150, 60.0, now),
        5
    );
    assert_eq!(
        state.timeleap_candidate_seconds_remaining(150, 60.0, now + Duration::from_secs(3)),
        5
    );
}

#[test]
fn editing_inputs_keeps_last_timeleap_results_until_explicit_resubmission() {
    let mut state = FieldState::default();
    state.timeleap_candidate_search_started = true;
    state
        .timeleap_candidate_groups
        .push(TimeleapCandidateGroup {
            frame1_start: 100,
            frame1_end: 100,
            frame1_tick_start: 50,
            frame1_tick_end: 50,
            representative_frame1: 100,
            frame2_ticks: 10,
            grace_frames: 0,
            target_frame_start: 100,
            target_frame_end: 100,
            target_tick_start: 60,
            target_tick_end: 60,
            representative_target_frame: 100,
            target_grace_frames: 2,
            candidates: smallvec::SmallVec::new(),
        });
    state.timeleap_search_status = TimeleapSearchStatus::Completed;
    state.timeleap_candidate_search_started_at = Some(Instant::now());

    state.target_consumption = "901".into();

    assert_eq!(state.timeleap_candidate_groups.len(), 1);
    assert_eq!(
        state.timeleap_search_status,
        TimeleapSearchStatus::Completed
    );
    assert!(state.timeleap_candidate_search_started);
    assert_eq!(state.timeleap_candidate_search_target_frame(), None);
    assert!(state.timeleap_candidate_search_started_at.is_some());
}
