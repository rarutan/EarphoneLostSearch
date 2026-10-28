use super::*;

#[test]
fn observation_input_is_locked_after_unique_candidate() {
    let mut state = FieldState::default();
    state.mode = FieldMode::Observing;
    assert!(state.is_observation_input_enabled());
    state.results.push(FieldResult {
        start_consumption: 477,
        first_wait_frame: 2,
        last_wait_frame: 4,
        first_consumption: 478,
        last_consumption: 479,
        intervals: vec![2],
        target_consumptions: Vec::new(),
        next_blink_wait_frame: 2,
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
    assert!(!state.is_observation_input_enabled());
    state.refresh_nearest_timeline_frame();
    assert_eq!(state.nearest_timeline_frame, Some(477));
}

#[test]
fn target_remains_editable_until_explicit_correction_entry() {
    let mut state = FieldState::default();
    state.timer_start = Some(Instant::now());
    assert!(state.can_edit_target());

    state.enter_correction_mode(crate::domain::timing::DEFAULT_FPS);
    assert!(!state.can_edit_target());

    state.cancel_observation();
    assert!(state.can_edit_target());
}

#[test]
fn target_above_limit_restores_last_committed_value_without_pending_search() {
    let mut state = FieldState::default();
    state.restore_target_input("12345".into());
    state.target_consumption = (crate::domain::rng::MAX_SFMT_FRAME + 1).to_string();
    state.target_dirty = true;

    let error = state.commit_target_input().unwrap_err();

    assert_eq!(state.target_consumption, "12345");
    assert!(!state.target_dirty);
    assert!(!state.target_research_pending);
    assert!(matches!(
        error,
        AppError::OutOfRange {
            field: InputField::TargetConsumption,
            ..
        }
    ));
}

#[test]
fn target_input_reports_parse_and_negative_errors_and_restores_last_value() {
    let mut state = FieldState::default();
    state.restore_target_input("42".into());

    state.target_consumption = "not-a-frame".into();
    assert!(matches!(
        state.commit_target_input(),
        Err(AppError::InvalidInteger(InputField::TargetConsumption))
    ));
    assert_eq!(state.target_consumption, "42");

    state.target_consumption = "-1".into();
    assert!(matches!(
        state.commit_target_input(),
        Err(AppError::NegativeValue(InputField::TargetConsumption))
    ));
    assert_eq!(state.target_consumption, "42");
}

#[test]
fn target_limit_itself_is_valid_and_becomes_the_rollback_value() {
    let mut state = FieldState::default();
    state.target_consumption = crate::domain::rng::MAX_SFMT_FRAME.to_string();
    state.target_dirty = true;

    state.commit_target_input().unwrap();

    assert_eq!(state.target_consumption, "50000000");
    state.target_consumption = (crate::domain::rng::MAX_SFMT_FRAME + 1).to_string();
    state.target_dirty = true;
    assert!(state.commit_target_input().is_err());
    assert_eq!(state.target_consumption, "50000000");
}

#[test]
fn stopping_at_timeline_limit_clears_timer_and_marks_range_state() {
    let mut state = FieldState {
        timer_start: Some(Instant::now()),
        ..FieldState::default()
    };

    state.stop_blink_timer_out_of_range();

    assert!(!state.timer_is_running());
    assert!(state.next_blink_out_of_range);
    assert_eq!(state.timer_seconds_value(), 0.0);
    assert!(state.notice.is_none());
}

#[test]
fn timer_does_not_fallback_to_a_blink_beyond_the_timeline_limit() {
    let mut state = FieldState::default();
    state.observed.push(Instant::now());
    state.results.push(FieldResult {
        start_consumption: crate::domain::rng::MAX_SFMT_FRAME,
        first_wait_frame: 0,
        last_wait_frame: 0,
        first_consumption: crate::domain::rng::MAX_SFMT_FRAME,
        last_consumption: crate::domain::rng::MAX_SFMT_FRAME,
        intervals: Vec::new(),
        target_consumptions: Vec::new(),
        next_blink_wait_frame: 30,
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
        timeline_cursor: Some(TimelineCursor {
            status: ModelStatus::new(1),
            stream_cursor: crate::domain::rng::MAX_SFMT_FRAME,
            consumption: crate::domain::rng::MAX_SFMT_FRAME,
            tick: 0,
            timeleap_runtime: None,
        }),
        target_cursor: None,
    });

    let error = state
        .start_blink_timer(crate::domain::timing::DEFAULT_FPS)
        .unwrap_err();

    assert!(matches!(
        error,
        AppError::InvalidState(StateIssue::TimelineLimitReached)
    ));
    assert!(!state.timer_is_running());
    assert!(state.next_blink_out_of_range);
}

#[test]
fn observation_input_is_capped_at_sixty_four_blinks() {
    let mut state = FieldState::default();
    state.mode = FieldMode::Observing;
    state.observed = (0..crate::application::state::MAX_BLINK_OBSERVATIONS)
        .map(|_| Instant::now())
        .collect();
    state.observe(crate::domain::timing::DEFAULT_FPS);
    assert_eq!(
        state.observed.len(),
        crate::application::state::MAX_BLINK_OBSERVATIONS
    );
    assert!(state.notice.is_none());
}

#[test]
fn zero_candidate_search_allows_additional_shift_for_retry() {
    let mut state = FieldState::default();
    state.mode = FieldMode::Observing;
    state.search_exhausted = true;
    assert!(state.is_observation_input_enabled());
    state.start_observation();
    assert!(state.is_observation_input_enabled());
}

#[test]
fn observation_reset_and_restart_clear_candidate_notice() {
    let mut state = FieldState::default();
    state.notice = Some("候補が見つかりません".to_owned());

    state.cancel_observation();

    assert!(state.notice.is_none());

    state.notice = Some("候補が見つかりません".to_owned());
    state.start_observation();

    assert!(state.notice.is_none());
}

#[test]
fn invalidating_search_results_keeps_observation_but_clears_runtime_state() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.observed.push(Instant::now());
    state.intervals.push(200);
    state.results.push(FieldResult {
        start_consumption: 477,
        first_wait_frame: 2,
        last_wait_frame: 4,
        first_consumption: 478,
        last_consumption: 479,
        intervals: vec![2],
        target_consumptions: Vec::new(),
        next_blink_wait_frame: 2,

        target_wait_frame: Some(4),
        target_exact: Some(true),
        target_frame_before: Some(10),
        target_frame_after: Some(10),
        observed_model: 0,
        models: 2,
        timeline: Vec::new(),
        candidate_blinks: Vec::new(),
        target_blink_count: Some(1),
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: None,
    });
    state.timer_start = Some(Instant::now());
    state.target_dirty = true;
    state.invalidate_timeleap_frame1_preview();
    state.timeleap_frame1_reachable = Some(true);
    let preview_generation = state.timeleap_frame1_preview_generation();

    state.invalidate_search_results();

    assert!(state.results.is_empty());
    assert!(state.inferred_indices.is_empty());
    assert!(state.timer_start.is_none());
    assert_eq!(state.observed.len(), 1);
    assert_eq!(state.intervals, vec![200]);
    assert!(state.target_dirty);
    assert!(state.notice.is_none());
    assert_eq!(state.timeleap_frame1_reachable, None);
    assert!(!state.timeleap_frame1_preview_is_pending());
    assert_eq!(
        state.timeleap_frame1_preview_generation(),
        preview_generation.wrapping_add(1)
    );
}

#[test]
fn target_edit_search_results_are_accepted_while_cancelled_results_are_not() {
    let mut state = FieldState::default();
    state.mode = FieldMode::Idle;
    state.intervals = vec![200, 210];
    assert!(state.accepts_search_result());

    state.cancel_observation();
    assert!(!state.accepts_search_result());
}

#[test]
fn timer_kind_can_toggle_when_idle() {
    let mut state = FieldState::default();
    state.observed.push(Instant::now());
    state.results.push(FieldResult {
        start_consumption: 477,
        first_wait_frame: 2,
        last_wait_frame: 4,
        first_consumption: 478,
        last_consumption: 479,
        intervals: vec![2],
        target_consumptions: Vec::new(),
        next_blink_wait_frame: 2,
        target_wait_frame: Some(4),
        target_exact: Some(true),
        target_frame_before: Some(10),
        target_frame_after: Some(10),
        observed_model: 0,
        models: 2,
        timeline: Vec::new(),
        candidate_blinks: Vec::new(),
        target_blink_count: Some(1),
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: None,
    });
    assert_eq!(state.timer_kind, FieldTimerKind::NextBlink);
    state.toggle_timer_kind(59.8621).unwrap();
    assert_eq!(state.timer_kind, FieldTimerKind::Target);
    state.toggle_timer_kind(59.8621).unwrap();
    assert_eq!(state.timer_kind, FieldTimerKind::NextBlink);
}
