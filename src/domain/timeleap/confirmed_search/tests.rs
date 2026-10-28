use super::super::runtime::apply_consider_delay;
use super::*;

#[test]
fn confirmed_target_drift_converts_signed_tick_difference_to_frames() {
    assert_eq!(confirmed_target_drift_frames(13, 10), 6);
    assert_eq!(confirmed_target_drift_frames(8, 10), -4);
    assert_eq!(confirmed_target_drift_frames(10, 10), 0);
}

#[test]
fn confirmed_target_grace_groups_only_adjacent_rows_with_same_generation_start_frame() {
    let mut rows = [
        Some(70_000),
        Some(70_000),
        Some(70_002),
        Some(70_002),
        None,
        Some(70_002),
        Some(70_002),
        Some(70_000),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, generation_start_frame)| ConfirmedTargetSearchRow {
        frame1: 480,
        target_frame: 1_000 + index as i64 * 2,
        drift_frames: 0,
        grace_frames: 0,
        generation_start_frame,
    })
    .collect::<Vec<_>>();

    assign_confirmed_target_grace_frames(&mut rows);

    assert_eq!(
        rows.iter().map(|row| row.grace_frames).collect::<Vec<_>>(),
        vec![4, 4, 4, 4, 0, 4, 4, 2],
        "a changed generation frame or unreachable row starts a separate grace interval"
    );
}

#[test]
fn confirmed_target_result_carries_precomputed_drift_marker_without_changing_rows() {
    let rows = vec![
        ConfirmedTargetSearchRow {
            frame1: 480,
            target_frame: 997,
            drift_frames: -4,
            grace_frames: 2,
            generation_start_frame: Some(900),
        },
        ConfirmedTargetSearchRow {
            frame1: 480,
            target_frame: 999,
            drift_frames: 0,
            grace_frames: 2,
            generation_start_frame: Some(920),
        },
    ];

    let result = ConfirmedTargetSearchResult::from_rows(rows.clone(), Some(910));

    assert_eq!(
        result.rows, rows,
        "the worker result retains every row in order"
    );
    assert_eq!(result.drift_marker_boundary, Some(1));
    assert_eq!(
        ConfirmedTargetSearchResult::from_rows(result.rows.clone(), Some(920))
            .drift_marker_boundary,
        None,
        "an exact generation-frame match does not add a boundary marker"
    );
}

#[test]
fn exact_hit_bounds_keep_intervening_unmatched_timeline_ticks() {
    let hits = [(10, 20, 700), (11, 22, 700), (13, 26, 700), (14, 28, 700)];
    let groups = qualified_generation_hit_ranges(&hits, 0);
    assert_eq!(groups, vec![(10, 11), (13, 14)]);
    assert_eq!(group_tick_bounds(&groups), Some((10, 14)));
}

#[test]
fn confirmed_target_search_starts_from_confirmed_timeline_and_filters_grace() {
    let seed = 0xA02D_5C71;
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=10_000,
        10_000,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate cache for confirmed Target search");
    let mut status = TimeleapStatus::new(4);
    status.is_boy = true;
    status.fidget_cd = 3;
    let start = ConfirmedTimeleapStart {
        frame: 500,
        sfmt_position: 500,
        tick: 0,
        status,
    };
    let original_start = start.clone();
    let mut status = start.status.clone();
    let mut cursor = Cursor {
        frame: start.frame,
        sfmt_position: start.sfmt_position,
        tick: start.tick,
    };
    let mut sfmt = cached_sfmt_at(&cache, cursor.sfmt_position)
        .expect("confirmed cursor has cached SFMT state");
    for _ in 0..5 {
        advance_one_tick(&mut status, &mut cursor, &mut sfmt);
    }
    let generation_frame = cursor.frame;
    let _nearby_baseline_target_frame = {
        let mut baseline_status = start.status.clone();
        let mut baseline_cursor = Cursor {
            frame: start.frame,
            sfmt_position: start.sfmt_position,
            tick: start.tick,
        };
        let mut baseline_sfmt = cached_sfmt_at(&cache, baseline_cursor.sfmt_position)
            .expect("confirmed cursor has cached SFMT state");
        for _ in 0..4 {
            advance_one_tick(
                &mut baseline_status,
                &mut baseline_cursor,
                &mut baseline_sfmt,
            );
        }
        baseline_cursor.frame
    };
    // A far-away display baseline must not move the generation-frame search.
    let baseline_target_frame = 9_000;
    let request = ConfirmedTargetSearchRequest {
        seed,
        confirmed_frame1: 480,
        timeline_generation: 7,
        source_config: None,
        start,
        baseline_target_frame,
        generation_start_frame: generation_frame,
        drift_search_generation_start_frame: None,
        drift_range_ticks: 4,
        grace_min_frames: 0,
        consider_delay: None,
    };
    let rows = run_confirmed_target_search(&request, &cache, &AtomicBool::new(false))
        .expect("search on the confirmed Timeline");
    assert_eq!(
        rows.len(),
        1,
        "only the exact TargetFrame tick reaches this frame"
    );
    assert!(rows
        .iter()
        .any(|row| { row.generation_start_frame == Some(generation_frame) && row.frame1 == 480 }));

    let mut same_center_search = request.clone();
    same_center_search.drift_search_generation_start_frame = Some(generation_frame);
    let centered_rows =
        run_confirmed_target_search(&same_center_search, &cache, &AtomicBool::new(false))
            .expect("drift mode displays around the measured generation frame");
    assert_eq!(centered_rows.len(), 9);
    assert!(centered_rows
        .iter()
        .any(|row| { row.generation_start_frame == Some(generation_frame) }));

    let mut drift_search = request.clone();
    drift_search.drift_search_generation_start_frame = Some(generation_frame.saturating_add(4));
    let drift_rows = run_confirmed_target_search(&drift_search, &cache, &AtomicBool::new(false))
        .expect("drift search uses the observed frame as its display-window center");
    assert!(!drift_rows.is_empty());
    assert!(drift_rows
        .iter()
        .any(|row| row.generation_start_frame == Some(generation_frame)));
    assert_eq!(drift_rows.len(), 9);
    assert!(drift_rows
        .iter()
        .any(|row| { row.generation_start_frame == Some(generation_frame.saturating_add(4)) }));
    assert!(drift_rows.iter().all(|row| row.drift_frames % 2 == 0));
    assert_eq!(
        request.start, original_start,
        "search must not mutate its input snapshot"
    );

    let mut strict_grace = request;
    strict_grace.grace_min_frames = 98;
    assert!(
        run_confirmed_target_search(&strict_grace, &cache, &AtomicBool::new(false))
            .expect("an unmet grace threshold is a normal empty result")
            .is_empty()
    );

    let mut unreachable_generation = strict_grace;
    unreachable_generation.grace_min_frames = 0;
    unreachable_generation.generation_start_frame =
        unreachable_generation.start.frame.saturating_sub(1);
    assert!(matches!(
        run_confirmed_target_search(&unreachable_generation, &cache, &AtomicBool::new(false)),
        Err(TimeleapSearchFailure::GenerationFrameUnreachable)
    ));
}

#[test]
fn confirmed_target_search_rebuilds_frame1_from_explicit_source_inputs() {
    let seed = 0xA02D_5C74;
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=10_000,
        10_000,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate cache for standalone TargetFrame search");
    let config = TimeleapConfig {
        seed,
        start_frame: 480,
        target_frame: 10_000,
        models: 2,
        min_frame1_elapsed_ticks: 0,
        max_frame1_elapsed_ticks: 0,
        mode: TimeleapMode::Menu,
        is_boy: true,
        handoff_range_frames: 0,
        courier: CourierConfig::default(),
    };
    let preparation = run_timeleap_frame1_preparation(
        &config,
        480,
        &TimeleapTimelineStart::initial(480, 2),
        &cache,
        &AtomicBool::new(false),
    )
    .expect("Frame1 should be simulated from the explicit starting frame");
    let candidate = preparation
        .candidates
        .iter()
        .find(|candidate| candidate.frame1 == 480)
        .expect("initial Frame1 is reachable");
    let request = ConfirmedTargetSearchRequest {
        seed,
        confirmed_frame1: 480,
        timeline_generation: 0,
        source_config: Some(config),
        start: ConfirmedTimeleapStart {
            frame: candidate.post_frame,
            sfmt_position: candidate.post_sfmt_position,
            tick: candidate.post_tick,
            status: candidate.post_status.clone(),
        },
        baseline_target_frame: candidate.post_frame,
        generation_start_frame: candidate.post_frame,
        drift_search_generation_start_frame: None,
        drift_range_ticks: 0,
        grace_min_frames: 0,
        consider_delay: None,
    };
    let rows = run_confirmed_target_search(&request, &cache, &AtomicBool::new(false))
        .expect("TargetFrame candidates should use the rebuilt post-Frame1 state");
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|row| row.frame1 == 480));
}

#[test]
fn target_search_without_consider_delay_ignores_event_delay_flags() {
    let seed = 0xA02D_5C74;
    let cache =
        ObservationCache::from_seed_for_test(seed, 10_000, crate::domain::timing::DEFAULT_FPS);
    let courier_cases = [
        CourierConfig::default(),
        CourierConfig {
            no_dex: true,
            ..CourierConfig::default()
        },
        CourierConfig {
            your_id: true,
            ..CourierConfig::default()
        },
        CourierConfig {
            your_id: true,
            is_egg: true,
            ..CourierConfig::default()
        },
    ];
    let mut expected_rows = None;
    let mut expected_transition = None;

    for courier in courier_cases {
        let config = TimeleapConfig {
            seed,
            start_frame: 480,
            target_frame: 10_000,
            models: 2,
            min_frame1_elapsed_ticks: 0,
            max_frame1_elapsed_ticks: 0,
            mode: TimeleapMode::Courier,
            is_boy: true,
            handoff_range_frames: 0,
            courier,
        };
        let preparation = run_timeleap_frame1_preparation(
            &config,
            480,
            &TimeleapTimelineStart::initial(480, 2),
            &cache,
            &AtomicBool::new(false),
        )
        .expect("Frame1 should be simulated from explicit courier inputs");
        let candidate = preparation
            .candidates
            .iter()
            .find(|candidate| candidate.frame1 == 480)
            .expect("initial Frame1 is reachable");
        let transition = (
            candidate.post_frame,
            candidate.post_sfmt_position,
            candidate.post_tick,
            candidate.post_status.clone(),
        );
        if let Some(expected) = expected_transition.as_ref() {
            assert_eq!(
                &transition, expected,
                "Timeleap's post-event state must not depend on Consider Delay flags"
            );
        } else {
            expected_transition = Some(transition);
        }

        let request = ConfirmedTargetSearchRequest {
            seed,
            confirmed_frame1: 480,
            timeline_generation: 0,
            source_config: Some(config),
            start: ConfirmedTimeleapStart {
                frame: candidate.post_frame,
                sfmt_position: candidate.post_sfmt_position,
                tick: candidate.post_tick,
                status: candidate.post_status.clone(),
            },
            baseline_target_frame: candidate.post_frame,
            generation_start_frame: candidate.post_frame,
            drift_search_generation_start_frame: None,
            drift_range_ticks: 0,
            grace_min_frames: 0,
            consider_delay: None,
        };
        let rows = run_confirmed_target_search(&request, &cache, &AtomicBool::new(false))
            .expect("TargetFrame search without Consider Delay");
        assert!(!rows.is_empty(), "the exact Timeleap landing is reachable");
        if let Some(expected) = expected_rows.as_ref() {
            assert_eq!(
                &rows, expected,
                "TargetFrame results without Consider Delay must ignore No Dex / ID / egg flags"
            );
        } else {
            expected_rows = Some(rows);
        }
    }
}

#[test]
fn confirmed_target_search_applies_consider_delay_to_isolated_candidate_states() {
    let seed = 0xA02D_5C72;
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=10_000,
        10_000,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate cache for delayed confirmed Target search");
    let start = ConfirmedTimeleapStart {
        frame: 500,
        sfmt_position: 500,
        tick: 100,
        status: TimeleapStatus::new(2),
    };
    let mut status = start.status.clone();
    let mut cursor = Cursor {
        frame: start.frame,
        sfmt_position: start.sfmt_position,
        tick: start.tick,
    };
    let mut sfmt = cached_sfmt_at(&cache, cursor.sfmt_position)
        .expect("confirmed cursor has cached SFMT state");
    for _ in 0..8 {
        advance_one_tick(&mut status, &mut cursor, &mut sfmt);
    }
    let baseline_target_frame = cursor.frame;
    let mut expected_status = start.status.clone();
    let mut expected_cursor = Cursor {
        frame: start.frame,
        sfmt_position: start.sfmt_position,
        tick: start.tick,
    };
    let mut expected_sfmt = cached_sfmt_at(&cache, expected_cursor.sfmt_position)
        .expect("confirmed cursor has cached SFMT state");
    for _ in 0..2 {
        advance_one_tick(
            &mut expected_status,
            &mut expected_cursor,
            &mut expected_sfmt,
        );
    }
    let generation_start_frame = expected_cursor.frame;
    let request = ConfirmedTargetSearchRequest {
        seed,
        confirmed_frame1: 480,
        timeline_generation: 9,
        source_config: None,
        start: start.clone(),
        baseline_target_frame,
        generation_start_frame,
        drift_search_generation_start_frame: None,
        drift_range_ticks: 16,
        grace_min_frames: 0,
        consider_delay: Some(ConsiderDelayConfig {
            delay_frames: 0,
            generator: ConsiderDelayGenerator::FixedSymbol(
                crate::domain::fixed_rng_catalog::StationaryDelayType::Normal,
            ),
            courier: CourierConfig::default(),
        }),
    };
    let rows = run_confirmed_target_search(&request, &cache, &AtomicBool::new(false))
        .expect("Consider Delay candidate simulation");
    assert!(rows.iter().any(|row| {
        row.target_frame == start.frame
            && row.generation_start_frame == Some(generation_start_frame)
    }));
    assert_eq!(request.start, start, "delay simulation must use a clone");

    let mut drift_search = request.clone();
    drift_search.drift_search_generation_start_frame =
        Some(generation_start_frame.saturating_add(1));
    let drift_rows = run_confirmed_target_search(&drift_search, &cache, &AtomicBool::new(false))
        .expect("drift display range keeps the Consider Delay candidate simulation");
    assert!(drift_rows.len() <= (drift_search.drift_range_ticks * 2 + 1) as usize);
    assert!(drift_rows
        .iter()
        .any(|row| { row.generation_start_frame == Some(generation_start_frame) }));
    assert!(drift_rows.iter().all(|row| row.drift_frames % 2 == 0));
}

#[test]
fn standalone_reachability_check_uses_entered_start_frame1_and_generation_frame() {
    let seed = 0x436f_7e6c;
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=10_000,
        10_000,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate cache for standalone reachability check");
    let start_frame = 500;
    let frame1 = 500;
    let config = TimeleapConfig {
        seed,
        start_frame,
        target_frame: 5_000,
        models: 4,
        min_frame1_elapsed_ticks: 0,
        max_frame1_elapsed_ticks: 0,
        mode: TimeleapMode::Menu,
        is_boy: true,
        handoff_range_frames: 0,
        courier: CourierConfig::default(),
    };
    let prepared = run_timeleap_frame1_preparation(
        &config,
        frame1,
        &TimeleapTimelineStart::initial(start_frame, config.models),
        &cache,
        &AtomicBool::new(false),
    )
    .expect("Frame1 is the explicit starting frame");
    let candidate = prepared
        .candidates
        .first()
        .expect("the starting Frame1 produces a candidate");
    let mut status = candidate.post_status.clone();
    let mut cursor = Cursor {
        frame: candidate.post_frame,
        sfmt_position: candidate.post_sfmt_position,
        tick: candidate.post_tick,
    };
    let mut sfmt = cached_sfmt_at(&cache, cursor.sfmt_position).expect("cached candidate SFMT");
    for _ in 0..40 {
        advance_one_tick(&mut status, &mut cursor, &mut sfmt);
    }
    let generation_frame = cursor.frame;
    assert!(generation_frame > candidate.post_frame);

    let request = TimeleapReachabilityRequest {
        config: TimeleapConfig {
            target_frame: generation_frame,
            ..config.clone()
        },
        frame1,
        consider_delay: None,
    };
    assert_eq!(
        run_timeleap_reachability_check(&request, &cache, &AtomicBool::new(false))
            .expect("reachability calculation"),
        TimeleapReachability::Reachable {
            target_input_frames: vec![generation_frame]
        }
    );

    let delay = ConsiderDelayConfig {
        delay_frames: 0,
        generator: ConsiderDelayGenerator::FixedSymbol(
            crate::domain::fixed_rng_catalog::StationaryDelayType::Normal,
        ),
        courier: CourierConfig::default(),
    };
    let mut delay_status = candidate.post_status.clone();
    let mut delay_cursor = Cursor {
        frame: candidate.post_frame,
        sfmt_position: candidate.post_sfmt_position,
        tick: candidate.post_tick,
    };
    let mut delay_sfmt =
        cached_sfmt_at(&cache, delay_cursor.sfmt_position).expect("cached Frame1 post-event SFMT");
    let _ = apply_consider_delay(
        &delay,
        &mut delay_status,
        &mut delay_cursor,
        &mut delay_sfmt,
        config.models,
        10_000,
    )
    .expect("simulate Consider Delay from the known A-input point");
    let delayed_generation_frame = delay_cursor.frame;
    assert!(delayed_generation_frame > candidate.post_frame);
    let delayed_request = TimeleapReachabilityRequest {
        config: TimeleapConfig {
            target_frame: delayed_generation_frame,
            ..config.clone()
        },
        frame1,
        consider_delay: Some(delay),
    };
    let delayed_result =
        run_timeleap_reachability_check(&delayed_request, &cache, &AtomicBool::new(false))
            .expect("Consider Delay reachability calculation");
    let TimeleapReachability::Reachable {
        target_input_frames,
    } = delayed_result
    else {
        panic!("the known Consider Delay input should reach its generation Frame");
    };
    assert!(target_input_frames.contains(&candidate.post_frame));

    let unreachable_request = TimeleapReachabilityRequest {
        frame1: start_frame - 1,
        ..request
    };
    assert_eq!(
        run_timeleap_reachability_check(&unreachable_request, &cache, &AtomicBool::new(false))
            .expect("unreachable Frame1 is a normal result"),
        TimeleapReachability::Frame1Unreachable
    );
}

#[test]
fn target_indicator_reachability_starts_after_the_selected_frame1_event_without_delay() {
    let seed = 0x436f_7e6c;
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=10_000,
        10_000,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate cache for post-Frame1 target check");
    let config = TimeleapConfig {
        seed,
        start_frame: 500,
        target_frame: 5_000,
        models: 4,
        min_frame1_elapsed_ticks: 0,
        max_frame1_elapsed_ticks: 0,
        mode: TimeleapMode::Menu,
        is_boy: true,
        handoff_range_frames: 0,
        courier: CourierConfig::default(),
    };
    let frame1 = 500;
    let prepared = run_timeleap_frame1_preparation(
        &config,
        frame1,
        &TimeleapTimelineStart::initial(config.start_frame, config.models),
        &cache,
        &AtomicBool::new(false),
    )
    .expect("prepare the exact Frame1 event");
    let candidate = prepared
        .candidates
        .iter()
        .find(|candidate| candidate.frame1 == frame1)
        .expect("the exact Frame1 candidate exists");

    // The event's own post-Frame1 location is reachable, while a target
    // already passed by the event is not. The check API intentionally has
    // no ConsiderDelay parameter, so the indicator cannot include it.
    assert!(timeleap_target_reachable_after_frame1(
        &prepared,
        frame1,
        candidate.post_frame,
        &cache,
        &AtomicBool::new(false),
    )
    .expect("check exact event landing"));
    if candidate.post_frame > frame1 {
        assert!(!timeleap_target_reachable_after_frame1(
            &prepared,
            frame1,
            candidate.post_frame - 1,
            &cache,
            &AtomicBool::new(false),
        )
        .expect("a target skipped by the event is unreachable"));
    }
    assert!(!timeleap_target_reachable_after_frame1(
        &prepared,
        frame1 + 1,
        candidate.post_frame,
        &cache,
        &AtomicBool::new(false),
    )
    .expect("a Frame1 absent from the selected preparation is unreachable"));
}

#[test]
fn standalone_reachability_check_supports_each_timeleap_event_mode() {
    let seed = 0x436f_7e6c;
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=10_000,
        10_000,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate cache for each event mode");

    for mode in [
        TimeleapMode::Menu,
        TimeleapMode::Dialogue,
        TimeleapMode::Courier,
    ] {
        let config = TimeleapConfig {
            seed,
            start_frame: 500,
            target_frame: 5_000,
            models: 4,
            min_frame1_elapsed_ticks: 0,
            max_frame1_elapsed_ticks: 0,
            mode,
            is_boy: true,
            handoff_range_frames: 0,
            courier: CourierConfig::default(),
        };
        let preparation = run_timeleap_frame1_preparation(
            &config,
            500,
            &TimeleapTimelineStart::initial(500, config.models),
            &cache,
            &AtomicBool::new(false),
        )
        .expect("Frame1 should be reachable for each event mode");
        let candidate = preparation
            .candidates
            .iter()
            .find(|candidate| candidate.frame1 == 500)
            .expect("exact Frame1 candidate");
        let mut status = candidate.post_status.clone();
        let mut cursor = Cursor {
            frame: candidate.post_frame,
            sfmt_position: candidate.post_sfmt_position,
            tick: candidate.post_tick,
        };
        let mut sfmt =
            cached_sfmt_at(&cache, cursor.sfmt_position).expect("cached post-event state");
        for _ in 0..20 {
            advance_one_tick(&mut status, &mut cursor, &mut sfmt);
        }
        let generation_frame = cursor.frame;
        let request = TimeleapReachabilityRequest {
            config: TimeleapConfig {
                target_frame: generation_frame,
                ..config
            },
            frame1: 500,
            consider_delay: None,
        };

        assert_eq!(
            run_timeleap_reachability_check(&request, &cache, &AtomicBool::new(false))
                .expect("standalone reachability check"),
            TimeleapReachability::Reachable {
                target_input_frames: vec![generation_frame]
            },
            "event mode {mode:?} should preserve its exact generation Frame"
        );
    }
}
