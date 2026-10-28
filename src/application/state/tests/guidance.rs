use super::*;

#[test]
fn guidance_batch_matches_main_reference_for_normal_waiting() {
    let request = GuidanceSearchRequest {
        session_generation: 0,
        observed_start: 10,
        observed_end: 90,
        range_start: 0,
        range_end: 1_000,
        seed: 0xB63523CA,
        target: 140,
        models: 6,
        offset: 200,
        consider_rotom: true,
        rotom_threshold: 79,
        observed_rotom_talk: None,
        pre_rotom_consumption: 0,
        npc_initial_load: 3,
        fastest_ticks: None,
    };

    assert_eq!(
        AppState::calculate_guidance_batch(request),
        main_reference_guidance_batch(request)
    );
}

#[test]
fn guidance_batch_matches_main_reference_for_fastest_close() {
    let seed = 0xB63523CA;
    let models = 6;
    let base = encounter_plan_with_observed_rotom_and_initial_load(seed, 10, 0, true, 79, None, 3)
        .unwrap()
        .encounter_start;
    let target = advance_timeline(seed, base, models, 5).unwrap();
    let request = GuidanceSearchRequest {
        session_generation: 0,
        observed_start: 10,
        observed_end: 90,
        range_start: 0,
        range_end: 1_000,
        seed,
        target,
        models,
        offset: 0,
        consider_rotom: true,
        rotom_threshold: 79,
        observed_rotom_talk: None,
        pre_rotom_consumption: 0,
        npc_initial_load: 3,
        fastest_ticks: Some(5),
    };

    assert_eq!(
        AppState::calculate_guidance_batch(request),
        main_reference_guidance_batch(request)
    );
}

#[test]
fn guidance_batch_matches_main_reference_at_a_high_absolute_position() {
    let request = GuidanceSearchRequest {
        session_generation: 0,
        observed_start: 50_000,
        observed_end: 50_080,
        range_start: 0,
        range_end: 100_000,
        seed: 1,
        target: 50_140,
        models: 23,
        offset: 0,
        consider_rotom: false,
        rotom_threshold: 79,
        observed_rotom_talk: None,
        pre_rotom_consumption: 0,
        npc_initial_load: 0,
        fastest_ticks: None,
    };

    assert_eq!(
        AppState::calculate_guidance_batch(request),
        main_reference_guidance_batch(request)
    );
}

#[test]
fn cached_guidance_batch_matches_seed_replay_at_a_high_absolute_position() {
    let request = GuidanceSearchRequest {
        session_generation: 0,
        observed_start: 50_000,
        observed_end: 50_080,
        range_start: 0,
        range_end: 100_000,
        seed: 1,
        target: 50_140,
        models: 23,
        offset: 0,
        consider_rotom: false,
        rotom_threshold: 79,
        observed_rotom_talk: None,
        pre_rotom_consumption: 0,
        npc_initial_load: 0,
        fastest_ticks: None,
    };
    let cache = crate::domain::rng::generate_observation_cache(
        request.seed,
        request.range_start..=request.range_end,
        request.target,
        DEFAULT_FPS,
    )
    .unwrap();

    let actual = AppState::calculate_guidance_batch_from_cache(request, &cache).unwrap();
    assert_eq!(
        actual
            .iter()
            .map(|(frame, reachable, _)| (*frame, *reachable))
            .collect::<Vec<_>>(),
        main_reference_guidance_batch(request)
    );
    let expected_seconds = cache
        .interval_seconds_range(
            request.observed_start,
            usize::try_from(request.observed_end - request.observed_start + 1).unwrap(),
        )
        .unwrap();
    assert_eq!(
        actual
            .iter()
            .map(|(_, _, seconds)| *seconds)
            .collect::<Vec<_>>(),
        expected_seconds
    );
}

#[test]
fn candidate_reachability_from_cache_matches_seed_replay() {
    let seed = 0x8A31_42C7;
    let models = 4;
    let initial =
        encounter_plan_with_observed_rotom_and_initial_load(seed, 10, 0, true, 79, None, 2)
            .unwrap()
            .encounter_start;
    let target = advance_timeline(seed, initial, models, 25).unwrap();
    let cache =
        crate::domain::rng::generate_observation_cache(seed, 0..=1_000, target, DEFAULT_FPS)
            .unwrap();

    for fastest_ticks in [None, Some(5)] {
        let request = CandidateReachabilityRequest {
            session_generation: 0,
            seed,
            target,
            models,
            offset: 0,
            consider_rotom: true,
            rotom_threshold: 79,
            observed_rotom_talk: None,
            pre_rotom_consumption: 0,
            npc_initial_load: 2,
            fastest_ticks,
            candidate_positions: vec![(0, 10), (1, 20)],
        };
        assert_eq!(
            AppState::calculate_candidate_reachability_from_cache(request.clone(), &cache).unwrap(),
            AppState::calculate_candidate_reachability(request),
        );
    }
}

#[test]
fn guidance_batch_short_circuits_target_beyond_current_window() {
    let request = GuidanceSearchRequest {
        session_generation: 0,
        observed_start: 100,
        observed_end: 104,
        range_start: 0,
        range_end: MAX_FRAME,
        seed: 1,
        target: 100 + MAX_PRODUCTION_LOOKAHEAD + 1,
        models: 6,
        offset: 0,
        consider_rotom: true,
        rotom_threshold: 79,
        observed_rotom_talk: None,
        pre_rotom_consumption: 0,
        npc_initial_load: 1,
        fastest_ticks: None,
    };
    assert_eq!(
        AppState::calculate_guidance_batch(request),
        vec![
            (100, false),
            (101, false),
            (102, false),
            (103, false),
            (104, false)
        ]
    );
}

#[test]
fn guidance_request_keeps_main_search_end_at_configured_range() {
    let mut state = AppState::default();
    state.range_start = "0".into();
    state.range_end = "1000".into();
    state.target = "5000".into();
    state.npc = "5".into();
    state.candidates = vec![TimelineCandidate {
        start_position: 0,
        sfmt_position: 3000,
        next_blink_ticks: Some(130),
        target_reachable: false,
    }];
    state.next_blink_index = Some(3000);

    let request = state.guidance_search_request().expect("guidance request");

    assert_eq!(request.observed_start, 3000);
    assert_eq!(request.observed_end, 3000);
    assert_eq!(request.models, 6);
}

#[test]
fn hatch_reachability_uses_nonzero_npc_count_in_the_computed_result() {
    let mut state = AppState::default();
    state.seed = "B63523CA".into();
    state.range_start = "0".into();
    state.range_end = "1000".into();
    state.npc = "10".into();
    state.consider_rotom_talk = false;
    state.candidates = vec![TimelineCandidate {
        start_position: 100,
        sfmt_position: 100,
        next_blink_ticks: Some(130),
        target_reachable: false,
    }];
    state.next_blink_index = Some(100);

    let mut guidance_differs_from_zero_npc = false;
    for target in 101..=1_000 {
        state.target = target.to_string();
        if let Some(mut request) = state.guidance_search_request() {
            assert_eq!(request.models, 11);
            request.observed_end = request.observed_start;
            let configured = AppState::calculate_guidance_batch(request);
            let mut zero_npc = request;
            zero_npc.models = 1;
            guidance_differs_from_zero_npc |=
                configured != AppState::calculate_guidance_batch(zero_npc);
        }
        if guidance_differs_from_zero_npc {
            break;
        }
    }
    assert!(
        guidance_differs_from_zero_npc,
        "right-side guidance must calculate with 11 models for NPC count 10"
    );

    state.candidates.push(TimelineCandidate {
        start_position: 110,
        sfmt_position: 110,
        next_blink_ticks: Some(130),
        target_reachable: false,
    });
    let mut candidate_differs_from_zero_npc = false;
    for target in 101..=1_000 {
        state.target = target.to_string();
        if let Some(request) = state.candidate_reachability_request() {
            assert_eq!(request.models, 11);
            let configured = AppState::calculate_candidate_reachability(request.clone());
            let mut zero_npc = request;
            zero_npc.models = 1;
            candidate_differs_from_zero_npc |=
                configured != AppState::calculate_candidate_reachability(zero_npc);
        }
        if candidate_differs_from_zero_npc {
            break;
        }
    }

    assert!(
        candidate_differs_from_zero_npc,
        "candidate ○× must calculate with 11 models for NPC count 10"
    );
}

#[test]
fn guidance_batch_covers_current_to_target_even_when_observation_range_ended() {
    let request = GuidanceSearchRequest {
        session_generation: 0,
        observed_start: 3_000,
        observed_end: 5_000,
        range_start: 0,
        range_end: 1_000,
        seed: 1,
        target: 5_000,
        models: 1,
        offset: 0,
        consider_rotom: false,
        rotom_threshold: 79,
        observed_rotom_talk: None,
        pre_rotom_consumption: 0,
        npc_initial_load: 0,
        fastest_ticks: None,
    };
    let results = AppState::calculate_guidance_batch(request);
    assert_eq!(results.len(), 2_001);
    assert_eq!(results.first().map(|(frame, _)| *frame), Some(3_000));
    assert_eq!(results.last().map(|(frame, _)| *frame), Some(5_000));
}

#[test]
fn fastest_guidance_applies_offset_ticks_once() {
    let seed = 1;
    let models = 2;
    let query = ProductionQuery {
        seed,
        target: 0,
        models,
        offset: 0,
        consider_rotom: true,
        rotom_threshold: 79,
        observed_rotom_talk: None,
        pre_rotom_consumption: 0,
        npc_initial_load: 1,
        fastest_ticks: Some(5),
    };
    let base = query.base_encounter_start(0).unwrap();
    let target = advance_timeline(seed, base, models, 5).unwrap();
    let results = AppState::calculate_guidance_batch(GuidanceSearchRequest {
        session_generation: 0,
        observed_start: 0,
        observed_end: 0,
        range_start: 0,
        range_end: 100,
        seed,
        target,
        models,
        offset: 0,
        consider_rotom: true,
        rotom_threshold: 79,
        observed_rotom_talk: None,
        pre_rotom_consumption: 0,
        npc_initial_load: 1,
        fastest_ticks: Some(5),
    });
    assert_eq!(results, vec![(0, true)]);
}

#[test]
fn guidance_snapshot_exposes_next_circle_after_worker_result() {
    let mut state = AppState::default();
    state.range_start = "0".into();
    state.range_end = "5000".into();
    state.target = "5000".into();
    state.npc = "5".into();
    state.observation_cache = crate::domain::rng::generate_observation_cache(
        0xB63523CA,
        0..=5_000,
        5_000,
        state.fps_value(),
    )
    .ok();
    state.candidates = vec![TimelineCandidate {
        start_position: 3000,
        sfmt_position: 3000,
        next_blink_ticks: Some(130),
        target_reachable: false,
    }];
    let request = state.guidance_search_request().unwrap();
    let results = AppState::calculate_guidance_batch(request);
    state.apply_guidance_results(request, results);
    assert!(state
        .hatch_guidance()
        .unwrap()
        .unwrap()
        .next_circle_frame
        .is_some());
}

#[test]
fn guidance_snapshot_keeps_current_result_visible_while_future_chunks_are_pending() {
    let mut state = AppState::default();
    state.range_start = "0".into();
    state.range_end = "5000".into();
    state.target = "5000".into();
    state.npc = "5".into();
    state.observation_cache = crate::domain::rng::generate_observation_cache(
        0xB63523CA,
        0..=5_000,
        5_000,
        state.fps_value(),
    )
    .ok();
    state.candidates = vec![TimelineCandidate {
        start_position: 3000,
        sfmt_position: 3000,
        next_blink_ticks: Some(130),
        target_reachable: false,
    }];

    let request = state.guidance_search_request().unwrap();
    let first_result = AppState::calculate_guidance_batch(request)[0];
    let mut first_request = request;
    first_request.observed_end = first_request.observed_start;
    state.apply_guidance_results(first_request, vec![first_result]);

    let guidance = state.hatch_guidance().unwrap().unwrap();
    assert_eq!(guidance.current_reachability, Some(first_result.1));
    assert!(state.guidance_search_pending());
}

#[test]
fn fastest_close_marks_target_unreachable_after_extended_target_window_finishes() {
    let mut state = AppState::default();
    state.range_start = "0".into();
    state.range_end = "5000".into();
    state.target = "6000".into();
    state.npc = "0".into();
    state.fastest_close = true;
    state.observation_cache = crate::domain::rng::generate_observation_cache(
        0xB63523CA,
        0..=10_000,
        10_000,
        state.fps_value(),
    )
    .ok();
    state.candidates = vec![TimelineCandidate {
        start_position: 4800,
        sfmt_position: 4800,
        next_blink_ticks: Some(130),
        target_reachable: false,
    }];
    state.next_blink_index = Some(4800);

    let request = state
        .guidance_search_request()
        .expect("Target window must remain searchable");
    let results = (request.observed_start..=request.observed_end)
        .map(|frame| (frame, false))
        .collect();
    state.apply_guidance_results(request, results);

    let guidance = state
        .hatch_guidance()
        .expect("guidance calculation")
        .expect("guidance snapshot");
    assert_eq!(guidance.current_reachability, Some(false));
    assert!(guidance.target_unreachable);
}

#[test]
fn fastest_close_grace_uses_seed_when_target_is_beyond_observation_cache() {
    let mut state = AppState::default();
    state.range_end = "5000".into();
    state.target = "10000".into();
    state.npc = "0".into();
    state.fastest_close = true;
    state.observation_cache = crate::domain::rng::generate_observation_cache(
        0xB63523CA,
        0..=5_000,
        5_000,
        state.fps_value(),
    )
    .ok();
    state.candidates = vec![TimelineCandidate {
        start_position: 4800,
        sfmt_position: 4800,
        next_blink_ticks: Some(130),
        target_reachable: false,
    }];
    state.next_blink_index = Some(4800);
    let request = state.guidance_search_request().unwrap();
    let results = AppState::calculate_guidance_batch(request);
    state.apply_guidance_results(request, results);
    let guidance = state.hatch_guidance().unwrap().unwrap();
    assert!(guidance.next_circle_frame.is_some());
    assert!(guidance.grace_seconds.is_some());
}

#[test]
fn guidance_marks_targets_outside_the_search_window_without_calling_them_unreachable() {
    for fastest_close in [false, true] {
        let mut state = AppState::default();
        state.range_start = "0".into();
        state.range_end = "5000".into();
        state.target = "110000".into();
        state.npc = "0".into();
        state.fastest_close = fastest_close;
        state.observation_cache = crate::domain::rng::generate_observation_cache(
            0xB63523CA,
            0..=5_000,
            5_000,
            state.fps_value(),
        )
        .ok();
        state.candidates = vec![TimelineCandidate {
            start_position: 4800,
            sfmt_position: 4800,
            next_blink_ticks: Some(130),
            target_reachable: false,
        }];
        state.next_blink_index = Some(4800);

        let guidance = state
            .hatch_guidance()
            .expect("guidance calculation")
            .expect("guidance snapshot");
        assert!(guidance.search_out_of_range);
        assert!(!guidance.target_unreachable);
    }
}

#[test]
fn normal_guidance_keeps_near_target_inside_the_current_window() {
    let mut state = AppState::default();
    state.range_start = "0".into();
    state.range_end = "5000".into();
    state.target = "6000".into();
    state.npc = "0".into();
    state.observation_cache = crate::domain::rng::generate_observation_cache(
        0xB63523CA,
        0..=5_000,
        5_000,
        state.fps_value(),
    )
    .ok();
    state.candidates = vec![TimelineCandidate {
        start_position: 4800,
        sfmt_position: 4800,
        next_blink_ticks: Some(130),
        target_reachable: false,
    }];
    state.next_blink_index = Some(4800);

    let guidance = state
        .hatch_guidance()
        .expect("guidance calculation")
        .expect("guidance snapshot");
    assert!(!guidance.search_out_of_range);
    assert!(!guidance.target_unreachable);
}

#[test]
fn normal_guidance_marks_a_target_before_current_as_unreachable() {
    let mut state = AppState::default();
    state.range_start = "0".into();
    state.range_end = "5000".into();
    state.target = "4000".into();
    state.npc = "0".into();
    state.observation_cache = crate::domain::rng::generate_observation_cache(
        0xB63523CA,
        0..=5_000,
        5_000,
        state.fps_value(),
    )
    .ok();
    state.candidates = vec![TimelineCandidate {
        start_position: 4800,
        sfmt_position: 4800,
        next_blink_ticks: Some(130),
        target_reachable: false,
    }];
    state.next_blink_index = Some(4800);

    let guidance = state
        .hatch_guidance()
        .expect("guidance calculation")
        .expect("guidance snapshot");
    assert!(!guidance.search_out_of_range);
    assert!(guidance.target_unreachable);
}

#[test]
fn stale_guidance_results_are_rejected_by_state() {
    let mut state = AppState::default();
    state.range_start = "0".into();
    state.range_end = "5000".into();
    state.target = "5000".into();
    state.npc = "5".into();
    state.observation_cache = crate::domain::rng::generate_observation_cache(
        0xB63523CA,
        0..=5_000,
        5_000,
        state.fps_value(),
    )
    .ok();
    state.candidates = vec![TimelineCandidate {
        start_position: 3000,
        sfmt_position: 3000,
        next_blink_ticks: Some(130),
        target_reachable: false,
    }];

    let current = state.guidance_search_request().unwrap();
    let mut stale = current;
    stale.seed ^= 1;
    let result = vec![(current.observed_start, false)];
    state.apply_guidance_results(stale, result);

    assert!(state.guidance.is_empty());
    assert_eq!(state.guidance.computed_until(), None);
}

#[test]
fn guidance_snapshot_finishes_grace_at_a_known_no_result() {
    let mut state = AppState::default();
    state.range_start = "0".into();
    state.range_end = "5000".into();
    state.target = "5000".into();
    state.npc = "5".into();
    state.observation_cache = crate::domain::rng::generate_observation_cache(
        0xB63523CA,
        0..=5_000,
        5_000,
        state.fps_value(),
    )
    .ok();
    state.candidates = vec![TimelineCandidate {
        start_position: 3000,
        sfmt_position: 3000,
        next_blink_ticks: Some(130),
        target_reachable: false,
    }];
    state.guidance.record_reachability(3000, true);
    state.guidance.record_reachability(3001, false);
    let interval_seconds = state
        .observation_cache
        .as_ref()
        .unwrap()
        .interval_seconds_range(3000, 2)
        .unwrap();
    state
        .guidance
        .cache_interval_seconds(3000, &interval_seconds);
    state.guidance.mark_computed_until(3001);

    let guidance = state.hatch_guidance().unwrap().unwrap();
    assert_eq!(guidance.current_reachability, Some(true));
    assert!(guidance.grace_seconds.is_some());
    assert!(state.guidance_search_pending());
}

#[test]
fn fastest_guidance_marks_theoretical_outside_position_as_no_result() {
    let mut state = AppState::default();
    state.range_start = "0".into();
    state.range_end = "10000".into();
    state.target = "5000".into();
    state.npc = "0".into();
    state.use_offset = true;
    state.encounter_offset = "1000".into();
    state.fastest_close = true;
    state.observation_cache = crate::domain::rng::generate_observation_cache(
        0xB63523CA,
        0..=10_000,
        10_000,
        state.fps_value(),
    )
    .ok();
    state.candidates = vec![TimelineCandidate {
        start_position: 3000,
        sfmt_position: 3000,
        next_blink_ticks: Some(130),
        target_reachable: false,
    }];

    let guidance = state.hatch_guidance().unwrap().unwrap();
    assert_eq!(guidance.current_reachability, Some(false));
    assert!(state.guidance_search_pending());
}

#[test]
fn fastest_close_near_target_keeps_outside_current_position_and_searches_target_window() {
    let mut state = AppState::default();
    state.range_start = "0".into();
    state.range_end = "10000".into();
    state.target = "6000".into();
    state.npc = "0".into();
    state.fastest_close = true;
    state.observation_cache = crate::domain::rng::generate_observation_cache(
        0xB63523CA,
        0..=10_000,
        10_000,
        state.fps_value(),
    )
    .ok();
    state.candidates = vec![TimelineCandidate {
        start_position: 4800,
        sfmt_position: 4800,
        next_blink_ticks: Some(130),
        target_reachable: false,
    }];
    state.next_blink_index = Some(4800);

    let guidance = state
        .hatch_guidance()
        .expect("guidance calculation")
        .expect("guidance snapshot");
    assert_eq!(guidance.current_reachability, Some(false));
    assert!(state.guidance_search_request().is_some());
    assert!(state.guidance_search_pending());
}

#[test]
fn fastest_close_searches_target_window_when_observation_range_has_ended() {
    let mut state = AppState::default();
    state.range_start = "0".into();
    state.range_end = "5000".into();
    state.target = "6000".into();
    state.npc = "0".into();
    state.fastest_close = true;
    state.observation_cache = crate::domain::rng::generate_observation_cache(
        0xB63523CA,
        0..=10_000,
        10_000,
        state.fps_value(),
    )
    .ok();
    state.candidates = vec![TimelineCandidate {
        start_position: 4800,
        sfmt_position: 4800,
        next_blink_ticks: Some(130),
        target_reachable: false,
    }];
    state.next_blink_index = Some(4800);

    let request = state
        .guidance_search_request()
        .expect("Target window must remain searchable");
    assert!(request.observed_end > 5000);
    assert!(state.guidance_search_pending());
}

#[test]
fn candidate_reachability_key_is_stable_until_candidate_position_changes() {
    let mut state = AppState::default();
    state.npc = "10".into();
    state.candidates = vec![
        TimelineCandidate {
            start_position: 100,
            sfmt_position: 100,
            next_blink_ticks: Some(10),
            target_reachable: false,
        },
        TimelineCandidate {
            start_position: 200,
            sfmt_position: 200,
            next_blink_ticks: Some(10),
            target_reachable: false,
        },
    ];
    state.next_blink_index = Some(100);

    let key = state.candidate_reachability_key().expect("candidate key");
    assert_eq!(state.candidate_reachability_key(), Some(key));
    let request = state
        .candidate_reachability_request_for_key(key)
        .expect("candidate snapshot");
    assert_eq!(request.candidate_positions, vec![(0, 100), (1, 200)]);
    assert_eq!(key.models, 11);
    assert_eq!(request.models, 11);

    state.npc = "0".into();
    let zero_npc_key = state
        .candidate_reachability_key()
        .expect("zero-NPC candidate key");
    assert_ne!(zero_npc_key, key);
    assert_eq!(zero_npc_key.models, 1);
    state.apply_candidate_reachability(&request, &[(0, true), (1, true)]);
    assert!(state
        .candidates
        .iter()
        .all(|candidate| !candidate.target_reachable));

    state.npc = "10".into();
    state.next_blink_index = Some(101);
    let advanced = state.candidate_reachability_key().expect("advanced key");
    assert_ne!(advanced, key);
    assert_eq!(advanced.selected_position, 101);
}

#[test]
fn replaced_candidate_set_rejects_the_previous_reachability_result() {
    let mut state = AppState::default();
    state.candidates = vec![
        TimelineCandidate {
            start_position: 100,
            sfmt_position: 100,
            next_blink_ticks: Some(10),
            target_reachable: false,
        },
        TimelineCandidate {
            start_position: 200,
            sfmt_position: 200,
            next_blink_ticks: Some(10),
            target_reachable: false,
        },
    ];
    let stale = state.candidate_reachability_request().expect("request");
    state.replace_candidates(vec![
        TimelineCandidate {
            start_position: 300,
            sfmt_position: 300,
            next_blink_ticks: Some(10),
            target_reachable: false,
        },
        TimelineCandidate {
            start_position: 400,
            sfmt_position: 400,
            next_blink_ticks: Some(10),
            target_reachable: false,
        },
    ]);

    state.apply_candidate_reachability(&stale, &[(0, true), (1, true)]);
    assert!(state
        .candidates
        .iter()
        .all(|candidate| !candidate.target_reachable));
}
