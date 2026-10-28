use super::*;

#[test]
fn production_preparation_builds_the_selected_timeline_off_the_cached_sequence() {
    let mut state = AppState::default();
    state.seed = "00000001".into();
    state.range_start = "0".into();
    state.range_end = "1000".into();
    state.target = "5000".into();
    state.npc = "5".into();
    state.candidates = vec![TimelineCandidate {
        start_position: 0,
        sfmt_position: 1000,
        next_blink_ticks: Some(130),
        target_reachable: false,
    }];
    install_test_cache(&mut state);
    state.recalculate_production();
    assert!(state.production_preparation_pending);
    assert!(state.production.is_none());

    let (request, cache) = state
        .production_preparation_request()
        .expect("production request");
    assert_eq!(request.query.models, 6);
    let prepared = AppState::calculate_production_preparation(request.clone(), cache)
        .expect("prepared production timeline");

    assert_eq!(prepared.production.target, 5000);
    assert!(state.apply_production_preparation(&request, prepared));
    assert!(!state.production_preparation_pending);
    assert_eq!(
        state.production.as_ref().map(|value| value.target),
        Some(5000)
    );
}

#[test]
fn npc_count_changes_rebuild_production_without_resetting_blink_state() {
    let mut state = AppState::default();
    state.mode = Mode::Ready;
    state.seed = "F3A71A2B".into();
    state.range_start = "0".into();
    state.range_end = "1000".into();
    state.target = "5000".into();
    state.consider_rotom_talk = false;
    state.npc = "0".into();
    state.candidates = vec![TimelineCandidate {
        start_position: 0,
        sfmt_position: 1000,
        next_blink_ticks: Some(130),
        target_reachable: false,
    }];
    let blink_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    state.next_blink = Some(blink_deadline);
    state.next_blink_index = Some(1000);
    install_test_cache(&mut state);
    state.recalculate_production();

    let (old_request, old_cache) = state
        .production_preparation_request()
        .expect("request with no NPCs");
    assert_eq!(old_request.query.models, 1);
    let old_prepared = AppState::calculate_production_preparation(old_request.clone(), old_cache)
        .expect("old production timeline");

    state.npc = "13".into();
    state.recalculate_production();

    assert_eq!(state.candidates.len(), 1);
    assert_eq!(state.candidates[0].sfmt_position, 1000);
    assert_eq!(state.next_blink, Some(blink_deadline));
    assert_eq!(state.next_blink_index, Some(1000));
    assert!(!state.apply_production_preparation(&old_request, old_prepared));

    let (new_request, new_cache) = state
        .production_preparation_request()
        .expect("request with edited NPC count");
    assert_eq!(new_request.query.models, 14);
    assert_eq!(
        state.normal_target_wait_key(1000).map(|key| key.models),
        Some(14)
    );
    let new_prepared = AppState::calculate_production_preparation(new_request.clone(), new_cache)
        .expect("updated production timeline");
    assert!(state.apply_production_preparation(&new_request, new_prepared));
    assert_eq!(state.next_blink, Some(blink_deadline));
}

#[test]
fn enter_during_production_preparation_starts_when_the_worker_result_arrives() {
    let mut state = AppState::default();
    state.mode = Mode::Ready;
    state.fastest_close = false;
    state.use_offset = false;
    state.range_start = "0".into();
    state.range_end = "1000".into();
    state.target = "5000".into();
    state.npc = "5".into();
    state.candidates = vec![TimelineCandidate {
        start_position: 0,
        sfmt_position: 1000,
        next_blink_ticks: Some(130),
        target_reachable: false,
    }];
    install_test_cache(&mut state);
    let config = state.search_config().expect("valid hatch conditions");
    let predicted_rotom = state
        .predicted_rotom_talk()
        .expect("cached Rotom prediction");
    let resolved_plan = state
        .plan_for_encounter(
            config.seed,
            1000,
            state.encounter_offset_value(),
            state.consider_rotom_talk,
            state.rotom_threshold_value(),
            Some(predicted_rotom),
        )
        .expect("plan with the Enter-time Rotom prediction");
    state.target = advance_timeline(
        config.seed,
        resolved_plan.encounter_start,
        config.npc_models,
        5,
    )
    .expect("reachable test Target")
    .to_string();
    state.mark_production_preparation_pending();
    let (stale_request, cache) = state
        .production_preparation_request()
        .expect("production request");
    let stale_prepared = AppState::calculate_production_preparation(stale_request.clone(), cache)
        .expect("prepared production timeline");

    state.begin_encounter();
    assert!(state.encounter_start_requested);
    assert!(state.is_ready());
    assert!(!state.apply_production_preparation(&stale_request, stale_prepared));
    assert!(state.production_preparation_pending);
    assert!(state.encounter_start_requested);

    let (current_request, cache) = state
        .production_preparation_request()
        .expect("request with Enter-time Rotom selection");
    let current_prepared =
        AppState::calculate_production_preparation(current_request.clone(), cache)
            .expect("current production timeline");

    assert!(state.apply_production_preparation(&current_request, current_prepared));

    assert!(!state.encounter_start_requested);
    assert!(state.is_countdown());
}

#[test]
fn target_wait_fallback_matches_exact_existing_timeline_calculation_and_is_cached() {
    let mut state = AppState::default();
    let seed = 0xD4CA_77A1;
    state.seed = format!("{seed:08x}");
    state.range_start = "0".into();
    state.range_end = "6000".into();
    state.target = "6000".into();
    state.npc = "4".into();
    let cache =
        crate::domain::rng::generate_observation_cache(seed, 0..=6_000, 6_000, state.fps_value())
            .unwrap();
    state.observation_cache = Some(cache.clone());
    state.candidates = vec![TimelineCandidate {
        start_position: 1_000,
        sfmt_position: 1_000,
        next_blink_ticks: Some(10),
        target_reachable: false,
    }];
    state.next_blink_index = Some(1_000);

    let request = state
        .normal_target_wait_fallback_request(1_000, None)
        .expect("fallback request");
    assert_eq!(state.normal_target_wait_ticks(1_000), None);
    let expected = timeline_target_from_cache(
        &cache,
        request.key.encounter_start,
        request.key.target,
        request.key.models,
    )
    .unwrap()
    .map(|timing| timing.timeline_tick);
    let calculated = AppState::calculate_normal_target_wait_fallback(request, &cache).unwrap();
    assert_eq!(calculated, expected);
    assert!(state.apply_normal_target_wait_fallback(request, calculated));
    assert_eq!(state.normal_target_wait_ticks(1_000), Some(calculated));
    assert!(state
        .normal_target_wait_fallback_request(1_000, None)
        .is_none());
}

#[test]
fn target_wait_fallback_rejects_a_result_for_a_position_that_has_moved() {
    let mut state = AppState::default();
    let seed = 0xD4CA_77A2;
    state.seed = format!("{seed:08x}");
    state.range_start = "0".into();
    state.range_end = "6000".into();
    state.target = "6000".into();
    state.npc = "4".into();
    state.observation_cache =
        crate::domain::rng::generate_observation_cache(seed, 0..=6_000, 6_000, state.fps_value())
            .ok();
    state.candidates = vec![TimelineCandidate {
        start_position: 1_000,
        sfmt_position: 1_000,
        next_blink_ticks: Some(10),
        target_reachable: false,
    }];
    state.next_blink_index = Some(1_000);
    let request = state
        .normal_target_wait_fallback_request(1_000, None)
        .expect("fallback request");
    state.next_blink_index = Some(1_001);

    assert!(!state.apply_normal_target_wait_fallback(request, Some(10)));
    assert!(state.normal_target_wait_cache.is_empty());
}
