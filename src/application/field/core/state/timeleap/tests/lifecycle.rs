use super::*;

#[test]
fn frame1_deadline_applies_offset_but_not_consider_delay() {
    let now = Instant::now();
    let fps = crate::domain::timing::DEFAULT_FPS;
    let mut without_delay = FieldState::default();
    without_delay.timeleap_enabled = true;
    without_delay.adjust = 0;

    let mut with_delay = without_delay.clone();
    with_delay.timeleap_consider_delay_enabled = true;
    with_delay.timeleap_consider_delay_frames = "10".into();
    let mut with_positive_offset = without_delay.clone();
    with_positive_offset.adjust = 20;
    let mut with_negative_offset = without_delay.clone();
    with_negative_offset.adjust = -20;

    assert_eq!(
        without_delay.frame1_deadline(1_000, now, fps),
        with_delay.frame1_deadline(1_000, now, fps),
    );
    let unadjusted_deadline = without_delay.frame1_deadline(1_000, now, fps).0;
    let positive_deadline = with_positive_offset.frame1_deadline(1_000, now, fps).0;
    let negative_deadline = with_negative_offset.frame1_deadline(1_000, now, fps).0;
    let offset_duration =
        Duration::from_secs_f64(crate::domain::timing::game_frames_to_seconds(20.0, fps));
    assert_eq!(
        unadjusted_deadline.duration_since(positive_deadline),
        offset_duration,
    );
    assert_eq!(
        negative_deadline.duration_since(unadjusted_deadline),
        offset_duration,
    );
}

#[test]
fn timeleap_adjustment_is_applied_once_to_observation_countdowns() {
    let now = Instant::now();
    let fps = crate::domain::timing::DEFAULT_FPS;
    for adjust in [-20, 20] {
        let mut state = FieldState::default();
        state.timeleap_enabled = true;
        state.adjust = adjust;
        state.timeleap_timeline_anchor = Some(now);

        let remaining = state
            .remaining_from_observation_at(120, fps, now)
            .expect("positive target wait should be available");
        let expected = crate::domain::timing::game_frames_to_seconds((120 - adjust) as f64, fps);
        assert!((remaining - expected).abs() < f64::EPSILON * 8.0);
    }
}

#[test]
fn timeleap_offset_changes_continue_to_shift_live_timer_and_frame1_deadline() {
    let now = Instant::now();
    let fps = crate::domain::timing::DEFAULT_FPS;
    let one_frame =
        Duration::from_secs_f64(crate::domain::timing::game_frames_to_seconds(1.0, fps));
    let initial_deadline = now + Duration::from_secs(10);
    let mut state = FieldState {
        timeleap_enabled: true,
        timeleap_frame1_deadline: Some(initial_deadline),
        timer_start: Some(now),
        ..FieldState::default()
    };

    state.adjust_by(1, fps);

    assert_eq!(state.adjust, 1);
    assert_eq!(
        state.timeleap_frame1_deadline,
        Some(initial_deadline - one_frame)
    );
    assert_eq!(state.timer_start, Some(now - one_frame));
}

#[test]
fn target_timer_switches_from_frame1_to_target_after_timeline_adoption() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timer_kind = FieldTimerKind::Target;
    assert_eq!(
        state.timeleap_lifecycle_policy().table_mode,
        TimeleapTableMode::Frame1
    );

    state.timeleap_timeline_anchor = Some(Instant::now());
    assert_eq!(
        state.timeleap_lifecycle_policy().table_mode,
        TimeleapTableMode::Target
    );
    assert_eq!(state.timer_kind, FieldTimerKind::Target);
}

#[test]
fn lifecycle_policy_keeps_table_summary_and_lock_coherent_across_phases() {
    let mut state = FieldState::default();
    state.timeleap_frame1_input = "106040".into();
    let disabled = state.timeleap_lifecycle_policy();
    assert_eq!(disabled.phase, TimeleapLifecyclePhase::Disabled);
    assert_eq!(disabled.table_mode, TimeleapTableMode::Target);
    assert_eq!(disabled.frame1_boundary_target, None);
    assert_eq!(disabled.summary_frame1, None);

    state.timeleap_enabled = true;
    state.timer_kind = FieldTimerKind::NextBlink;
    let setup = state.timeleap_lifecycle_policy();
    assert_eq!(setup.phase, TimeleapLifecyclePhase::Frame1Setup);
    assert_eq!(setup.table_mode, TimeleapTableMode::Frame1);
    assert_eq!(setup.frame1_boundary_target, Some(106040));
    assert!(setup.frame1_input_enabled);
    assert!(setup.candidate_search_enabled);

    state.timeleap_frame1_preparing = true;
    assert_eq!(
        state.timeleap_lifecycle_policy().phase,
        TimeleapLifecyclePhase::Frame1Preparing
    );
    assert!(!state.timeleap_lifecycle_policy().frame1_input_enabled);
    state.timeleap_frame1_preparing = false;

    state.timeleap_frame1_reachable = Some(false);
    let unreachable = state.timeleap_lifecycle_policy();
    assert_eq!(unreachable.phase, TimeleapLifecyclePhase::Frame1Unreachable);
    assert_eq!(unreachable.frame1_boundary_target, None);
    state.timeleap_frame1_reachable = None;

    state.timeleap_locked = true;
    state.timeleap_stage = TimeleapStage::Frame1Waiting;
    let waiting = state.timeleap_lifecycle_policy();
    assert_eq!(waiting.phase, TimeleapLifecyclePhase::Frame1Waiting);
    assert!(waiting.configuration_locked);
    assert!(!waiting.frame1_input_enabled);
    assert!(!waiting.candidate_search_enabled);
    assert_eq!(waiting.summary_frame1, None);

    state.timeleap_stage = TimeleapStage::Failed;
    let frame1_failed = state.timeleap_lifecycle_policy();
    assert_eq!(frame1_failed.phase, TimeleapLifecyclePhase::Frame1Failed);
    assert!(frame1_failed.candidate_search_enabled);

    state.timeleap_stage = TimeleapStage::Assumed;
    state.timeleap_selected_frame = Some(106040);
    state.timeleap_timeline_anchor = Some(Instant::now());
    let assumed = state.timeleap_lifecycle_policy();
    assert_eq!(assumed.phase, TimeleapLifecyclePhase::PostTimeleapAssumed);
    assert_eq!(assumed.table_mode, TimeleapTableMode::Target);
    assert!(!assumed.frame1_input_enabled);
    assert!(assumed.candidate_search_enabled);
    assert_eq!(assumed.summary_frame1, Some(106040));

    state.timeleap_stage = TimeleapStage::Recheck;
    assert_eq!(
        state.timeleap_lifecycle_policy().phase,
        TimeleapLifecyclePhase::PostTimeleapRechecking
    );
    assert_eq!(
        state.timeleap_lifecycle_policy().summary_frame1,
        Some(106040)
    );

    state.timeleap_stage = TimeleapStage::Succeeded;
    state.timeleap_confirmed_frame1 = Some(106042);
    let confirmed = state.timeleap_lifecycle_policy();
    assert_eq!(
        confirmed.phase,
        TimeleapLifecyclePhase::PostTimeleapConfirmed
    );
    assert_eq!(confirmed.summary_frame1, Some(106042));

    state.timeleap_stage = TimeleapStage::Failed;
    assert_eq!(
        state.timeleap_lifecycle_policy().phase,
        TimeleapLifecyclePhase::PostTimeleapRecheckFailed
    );
    assert_eq!(
        state.timeleap_lifecycle_policy().summary_frame1,
        Some(106042)
    );
}

#[test]
fn target_passed_drift_check_enters_correction_without_accepting_shift() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_locked = true;
    state.timeleap_stage = TimeleapStage::Succeeded;
    state.timer_start = Some(Instant::now());

    state
        .begin_timeleap_recheck(true, crate::domain::timing::DEFAULT_FPS)
        .expect("passed Target should enter the ordinary correction flow");

    assert!(matches!(state.mode, FieldMode::Correction));
    assert!(state.timer_start.is_none());
    assert!(!state.timeleap_accepts_shift());
}

#[test]
fn stale_target_check_failure_preserves_latest_pending_request() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.target_last_committed = "1200".into();
    state.timeleap_target_check_pending = true;
    let generation = state.timeleap_frame1_preview_generation();

    assert!(!state.fail_timeleap_target_check(generation, 1100));
    assert!(state.timeleap_target_check_pending);
    assert_eq!(state.timeleap_target_reachable, None);

    assert!(state.fail_timeleap_target_check(generation, 1200));
    assert!(!state.timeleap_target_check_pending);
    assert_eq!(state.timeleap_target_reachable, None);
}

#[test]
fn stale_frame1_generation_cannot_clear_target_check_pending() {
    let mut state = FieldState {
        timeleap_enabled: true,
        target_last_committed: "1200".into(),
        ..FieldState::default()
    };
    let stale_generation = state.timeleap_frame1_preview_generation();
    state.invalidate_timeleap_frame1_preview();

    assert!(!state.clear_timeleap_target_check_for_frame1_generation(stale_generation));
    assert!(state.timeleap_target_check_is_pending());

    let current_generation = state.timeleap_frame1_preview_generation();
    assert!(state.clear_timeleap_target_check_for_frame1_generation(current_generation));
    assert!(!state.timeleap_target_check_is_pending());
}

#[test]
fn missing_target_check_snapshot_clears_pending_state_in_field_owner() {
    let mut state = FieldState {
        timeleap_target_check_pending: true,
        ..FieldState::default()
    };

    assert!(state.prepare_timeleap_target_check_request().is_none());
    assert!(!state.timeleap_target_check_is_pending());
}

#[test]
fn stale_frame1_preview_panic_cannot_fail_the_current_generation() {
    let mut state = FieldState {
        timeleap_enabled: true,
        target_last_committed: "1200".into(),
        ..FieldState::default()
    };
    let stale_generation = state.timeleap_frame1_preview_generation();
    state.invalidate_timeleap_frame1_preview();

    assert!(!state.fail_timeleap_frame1_preview_generation(stale_generation));
    assert!(state.timeleap_frame1_preview_is_pending());
    assert!(state.timeleap_target_check_is_pending());

    let current_generation = state.timeleap_frame1_preview_generation();
    assert!(state.fail_timeleap_frame1_preview_generation(current_generation));
    assert!(state.timeleap_frame1_preview_failed());
    assert!(!state.timeleap_target_check_is_pending());
}

#[test]
fn frame1_preview_validity_ignores_live_cursor_advancement() {
    let seed = 0x1234_5678;
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    let now = Instant::now();
    state.timer_start = Some(now);
    state.timeleap_frame1_deadline = Some(now + Duration::from_secs(10));
    state.seed = format!("{seed:08X}");
    state.npc_count = "1".into();
    state.timeleap_frame1_input = "1000".into();
    state.target_last_committed = "1200".into();
    state.results.push(FieldResult {
        start_consumption: 480,
        first_wait_frame: 0,
        last_wait_frame: 0,
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
        timeline: vec![FieldTimelineRow {
            frame: 900,
            timeline_frame: 420,
            duration_frames: 2,
        }],
        candidate_blinks: Vec::new(),
        target_blink_count: None,
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: Some(TimelineCursor {
            status: crate::domain::rng::ModelStatus::new(2),
            stream_cursor: 700,
            consumption: 700,
            tick: 350,
            timeleap_runtime: None,
        }),
    });
    let config = TimeleapConfig {
        seed,
        start_frame: 700,
        target_frame: 1000,
        models: 2,
        min_frame1_elapsed_ticks: 0,
        max_frame1_elapsed_ticks: 0,
        mode: TimeleapMode::Menu,
        is_boy: true,
        handoff_range_frames: 98,
        courier: CourierConfig::default(),
    };
    state.timeleap_frame1_preview_state =
        TimeleapFrame1PreviewState::Ready(TimeleapFrame1Preview {
            config,
            frame1: 1000,
            target_frame: Some(1200),
            start: crate::domain::timeleap::TimeleapTimelineStart::initial(700, 2),
            preparation: crate::domain::timeleap::TimeleapFrame1Preparation {
                nominal_frame1: 1000,
                nominal_tick: 150,
                candidates: Vec::new(),
            },
        });
    let generation = state.timeleap_frame1_preview_generation();

    // The Target-facing cursor and TargetFrame may be refreshed while the
    // normal Timeline advances, but Frame1 is still the same requested
    // deadline and the prepared event state remains valid.
    let target_cursor = state.results[0].target_cursor.as_mut().unwrap();
    target_cursor.stream_cursor = 702;
    target_cursor.consumption = 702;
    target_cursor.tick = 351;
    state.target_last_committed = "1300".into();
    assert!(state.timeleap_frame1_preview_matches_current());
    assert!(!state.timeleap_frame1_preview_is_stale());
    assert!(state.has_timeleap_frame1_preview());
    assert!(state.timer_start.is_some());
    assert_eq!(state.timeleap_frame1_preview_generation(), generation);
    state.refresh_timeleap_target_check_target();
    assert_eq!(
        match &state.timeleap_frame1_preview_state {
            TimeleapFrame1PreviewState::Ready(preview) => preview.target_frame,
            _ => panic!("preview remains ready while only Target changes"),
        },
        Some(1300)
    );
    assert!(state.timeleap_target_check_pending);
    assert_eq!(state.timeleap_frame1_preview_generation(), generation);
    for _ in 0..120 {
        state.refresh_timeleap_target_check_target();
        assert!(state.timeleap_frame1_preview_matches_current());
        assert!(!state.needs_timeleap_frame1_preview());
        assert!(state.timeleap_target_check_pending);
    }
    state.timeleap_target_check_pending = false;
    state.timeleap_target_reachable = Some(true);
    state.refresh_timeleap_target_check_target();
    assert!(!state.timeleap_target_check_pending);
    assert_eq!(state.timeleap_target_reachable, Some(true));
    assert_eq!(
        state.timeleap_frame1_remaining_at(now, crate::domain::timing::DEFAULT_FPS),
        Some(10.0)
    );
    assert!(
        state
            .timeleap_frame1_remaining_at(
                now + Duration::from_secs(2),
                crate::domain::timing::DEFAULT_FPS
            )
            .unwrap()
            < 10.0
    );
    assert!(state.timeleap_frame1_preview_matches_current());
    assert!(state.timer_start.is_some());

    state.timeleap_frame1_input = "1002".into();
    assert!(!state.timeleap_frame1_preview_matches_current());
    assert!(state.timeleap_frame1_preview_is_stale());
    assert_eq!(
        state.reconcile_timeleap_frame1_preview(),
        TimeleapFrame1PreviewReconciliation::Restart
    );
    assert!(state.timeleap_frame1_preview_is_pending());
}

#[test]
fn unreachable_frame1_result_is_stable_until_inputs_are_invalidated() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_stage = TimeleapStage::Off;
    state.timeleap_frame1_input = "1000".into();
    state.results.push(FieldResult {
        start_consumption: 480,
        first_wait_frame: 0,
        last_wait_frame: 0,
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
        models: 1,
        timeline: Vec::new(),
        candidate_blinks: Vec::new(),
        target_blink_count: None,
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: None,
    });

    assert_eq!(
        state.reconcile_timeleap_frame1_preview(),
        TimeleapFrame1PreviewReconciliation::Schedule
    );
    assert!(state.timeleap_frame1_preview_is_pending());
    state.timeleap_frame1_deadline = Some(Instant::now() + Duration::from_secs(20));
    state.timeleap_frame1_wait_seconds = 20.0;
    state.set_timeleap_frame1_unreachable();
    assert_eq!(state.timeleap_frame1_deadline, None);
    assert_eq!(state.timeleap_frame1_wait_seconds, 0.0);
    let completed_generation = state.timeleap_frame1_preview_generation();

    for _ in 0..120 {
        assert_eq!(
            state.reconcile_timeleap_frame1_preview(),
            TimeleapFrame1PreviewReconciliation::Stable,
            "the actual per-frame reconciliation path must preserve terminal results"
        );
        assert_eq!(
            state.timeleap_frame1_indicator(),
            Some(TimeleapFrame1Indicator::Unreachable)
        );
        assert!(!state.timeleap_frame1_preview_is_stale());
        assert!(!state.timeleap_frame1_preview_is_pending());
        assert!(!state.needs_timeleap_frame1_preview());
        assert_eq!(
            state.timeleap_frame1_preview_generation(),
            completed_generation,
            "redraws must not invalidate or restart a definitive unreachable result"
        );
    }

    state.invalidate_timeleap_frame1_preview();
    assert_eq!(state.timeleap_frame1_indicator(), None);
    assert!(state.timeleap_frame1_preview_is_pending());
    assert_eq!(
        state.timeleap_frame1_preview_generation(),
        completed_generation.wrapping_add(1),
        "an explicit input/timeline invalidation must create a new calculation generation"
    );
    assert!(!state.timeleap_frame1_preview_is_stale());
    assert_eq!(
        state.reconcile_timeleap_frame1_preview(),
        TimeleapFrame1PreviewReconciliation::Stable,
        "Pending is scheduled by the worker runner, not invalidated again by reconciliation"
    );
}

#[test]
fn losing_the_unique_candidate_discards_its_frame1_preview_generation() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.results.push(FieldResult {
        start_consumption: 480,
        first_wait_frame: 0,
        last_wait_frame: 0,
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
        models: 1,
        timeline: Vec::new(),
        candidate_blinks: Vec::new(),
        target_blink_count: None,
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: None,
    });
    state.invalidate_timeleap_frame1_preview();
    state.timeleap_frame1_reachable = Some(true);
    let old_generation = state.timeleap_frame1_preview_generation();

    assert_eq!(state.advance_candidates_by_interval(2), 0);
    assert!(state.results.is_empty());
    assert_eq!(state.timeleap_frame1_reachable, None);
    assert!(!state.timeleap_frame1_preview_is_pending());
    assert_eq!(
        state.timeleap_frame1_preview_generation(),
        old_generation.wrapping_add(1)
    );
    assert_eq!(
        state.reconcile_timeleap_frame1_preview(),
        TimeleapFrame1PreviewReconciliation::Stable,
        "without a unique Timeline, the discarded preview must not be re-requested"
    );
}

#[test]
fn observation_start_and_cancel_clear_stale_timeleap_reachability() {
    for reset in [
        FieldState::start_observation as fn(&mut FieldState),
        FieldState::cancel_observation,
    ] {
        let mut state = FieldState::default();
        state.timeleap_enabled = true;
        state.timer_kind = FieldTimerKind::Target;
        state.invalidate_timeleap_frame1_preview();
        state.timeleap_frame1_reachable = Some(true);
        state.timeleap_target_reachable = Some(false);
        state.timeleap_target_check_pending = true;
        state.timeleap_frame1_deadline = Some(Instant::now() + Duration::from_secs(20));
        state.timeleap_frame1_wait_seconds = 20.0;
        let old_generation = state.timeleap_frame1_preview_generation();

        reset(&mut state);

        assert_eq!(state.timeleap_frame1_indicator(), None);
        assert_eq!(state.target_reachability_for_display(Some(true)), None);
        assert!(!state.timeleap_target_check_pending);
        assert_eq!(state.timeleap_frame1_deadline, None);
        assert_eq!(state.timeleap_frame1_wait_seconds, 0.0);
        assert!(!state.timeleap_frame1_preview_is_pending());
        assert_eq!(
            state.reconcile_timeleap_frame1_preview(),
            TimeleapFrame1PreviewReconciliation::Stable
        );
        assert_eq!(
            state.timeleap_frame1_preview_generation(),
            old_generation.wrapping_add(1)
        );
    }
}

#[test]
fn frame1_preview_terminal_states_do_not_request_duplicate_work() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.results.push(FieldResult {
        start_consumption: 480,
        first_wait_frame: 0,
        last_wait_frame: 0,
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
        models: 1,
        timeline: Vec::new(),
        candidate_blinks: Vec::new(),
        target_blink_count: None,
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: None,
    });

    assert_eq!(
        state.reconcile_timeleap_frame1_preview(),
        TimeleapFrame1PreviewReconciliation::Schedule
    );
    assert!(state.timeleap_frame1_preview_is_pending());
    assert_eq!(
        state.reconcile_timeleap_frame1_preview(),
        TimeleapFrame1PreviewReconciliation::Stable
    );

    // Cancellation returns to Idle so the scheduler may retry; a calculation
    // error is terminal until explicit invalidation.
    state.reset_timeleap_frame1_preview_to_idle();
    assert_eq!(
        state.reconcile_timeleap_frame1_preview(),
        TimeleapFrame1PreviewReconciliation::Schedule
    );
    state.mark_timeleap_frame1_preview_failed();
    for _ in 0..120 {
        assert_eq!(
            state.reconcile_timeleap_frame1_preview(),
            TimeleapFrame1PreviewReconciliation::Stable
        );
        assert!(state.timeleap_frame1_preview_failed());
        assert!(!state.timeleap_frame1_preview_is_pending());
        assert!(!state.timeleap_frame1_preview_is_stale());
    }

    state.invalidate_timeleap_frame1_preview();
    assert!(!state.timeleap_frame1_preview_failed());
    assert!(state.timeleap_frame1_preview_is_pending());
    assert_eq!(
        state.reconcile_timeleap_frame1_preview(),
        TimeleapFrame1PreviewReconciliation::Stable
    );
}

#[test]
fn ready_invalidates_inflight_frame1_preview_before_it_can_replace_target_timer() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.invalidate_timeleap_frame1_preview();
    let stale_generation = state.timeleap_frame1_preview_generation();
    state.begin_timeleap_frame1_preparation();
    assert_ne!(state.timeleap_frame1_preview_generation(), stale_generation);
    assert!(!state.timeleap_frame1_preview_is_pending());
    assert!(!state.fail_timeleap_frame1_preview_generation(stale_generation));

    let now = Instant::now();
    state.timeleap_frame1_preparing = false;
    state.timeleap_stage = TimeleapStage::Assumed;
    state.timeleap_timeline_anchor = Some(now);
    state.timer_kind = FieldTimerKind::Target;
    state.timer_start = Some(now);
    state.timer_seconds = 77.0;
    let config = TimeleapConfig {
        seed: 0x1234_5678,
        start_frame: 480,
        target_frame: 1000,
        models: 2,
        min_frame1_elapsed_ticks: 0,
        max_frame1_elapsed_ticks: 0,
        mode: TimeleapMode::Menu,
        is_boy: true,
        handoff_range_frames: 98,
        courier: CourierConfig::default(),
    };
    state.set_timeleap_frame1_preview(
        stale_generation,
        config,
        1000,
        None,
        crate::domain::timeleap::TimeleapTimelineStart::initial(480, 2),
        now,
        crate::domain::timeleap::TimeleapFrame1Preparation {
            nominal_frame1: 1000,
            nominal_tick: 500,
            candidates: Vec::new(),
        },
        crate::domain::timing::DEFAULT_FPS,
    );
    assert_eq!(state.timer_kind, FieldTimerKind::Target);
    assert_eq!(state.timer_start, Some(now));
    assert_eq!(state.timer_seconds, 77.0);
    assert!(!state.has_timeleap_frame1_preview());
}

#[test]
fn stale_frame1_preview_worker_result_cannot_replace_new_pending_generation() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.invalidate_timeleap_frame1_preview();
    let current_generation = state.timeleap_frame1_preview_generation();
    let config = TimeleapConfig {
        seed: 0x1234_5678,
        start_frame: 480,
        target_frame: 1000,
        models: 2,
        min_frame1_elapsed_ticks: 0,
        max_frame1_elapsed_ticks: 0,
        mode: TimeleapMode::Menu,
        is_boy: true,
        handoff_range_frames: 98,
        courier: CourierConfig::default(),
    };

    state.set_timeleap_frame1_preview(
        current_generation.wrapping_sub(1),
        config,
        1000,
        None,
        crate::domain::timeleap::TimeleapTimelineStart::initial(480, 2),
        Instant::now(),
        crate::domain::timeleap::TimeleapFrame1Preparation {
            nominal_frame1: 1000,
            nominal_tick: 500,
            candidates: Vec::new(),
        },
        crate::domain::timing::DEFAULT_FPS,
    );

    assert!(state.timeleap_frame1_preview_is_pending());
    assert!(!state.has_timeleap_frame1_preview());
    assert_eq!(state.timeleap_frame1_reachable, None);
    assert_eq!(
        state.timeleap_frame1_preview_generation(),
        current_generation
    );
}

#[test]
fn consider_delay_input_uses_requested_bounds_and_preserves_source_negative_preset() {
    let mut state = FieldState::default();
    state.timeleap_consider_delay_frames = "9998".into();
    assert_eq!(state.timeleap_consider_delay_value(), Ok(9998));

    state.timeleap_consider_delay_frames = "9999".into();
    assert!(state.timeleap_consider_delay_value().is_err());

    state.timeleap_search_category = 11;
    state.timeleap_search_preset = 10;
    state.timeleap_consider_delay_frames = "-4".into();
    assert_eq!(state.timeleap_consider_delay_value(), Ok(-4));

    state.timeleap_search_preset = 0;
    assert!(state.timeleap_consider_delay_value().is_err());
}

#[test]
fn timeleap_uses_confirmed_models_for_ranges_and_configured_models_for_manual_frame1() {
    let mut state = FieldState {
        npc_count: "13~18".into(),
        multiple_npc_search: true,
        results: vec![FieldResult {
            start_consumption: 480,
            first_wait_frame: 100,
            last_wait_frame: 200,
            first_consumption: 580,
            last_consumption: 680,
            intervals: vec![100],
            target_consumptions: vec![580, 680],
            next_blink_wait_frame: 100,
            target_wait_frame: None,
            target_exact: None,
            target_frame_before: None,
            target_frame_after: None,
            observed_model: 0,
            models: 16,
            timeline: Vec::new(),
            candidate_blinks: Vec::new(),
            target_blink_count: None,
            target_blink_count_offset_from_first_row: 0,
            timeline_cursor: None,
            target_cursor: None,
        }],
        ..FieldState::default()
    };

    assert_eq!(state.timeleap_models().unwrap(), 16);
    state.selected_result = 1;
    assert!(state.timeleap_models().is_err());
    state.results.clear();
    state.npc_count = "3".into();
    assert_eq!(state.timeleap_models().unwrap(), 4);
}

#[test]
fn cancelling_ready_unlocks_inputs_without_clearing_observation_or_timer() {
    let now = Instant::now();
    let deadline = now + Duration::from_secs(30);
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_stage = TimeleapStage::Frame1Waiting;
    state.timeleap_locked = true;
    state.target_input_locked = true;
    state.timeleap_frame1_input = "12000".into();
    state.timeleap_frame1_deadline = Some(deadline);
    state.timeleap_frame1_wait_seconds = 30.0;
    state.timer_kind = FieldTimerKind::Target;
    state.timer_start = Some(now);
    state.observed = vec![now];
    state.intervals = vec![24, 38];

    state.cancel_timeleap_ready();

    assert_eq!(state.timeleap_stage, TimeleapStage::Off);
    assert!(!state.timeleap_locked);
    assert!(!state.target_input_locked);
    assert_eq!(state.timeleap_frame1_input, "12000");
    assert_eq!(state.observed, vec![now]);
    assert_eq!(state.intervals, vec![24, 38]);
    assert_eq!(state.timeleap_frame1_deadline, Some(deadline));
    assert!(state.timer_is_running());
}

#[test]
fn frame1_passage_unlocks_only_the_research_range_among_timeleap_settings() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_locked = true;
    assert!(!state.timeleap_frame1_has_passed());
    assert!(!state.can_edit_timeleap_research_range());

    state.timeleap_stage = TimeleapStage::Assumed;
    assert!(state.timeleap_frame1_has_passed());
    assert!(state.can_edit_timeleap_research_range());
    assert!(state.timeleap_locked, "Timeleap setup remains locked");

    state.timeleap_stage = TimeleapStage::ShiftWaiting;
    assert!(state.timeleap_frame1_has_passed());
    assert!(!state.can_edit_timeleap_research_range());

    state.timeleap_stage = TimeleapStage::Failed;
    assert!(state.can_edit_timeleap_research_range());
}

#[test]
fn frame1_research_range_converts_frame_width_to_tick_radius() {
    assert!(frame1_tick_is_in_research_range(4_990, 5_000, 20));
    assert!(frame1_tick_is_in_research_range(5_010, 5_000, 20));
    assert!(!frame1_tick_is_in_research_range(4_989, 5_000, 20));
    assert!(!frame1_tick_is_in_research_range(5_011, 5_000, 20));
    assert!(frame1_tick_is_in_research_range(5_010, 5_000, 21));
    assert!(frame1_tick_is_in_research_range(5_049, 5_000, 98));
    assert!(!frame1_tick_is_in_research_range(5_050, 5_000, 98));
}

#[test]
fn manual_frame1_candidate_can_start_without_candidate_table_results() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.begin_timeleap_candidate_search();
    state.apply_timeleap_candidate_groups(Vec::new());
    let status = crate::domain::timeleap::TimeleapStatus {
        model: crate::domain::rng::ModelStatus::new(2),
        phase: 0,
        fidget_cd: -1,
        is_boy: false,
        raining: false,
    };
    let candidate = TimeleapCandidate {
        frame1: 10_000,
        frame1_tick: 5_000,
        post_tick: 5_000,
        grace_frames: 0,
        pre_frame: 10_000,
        pre_sfmt_position: 10_000,
        pre_status: status.clone(),
        post_frame: 10_000,
        post_sfmt_position: 10_000,
        post_status: status,
        target_inputs: vec![crate::domain::timeleap::TimeleapTargetInput {
            frame: 10_000,
            tick: 5_000,
        }]
        .into(),
    };

    state.apply_timeleap_frame1_candidates(
        crate::domain::timeleap::TimeleapFrame1Preparation {
            nominal_frame1: 10_000,
            nominal_tick: 5_000,
            candidates: vec![candidate],
        },
        60.0,
    );

    assert!(state.timeleap_search_completed_without_candidates());
    assert!(state.timeleap_locked);
    assert_eq!(state.timeleap_stage, TimeleapStage::Frame1Waiting);
    assert_eq!(state.timeleap_frame1_target, Some(10_000));
}

#[test]
fn manual_frame1_request_uses_the_input_without_target_or_candidate_search() {
    let seed = 0x436F_7E6C;
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=1_000,
        1_000,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate cache for manual Frame1 request");
    let mut state = FieldState::default();
    state.seed = format!("{seed:08X}");
    state.range_start = "0".into();
    state.target_consumption.clear();
    state.timeleap_frame1_input = "0".into();
    state.set_observation_cache(cache);

    let (config, frame1, _, _) = state
        .prepare_timeleap_frame1(crate::domain::timing::DEFAULT_FPS)
        .expect("Frame1 request does not depend on Target or search results");

    assert_eq!(frame1, 0);
    assert_eq!(config.target_frame, frame1);
    assert_eq!(config.handoff_range_frames, 98);
    assert!(state.timeleap_candidate_groups.is_empty());
}
