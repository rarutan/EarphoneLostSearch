use super::*;

#[test]
fn observed_timeline_drift_corrects_offset_in_the_opposite_direction() {
    assert_eq!(offset_after_observed_timeline_drift(0, 2), -2);
    assert_eq!(offset_after_observed_timeline_drift(20, 4), 16);
    assert_eq!(offset_after_observed_timeline_drift(20, -4), 24);
}

#[test]
fn observation_match_keeps_interval_and_sfmt_position_together() {
    let mut state = AppState::default();
    state.mode = Mode::Observing;
    state.intervals = vec![283, 381];
    let matched = ObservationMatch {
        start_position: 12,
        sfmt_position: 35,
        next_blink_ticks: 417,
    };
    let candidate = state.make_candidate(matched);
    assert_eq!(candidate.start_position, 12);
    assert_eq!(candidate.sfmt_position, 35);
    assert_eq!(candidate.next_blink_ticks, Some(417));
}

#[test]
fn range_button_uses_target_as_end() {
    let mut state = AppState::default();
    state.target = "12345".into();
    state.range_before_target = "10000".into();
    state.set_range_from_target();
    assert_eq!(state.range_start, "2345");
    assert_eq!(state.range_end, "12345");

    state.target = "5000".into();
    state.set_range_from_target();
    assert_eq!(state.range_start, "0");
    assert_eq!(state.range_end, "5000");

    state.target = "12345".into();
    state.range_before_target = "1234".into();
    state.set_range_from_target();
    assert_eq!(state.range_start, "11111");
}

#[test]
fn search_config_rejects_invalid_or_unreasonably_large_npc_counts() {
    let mut state = AppState::default();
    state.npc = "not-a-number".into();
    assert!(state.search_config().is_err());

    state.npc = (MAX_NPC_COUNT + 1).to_string();
    assert!(state.search_config().is_err());

    state.npc = MAX_NPC_COUNT.to_string();
    assert!(state.search_config().is_ok());
}

#[test]
fn search_config_uses_edited_hatch_npc_count_for_model_count() {
    let mut state = AppState::default();
    state.npc = "13".into();

    let config = state.search_config().expect("valid NPC input");

    assert_eq!(config.npc_models, 14);
}

#[test]
fn search_config_allows_target_far_from_observation_range_start() {
    let mut state = AppState::default();
    state.range_start = "0".into();
    state.target = (MAX_PRODUCTION_LOOKAHEAD + 1).to_string();
    assert!(state.search_config().is_ok());

    state.range_start = "9000000".into();
    state.range_end = (9_000_000 + MAX_PRODUCTION_LOOKAHEAD).to_string();
    state.target = (9_000_000 + MAX_PRODUCTION_LOOKAHEAD).to_string();
    assert!(state.search_config().is_ok());
}

#[test]
fn incomplete_guidance_results_do_not_advance_computed_end() {
    let mut state = AppState::default();
    let request = GuidanceSearchRequest {
        session_generation: 0,
        observed_start: 10,
        observed_end: 12,
        range_start: 0,
        range_end: 100,
        seed: 1,
        target: 20,
        models: 1,
        offset: 0,
        consider_rotom: false,
        rotom_threshold: 79,
        observed_rotom_talk: None,
        pre_rotom_consumption: 0,
        npc_initial_load: 0,
        fastest_ticks: None,
    };

    state.apply_guidance_results(request, vec![(10, false), (12, true)]);

    assert_eq!(state.guidance.computed_until(), None);
    assert!(state.guidance.is_empty());
}

#[test]
fn encounter_offset_uses_configured_timeline_fps() {
    let mut state = AppState::default();
    state.fps = "50".into();
    state.use_offset = true;
    state.encounter_offset = "100".into();
    assert!((state.encounter_offset_seconds() - 100.0 / 50.0).abs() < f64::EPSILON);

    state.encounter_offset = "101".into();
    assert!(state.encounter_offset_error().is_some());
}

#[test]
fn fps_value_accepts_machine_specific_calibration_and_falls_back_for_invalid_input() {
    let mut state = AppState::default();
    state.fps = "59.8702".into();
    assert_eq!(state.fps_value(), 59.8702);
    assert_eq!(state.search_config().unwrap().fps, 59.8702);

    state.fps = "0".into();
    assert_eq!(state.fps_value(), DEFAULT_FPS);

    state.fps = "not-a-number".into();
    assert_eq!(state.fps_value(), DEFAULT_FPS);
}
