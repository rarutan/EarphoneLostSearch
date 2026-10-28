use super::*;

#[test]
fn shared_scan_cache_reserves_same_key_once_and_keeps_distinct_keys_independent() {
    let cache = Arc::new(SharedConsiderDelayScanCache::new(3));
    let key = shared_scan_test_key(100);
    let owner = match cache.lookup_or_reserve(&key) {
        SharedMemoLookup::Owner(entry) => entry,
        _ => panic!("first lookup must reserve the key"),
    };

    let (waiting_tx, waiting_rx) = std::sync::mpsc::channel();
    let waiter_cache = cache.clone();
    let waiter_key = key.clone();
    let waiter = std::thread::spawn(move || {
        let entry = match waiter_cache.lookup_or_reserve(&waiter_key) {
            SharedMemoLookup::Wait(entry) => entry,
            _ => panic!("same-key lookup must wait for its owner"),
        };
        waiting_tx.send(()).expect("notify owner");
        waiter_cache.wait(&entry, None).expect("wait for result")
    });

    waiting_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("waiter reached the in-flight entry");
    let owner_compute_count = AtomicUsize::new(0);
    owner_compute_count.fetch_add(1, Ordering::Relaxed);
    let mut path = PendingSharedScanPath::default();
    path.claim(owner, 100);
    path.complete_with(|_| None);
    assert!(matches!(
        waiter.join().expect("waiter thread"),
        SharedMemoWait::Ready(None)
    ));
    assert_eq!(owner_compute_count.load(Ordering::Relaxed), 1);

    let distinct_owner_a = match cache.lookup_or_reserve(&shared_scan_test_key(200)) {
        SharedMemoLookup::Owner(entry) => entry,
        _ => panic!("a different key must be reservable while other work is pending"),
    };
    let distinct_owner_b = match cache.lookup_or_reserve(&shared_scan_test_key(300)) {
        SharedMemoLookup::Owner(entry) => entry,
        _ => panic!("different keys must not wait on each other"),
    };
    let mut distinct_path = PendingSharedScanPath::default();
    distinct_path.claim(distinct_owner_a, 200);
    distinct_path.claim(distinct_owner_b, 300);
    distinct_path.complete_with(|_| None);
    assert_eq!(cache.len(), 3);
}

#[test]
fn shared_memo_caches_respect_one_search_wide_byte_budget() {
    let budget = Arc::new(CacheMemoryBudget::new(16));
    let scan_cache = SharedConsiderDelayScanCache::with_budget(budget.clone(), 8);
    let suffix_cache = SharedSingleFlightCache::<u8, u8>::with_budget(budget.clone(), 8);

    assert!(matches!(
        scan_cache.lookup_or_reserve(&shared_scan_test_key(100)),
        SharedMemoLookup::Owner(_)
    ));
    assert!(matches!(
        suffix_cache.lookup_or_reserve(&1),
        SharedMemoLookup::Owner(_)
    ));
    assert_eq!(budget.peak_bytes(), 16);
    assert!(matches!(
        suffix_cache.lookup_or_reserve(&2),
        SharedMemoLookup::Uncached
    ));
    assert!(matches!(
        scan_cache.lookup_or_reserve(&shared_scan_test_key(100)),
        SharedMemoLookup::Wait(_)
    ));
    assert_eq!(scan_cache.miss_count(), 1);
    assert_eq!(suffix_cache.miss_count(), 2);
    assert_eq!(suffix_cache.bypass_count(), 1);
    assert_eq!(scan_cache.hit_count(), 0);
}

#[test]
fn shared_scan_cache_wait_is_cancelable_and_abandoned_entries_can_be_retried() {
    let cache = Arc::new(SharedConsiderDelayScanCache::new(1));
    let key = shared_scan_test_key(100);
    let owner = match cache.lookup_or_reserve(&key) {
        SharedMemoLookup::Owner(entry) => entry,
        _ => panic!("first lookup must reserve the key"),
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let waiter_cache = cache.clone();
    let waiter_cancel = cancel.clone();
    let waiter_key = key.clone();
    let waiter = std::thread::spawn(move || {
        let entry = match waiter_cache.lookup_or_reserve(&waiter_key) {
            SharedMemoLookup::Wait(entry) => entry,
            _ => panic!("same-key lookup must wait for its owner"),
        };
        waiter_cache.wait(&entry, Some(&waiter_cancel))
    });

    while cache.wait_count() == 0 {
        std::thread::yield_now();
    }
    cancel.store(true, Ordering::Release);
    assert!(waiter.join().expect("waiter thread").is_err());

    let mut abandoned_path = PendingSharedScanPath::default();
    abandoned_path.claim(owner, 100);
    drop(abandoned_path);
    assert!(matches!(
        cache.lookup_or_reserve(&key),
        SharedMemoLookup::Owner(_)
    ));
    assert!(matches!(
        cache.lookup_or_reserve(&shared_scan_test_key(200)),
        SharedMemoLookup::Uncached
    ));
}

#[test]
fn safe_frame_tick_matches_reference_transition() {
    let cases = [
        (0x1234_5678, vec![0, 1, 0, 1], -1, false),
        (0x7627_5c90, vec![1, 0, 1, 0, 0], 4, false),
        (0xb248_ea3b, vec![0, 1, 0, 1, 0, 1], -1, true),
        (0x436f_7e6c, vec![0, -5, 1, 2, 0, 1], 2, true),
    ];

    for (seed, remain, fidget_cd, raining) in cases {
        let mut fast_status = TimeleapStatus::new(remain.len());
        fast_status.model.remain = remain;
        fast_status.fidget_cd = fidget_cd;
        fast_status.raining = raining;
        fast_status.phase = 1;
        fast_status.is_boy = true;
        let mut reference_status = fast_status.clone();
        let mut fast_cursor = Cursor {
            frame: 10_000,
            sfmt_position: 10_000,
            tick: 37,
        };
        let mut reference_cursor = fast_cursor;
        let mut fast_sfmt = Sfmt::new(seed);
        let mut reference_sfmt = fast_sfmt.clone();
        let active_models = fast_status.model.remain.len();
        let mut saw_fast_tick = false;
        let mut saw_minus_five_entry = false;
        let mut saw_minus_five_resolution = false;

        for _ in 0..512 {
            let expected_safe = fast_status.fidget_cd != 1
                && fast_status.model.remain[..active_models]
                    .iter()
                    .all(|remain| matches!(*remain, 0 | 1));
            begin_model_rng_trace();
            let (fast_blinks, used_fast_path) = advance_one_tick_with_active_models_traced(
                &mut fast_status,
                &mut fast_cursor,
                &mut fast_sfmt,
                active_models,
            );
            let fast_trace = finish_model_rng_trace();
            assert_eq!(used_fast_path, expected_safe, "seed {seed:08x}");
            saw_fast_tick |= used_fast_path;
            saw_minus_five_entry |= fast_trace
                .iter()
                .any(|event| matches!(event.kind, ModelRngEventKind::EnteredMinusFive { .. }));
            saw_minus_five_resolution |= fast_trace
                .iter()
                .any(|event| matches!(event.kind, ModelRngEventKind::ResolvedMinusFive { .. }));

            begin_model_rng_trace();
            let reference_blinks = advance_one_tick_reference(
                &mut reference_status,
                &mut reference_cursor,
                &mut reference_sfmt,
            );
            let reference_trace = finish_model_rng_trace();

            assert_eq!(fast_status, reference_status, "seed {seed:08x}");
            assert_eq!(fast_cursor, reference_cursor, "seed {seed:08x}");
            assert_eq!(fast_sfmt.state, reference_sfmt.state, "seed {seed:08x}");
            assert_eq!(fast_sfmt.index, reference_sfmt.index, "seed {seed:08x}");
            assert_eq!(fast_blinks, reference_blinks, "seed {seed:08x}");
            assert_eq!(fast_trace, reference_trace, "seed {seed:08x}");

            if used_fast_path {
                assert!(fast_blinks.is_empty());
            }
        }
        assert!(saw_fast_tick, "seed {seed:08x} did not exercise Safe Frame");
        assert!(
            saw_minus_five_entry,
            "seed {seed:08x} did not enter remain=-5"
        );
        assert!(
            saw_minus_five_resolution,
            "seed {seed:08x} did not resolve remain=-5"
        );
    }
}

#[test]
fn safe_frame_tick_updates_only_active_models_during_delay() {
    let mut initial = TimeleapStatus::new(5);
    initial.model.remain = vec![0, 1, 12, -3, 1];
    initial.raining = true;
    initial.phase = 1;
    initial.fidget_cd = 5;
    let mut fast_status = initial.clone();
    let mut reference_status = initial;
    let mut fast_cursor = Cursor {
        frame: 900,
        sfmt_position: 900,
        tick: 2,
    };
    let mut reference_cursor = fast_cursor;
    let mut fast_sfmt = Sfmt::new(0x8322_38a9);
    let mut reference_sfmt = fast_sfmt.clone();

    for _ in 0..64 {
        let _ = advance_delay_tick(&mut fast_status, &mut fast_cursor, &mut fast_sfmt, 2);
        advance_delay_tick_reference(
            &mut reference_status,
            &mut reference_cursor,
            &mut reference_sfmt,
            2,
        );

        assert_eq!(fast_status, reference_status);
        assert_eq!(fast_cursor, reference_cursor);
        assert_eq!(fast_sfmt.state, reference_sfmt.state);
        assert_eq!(fast_sfmt.index, reference_sfmt.index);
    }
}

#[test]
#[ignore = "手動性能測定用。cargo test safe_frame_fast_path_benchmark --release -- --ignored --nocapture で実行"]
fn safe_frame_fast_path_benchmark() {
    use std::hint::black_box;
    use std::time::Instant;

    const TICKS: usize = 1_000_000;
    for models in [1, 15, 22] {
        let mut fast_status = TimeleapStatus::new(models);
        let mut reference_status = fast_status.clone();
        let mut fast_cursor = Cursor {
            frame: 50_000,
            sfmt_position: 50_000,
            tick: 0,
        };
        let mut reference_cursor = fast_cursor;
        let mut fast_sfmt = Sfmt::new(0x6a31_9d27);
        let mut reference_sfmt = fast_sfmt.clone();
        let mut safe_ticks = 0usize;
        let active_models = models;

        let fast_start = Instant::now();
        for _ in 0..TICKS {
            let (_, used_fast_path) = advance_one_tick_with_active_models(
                &mut fast_status,
                &mut fast_cursor,
                &mut fast_sfmt,
                active_models,
            );
            safe_ticks += usize::from(used_fast_path);
            black_box(fast_cursor.frame);
        }
        let fast_elapsed = fast_start.elapsed();

        let reference_start = Instant::now();
        for _ in 0..TICKS {
            advance_one_tick_reference(
                &mut reference_status,
                &mut reference_cursor,
                &mut reference_sfmt,
            );
            black_box(reference_cursor.frame);
        }
        let reference_elapsed = reference_start.elapsed();

        assert_eq!(fast_status, reference_status);
        assert_eq!(fast_cursor, reference_cursor);
        assert_eq!(fast_sfmt.state, reference_sfmt.state);
        assert_eq!(fast_sfmt.index, reference_sfmt.index);
        println!(
                "models={models} ticks={TICKS} safe_ticks={safe_ticks} fast={fast_elapsed:?} reference={reference_elapsed:?}"
            );
    }
}

#[test]
fn consider_delay_parallel_search_matches_serial_for_single_and_multiple_workers() {
    use crate::domain::fixed_rng_catalog::StationaryDelayType as DelayType;

    let seed = 0x6a31_9d27;
    let cache_end = 50_000;
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=cache_end,
        cache_end,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate cache for parallel equivalence test");
    let mut config = TimeleapConfig {
        seed,
        start_frame: 500,
        target_frame: 10_000,
        models: 2,
        min_frame1_elapsed_ticks: 8,
        max_frame1_elapsed_ticks: 20,
        mode: TimeleapMode::Menu,
        is_boy: true,
        handoff_range_frames: 0,
        courier: CourierConfig::default(),
    };
    let delay = ConsiderDelayConfig {
        delay_frames: 0,
        generator: ConsiderDelayGenerator::FixedSymbol(DelayType::FirstKartana),
        courier: CourierConfig::default(),
    };

    // Construct a reachable target from one Frame1 candidate so the comparison
    // checks a non-empty result rather than only matching empty lists.
    let mut status = TimeleapStatus::new(config.models);
    let mut cursor = Cursor {
        frame: config.start_frame,
        sfmt_position: config.start_frame,
        tick: 0,
    };
    let mut sfmt = cached_sfmt_at(&cache, cursor.sfmt_position).expect("initial SFMT");
    for _ in 0..12 {
        advance_one_tick(&mut status, &mut cursor, &mut sfmt);
    }
    apply_event(
        &mut status,
        &mut cursor,
        &mut sfmt,
        config.mode,
        &config.courier,
    );
    let _ = apply_consider_delay(
        &delay,
        &mut status,
        &mut cursor,
        &mut sfmt,
        config.models,
        cache_end,
    )
    .expect("simulate target candidate");
    assert!(cursor.frame < cache_end);
    config.target_frame = cursor.frame;
    let start = TimeleapTimelineStart::initial(config.start_frame, config.models);

    let serial = search_candidates_from_timeline_with_stats(
        &config,
        &start,
        0,
        &cache,
        None,
        true,
        Some(&delay),
        false,
    )
    .expect("serial candidate search")
    .0;
    assert!(serial
        .iter()
        .flat_map(|group| &group.candidates)
        .any(|candidate| { candidate.frame1_tick == 12 }));

    for worker_count in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(worker_count)
            .build()
            .expect("build local Rayon pool");
        let (parallel, parallel_stats) = pool
            .install(|| {
                search_candidates_from_timeline_with_stats(
                    &config,
                    &start,
                    0,
                    &cache,
                    None,
                    true,
                    Some(&delay),
                    true,
                )
            })
            .expect("parallel candidate search");
        assert_eq!(parallel, serial, "worker_count={worker_count}");
        assert!(parallel_stats.candidate_ticks_simulated > 0);
    }

    let cancelled = AtomicBool::new(true);
    assert!(search_candidates_from_timeline_with_stats(
        &config,
        &start,
        0,
        &cache,
        Some(&cancelled),
        true,
        Some(&delay),
        true,
    )
    .is_err());
}

#[test]
fn no_dex_event7_parallel_search_matches_serial_candidates() {
    let seed = 0x7627_5c90;
    let cache_end = 20_000;
    let cache = ObservationCache::from_seed_for_test(
        seed,
        cache_end + 311,
        crate::domain::timing::DEFAULT_FPS,
    );
    let mut config = TimeleapConfig {
        seed,
        start_frame: 500,
        target_frame: cache_end,
        models: 2,
        min_frame1_elapsed_ticks: 8,
        max_frame1_elapsed_ticks: 20,
        mode: TimeleapMode::Menu,
        is_boy: true,
        handoff_range_frames: 0,
        courier: CourierConfig::default(),
    };
    let delay = ConsiderDelayConfig {
        delay_frames: 42,
        generator: ConsiderDelayGenerator::Event,
        courier: CourierConfig {
            no_dex: true,
            ..CourierConfig::default()
        },
    };
    config.courier.no_dex = true;

    let mut status = TimeleapStatus::new(config.models);
    let mut cursor = Cursor {
        frame: config.start_frame,
        sfmt_position: config.start_frame,
        tick: 0,
    };
    let mut sfmt = cached_sfmt_at(&cache, cursor.sfmt_position).expect("initial SFMT");
    for _ in 0..12 {
        advance_one_tick(&mut status, &mut cursor, &mut sfmt);
    }
    apply_event(
        &mut status,
        &mut cursor,
        &mut sfmt,
        config.mode,
        &config.courier,
    );
    let _ = apply_consider_delay(
        &delay,
        &mut status,
        &mut cursor,
        &mut sfmt,
        config.models,
        cache_end,
    )
    .expect("simulate No Dex Event7 target");
    assert!(cursor.frame < cache_end);
    config.target_frame = cursor.frame;
    let start = TimeleapTimelineStart::initial(config.start_frame, config.models);

    let (serial, serial_stats) = search_candidates_from_timeline_with_stats(
        &config,
        &start,
        0,
        &cache,
        None,
        true,
        Some(&delay),
        false,
    )
    .expect("serial No Dex candidate search");
    assert_eq!(serial_stats.core_frame1_origin_ticks, 13);
    assert_eq!(serial_stats.scope_resolution_ticks, 8);
    assert!(serial
        .iter()
        .flat_map(|group| &group.candidates)
        .any(|candidate| candidate.frame1_tick == 12));

    for worker_count in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(worker_count)
            .build()
            .expect("build local Rayon pool");
        let (parallel, parallel_stats) = pool
            .install(|| {
                search_candidates_from_timeline_with_stats(
                    &config,
                    &start,
                    0,
                    &cache,
                    None,
                    true,
                    Some(&delay),
                    true,
                )
            })
            .expect("parallel No Dex candidate search");
        assert_eq!(
            parallel_stats.core_frame1_origin_ticks, serial_stats.core_frame1_origin_ticks,
            "parallel No Dex keeps the configured core origin count"
        );
        assert_eq!(parallel, serial, "worker_count={worker_count}");
    }

    let mut ordinary_config = config.clone();
    ordinary_config.courier.no_dex = false;
    let ordinary_delay = ConsiderDelayConfig {
        courier: CourierConfig::default(),
        ..delay
    };
    let (_, ordinary_stats) = search_candidates_from_timeline_with_stats(
        &ordinary_config,
        &start,
        0,
        &cache,
        None,
        true,
        Some(&ordinary_delay),
        true,
    )
    .expect("ordinary Consider Delay candidate search");
    assert_eq!(
        ordinary_stats.core_frame1_origin_ticks, serial_stats.core_frame1_origin_ticks,
        "No Dex changes Consider Delay outcomes, not the core Frame1 origin range"
    );
    assert_eq!(
        ordinary_stats.scope_resolution_ticks,
        serial_stats.scope_resolution_ticks
    );
}

#[test]
#[ignore = "Consider Delay landing-order and state-transition diagnostic sweep"]
fn consider_delay_overshoot_cutoff_tracks_blink_state_transitions() {
    use crate::domain::fixed_rng_catalog::StationaryDelayType as DelayType;

    struct Sample {
        name: &'static str,
        seed: u32,
        models: usize,
        mode: TimeleapMode,
        delay: ConsiderDelayConfig,
    }

    let samples = [
        Sample {
            name: "First Kartana / 22 models / changing active count",
            seed: 0x436f_7e6c,
            models: 22,
            mode: TimeleapMode::Menu,
            delay: ConsiderDelayConfig {
                delay_frames: 0,
                generator: ConsiderDelayGenerator::FixedSymbol(DelayType::FirstKartana),
                courier: CourierConfig::default(),
            },
        },
        Sample {
            name: "First Xurkitree / 22 models / long profile",
            seed: 0x7627_5c90,
            models: 22,
            mode: TimeleapMode::Menu,
            delay: ConsiderDelayConfig {
                delay_frames: 400,
                generator: ConsiderDelayGenerator::FixedSymbol(DelayType::FirstXurkitree),
                courier: CourierConfig::default(),
            },
        },
        Sample {
            name: "Common legendary / 15 models / active-count reduction",
            seed: 0x8322_38a9,
            models: 15,
            mode: TimeleapMode::Dialogue,
            delay: ConsiderDelayConfig {
                delay_frames: 80,
                generator: ConsiderDelayGenerator::FixedSymbol(DelayType::CommonLegendary),
                courier: CourierConfig::default(),
            },
        },
        Sample {
            name: "Courier No Dex / 15 models",
            seed: 0xb248_ea3b,
            models: 15,
            mode: TimeleapMode::Courier,
            delay: ConsiderDelayConfig {
                delay_frames: 120,
                generator: ConsiderDelayGenerator::Event,
                courier: CourierConfig {
                    no_dex: true,
                    ..CourierConfig::default()
                },
            },
        },
    ];

    let candidate_ticks = 256;
    let cache_end = 100_000;
    let sample_count = samples.len();
    let mut total_entered_minus_five = 0;
    let mut total_resolved_minus_five = 0;
    let mut total_landing_drops = 0;

    for sample in samples {
        let cache = ObservationCache::from_seed_for_test(
            sample.seed,
            cache_end,
            crate::domain::timing::DEFAULT_FPS,
        );

        let mut status = TimeleapStatus::new(sample.models);
        status.is_boy = true;
        status.raining = true;
        let mut cursor = Cursor {
            frame: 480,
            sfmt_position: 480,
            tick: 0,
        };
        let mut sfmt = cached_sfmt_at(&cache, cursor.sfmt_position)
            .unwrap_or_else(|error| panic!("{}: initial SFMT: {error}", sample.name));
        apply_event(
            &mut status,
            &mut cursor,
            &mut sfmt,
            sample.mode,
            &sample.delay.courier,
        );

        let mut landings = Vec::with_capacity(candidate_ticks);
        let mut landing_sfmt_positions = Vec::with_capacity(candidate_ticks);
        let mut input_frames = Vec::with_capacity(candidate_ticks);
        let mut final_remains = Vec::with_capacity(candidate_ticks);
        let mut trace_by_candidate = Vec::with_capacity(candidate_ticks);
        let mut candidates_with_state_events = 0;
        for _ in 0..candidate_ticks {
            let mut trial_status = status.clone();
            let mut trial_cursor = cursor;
            let mut trial_sfmt = sfmt.clone();

            begin_model_rng_trace();
            // The cap is intentionally far beyond this candidate's delay
            // so this measures the completed landing, not the search's
            // target-pruned partial simulation.
            let _ = apply_consider_delay_traced(
                &sample.delay,
                &mut trial_status,
                &mut trial_cursor,
                &mut trial_sfmt,
                sample.models,
                cache_end - 1_000,
            )
            .unwrap_or_else(|error| panic!("{}: delay simulation: {error}", sample.name));
            let events = finish_model_rng_trace();
            candidates_with_state_events += usize::from(!events.is_empty());
            total_entered_minus_five += events
                .iter()
                .filter(|event| matches!(event.kind, ModelRngEventKind::EnteredMinusFive { .. }))
                .count();
            total_resolved_minus_five += events
                .iter()
                .filter(|event| matches!(event.kind, ModelRngEventKind::ResolvedMinusFive { .. }))
                .count();
            landings.push(trial_cursor.frame);
            landing_sfmt_positions.push(trial_cursor.sfmt_position);
            input_frames.push(cursor.frame);
            final_remains.push(trial_status.model.remain.clone());
            trace_by_candidate.push(events);

            advance_one_tick(&mut status, &mut cursor, &mut sfmt);
        }

        let mut first_drop = None;
        let mut prefix_max = landings[0];
        let mut prefix_max_index = 0;
        for (index, landing) in landings.iter().copied().enumerate().skip(1) {
            if landing < prefix_max {
                first_drop = Some((prefix_max_index, index, prefix_max, landing));
                break;
            }
            if landing > prefix_max {
                prefix_max = landing;
                prefix_max_index = index;
            }
        }

        if let Some((high_index, low_index, high, low)) = first_drop {
            total_landing_drops += 1;
            let transition_events: Vec<_> = trace_by_candidate[high_index..=low_index]
                .iter()
                .flatten()
                .copied()
                .collect();
            let enter_count = transition_events
                .iter()
                .filter(|event| matches!(event.kind, ModelRngEventKind::EnteredMinusFive { .. }))
                .count();
            let resolve_count = transition_events
                .iter()
                .filter(|event| matches!(event.kind, ModelRngEventKind::ResolvedMinusFive { .. }))
                .count();
            let first_over_target = landings
                .iter()
                .position(|landing| *landing > low)
                .unwrap_or(usize::MAX);
            let input_scan_end = input_frames
                .iter()
                .position(|frame| *frame > low)
                .unwrap_or(candidate_ticks);
            let event_aware_stop = (0..input_scan_end)
                .find(|index| landings[*index] > low && trace_by_candidate[*index].is_empty());
            let crossing_events = trace_by_candidate
                .get(first_over_target)
                .into_iter()
                .flatten()
                .filter(|event| {
                    event.sfmt_position > low && event.sfmt_position <= landings[first_over_target]
                })
                .copied()
                .collect::<Vec<_>>();
            let first_event_difference = trace_by_candidate[high_index]
                .iter()
                .zip(&trace_by_candidate[low_index])
                .position(|(left, right)| left != right);
            let first_event_difference_pair = first_event_difference.map(|index| {
                (
                    trace_by_candidate[high_index].get(index).copied(),
                    trace_by_candidate[low_index].get(index).copied(),
                )
            });
            let remain_differences = final_remains[high_index]
                .iter()
                .zip(&final_remains[low_index])
                .enumerate()
                .filter_map(|(model, (left, right))| {
                    (left != right).then_some((model, *left, *right))
                })
                .take(8)
                .collect::<Vec<_>>();
            assert_eq!(
                first_over_target, high_index,
                "{}: the earlier landing must be the first over-target result for Target={low}",
                sample.name
            );
            assert_eq!(
                landings[low_index], low,
                "{}: exhaustive later candidate must exactly reach Target={low}",
                sample.name
            );
            assert!(
                    event_aware_stop.is_none_or(|stop_index| stop_index > low_index),
                    "{}: checking all three event signals along the full Consider Delay path stopped before a later exact Target candidate",
                    sample.name
                );
            assert!(
                enter_count > 0 && resolve_count > 0,
                "{}: the two delay paths must exercise both %128 and -5/%3 transitions",
                sample.name
            );
            assert!(
                    crossing_events.is_empty(),
                    "{}: this counterexample is intended to show that checking only (Target, first overshoot] misses earlier branch events",
                    sample.name
                );
            eprintln!(
                    "{}: candidate#{high_index} input={} -> {high} (SFMT {}), candidate#{low_index} input={} -> Target {low} (SFMT {}); first over-target=#{first_over_target}; event-aware stop before input exceeds Target={event_aware_stop:?}; special events strictly after Target through overshoot={:?}; both delay paths: %128 hits={enter_count}, -5/%3 resolutions={resolve_count}; ending remain differences (model,left,right)={remain_differences:?}; first event-list difference={first_event_difference_pair:?}; candidates with any such event={candidates_with_state_events}/{candidate_ticks}; first landings={:?}",
                    sample.name,
                    input_frames[high_index],
                    landing_sfmt_positions[high_index],
                    input_frames[low_index],
                    landing_sfmt_positions[low_index],
                    crossing_events,
                    &landings[..landings.len().min(12)],
                );
        } else {
            eprintln!(
                    "{}: no landing drop in {candidate_ticks} adjacent A-input candidates; landing range {}..{}",
                    sample.name,
                    landings.iter().min().unwrap(),
                    landings.iter().max().unwrap()
                );
        }
    }

    assert!(
        total_entered_minus_five > 0,
        "the sweep must observe %128 == 0 transitions"
    );
    assert!(
        total_resolved_minus_five > 0,
        "the sweep must observe -5 resolution via %3"
    );
    assert_eq!(
        total_landing_drops, sample_count,
        "every profile should retain a later exact Target after its first overshoot"
    );
    eprintln!(
            "summary: entered -5 events={total_entered_minus_five}, resolved -5 events={total_resolved_minus_five}, candidate-series with landing drops={total_landing_drops}/{}",
            sample_count
        );
}

#[test]
fn courier_consumption_matches_event7_default_order() {
    let mut sfmt = Sfmt::new(0x1234_5678);
    assert_eq!(consume_courier(&mut sfmt, &CourierConfig::default()), 11);
    let locked = CourierConfig {
        no_dex: false,
        your_id: false,
        is_egg: false,
        ec: Some(1),
        pid_type: 3,
        pid: 1,
        iv_count: 0,
        fixed_ivs: [31, 31, 31, 31, 31, 31],
        ability_locked: true,
        ability: 1,
        nature_locked: true,
        nature: 1,
        gender_locked: true,
        gender_ratio: 127,
    };
    assert_eq!(consume_courier(&mut sfmt, &locked), 0);
}

#[test]
fn event7_window_bound_uses_observed_generation_consumption_not_fixed_padding() {
    let seed = 0x13ab_9f21;
    let config = TimeleapConfig {
        seed,
        start_frame: 300,
        target_frame: 2_000,
        models: 4,
        min_frame1_elapsed_ticks: 0,
        max_frame1_elapsed_ticks: 0,
        mode: TimeleapMode::Courier,
        is_boy: true,
        handoff_range_frames: 0,
        courier: CourierConfig::default(),
    };
    let delay = ConsiderDelayConfig {
        delay_frames: 42,
        generator: ConsiderDelayGenerator::Event,
        courier: CourierConfig {
            no_dex: true,
            iv_count: 3,
            ..CourierConfig::default()
        },
    };
    let cache = ObservationCache::from_seed_for_test(
        seed,
        config.target_frame + 311,
        crate::domain::timing::DEFAULT_FPS,
    );
    let cursor = Cursor {
        frame: config.start_frame,
        sfmt_position: config.start_frame,
        tick: 0,
    };
    let actual_bounds = max_event_generation_consumption(&cache, &delay, &config, cursor, None)
        .expect("measure exact Event7 generation range");
    let values = cache
        .read_values(
            cursor.sfmt_position,
            (config.target_frame - cursor.frame) as usize,
        )
        .expect("read all candidate generation start positions");
    let mut expected_bounds = EventGenerationFrameBounds::default();
    let mut expected_min = None;
    for offset in 0..values.len() {
        if let Some(consumed) = courier_consumption_from_values(&delay.courier, &values[offset..]) {
            expected_min =
                Some(expected_min.map_or(consumed, |current: usize| current.min(consumed)));
            expected_bounds.max = expected_bounds.max.max(consumed as i64);
        }
    }
    expected_bounds.min = expected_min.map_or(0, |value| value as i64);
    assert_eq!(actual_bounds, expected_bounds);

    let delay_ticks = 42 / 2 + 2;
    let ordinary_window =
        consider_delay_initial_window_frames(&delay, delay_ticks, &config, 0, false).0;
    let actual_window =
        consider_delay_initial_window_frames(&delay, delay_ticks, &config, actual_bounds, false).0;
    assert_eq!(actual_window, ordinary_window + actual_bounds.max);
    assert!(
        actual_bounds.max < 2_048,
        "the bound must come from the actual SFMT sequence"
    );
}

#[test]
fn no_dex_event7_consumption_bound_scan_cancels_promptly() {
    let seed = 0x7627_5c90;
    let target_frame = 500_000;
    let config = TimeleapConfig {
        seed,
        start_frame: 500,
        target_frame,
        models: 2,
        min_frame1_elapsed_ticks: 0,
        max_frame1_elapsed_ticks: 0,
        mode: TimeleapMode::Menu,
        is_boy: true,
        handoff_range_frames: 0,
        courier: CourierConfig::default(),
    };
    let delay = ConsiderDelayConfig {
        delay_frames: 62,
        generator: ConsiderDelayGenerator::Event,
        courier: CourierConfig {
            no_dex: true,
            iv_count: 3,
            ..CourierConfig::default()
        },
    };
    let cache = ObservationCache::from_seed_for_test(
        seed,
        target_frame + 311,
        crate::domain::timing::DEFAULT_FPS,
    );
    let cursor = Cursor {
        frame: config.start_frame,
        sfmt_position: config.start_frame,
        tick: 0,
    };
    // Exercise the scan's cancellation checks deterministically. A timed
    // canceller made this assertion depend on runner scheduling and could run
    // after the scan had already completed on faster platforms.
    let cancel = AtomicBool::new(true);
    let started = std::time::Instant::now();
    let result = max_event_generation_consumption(&cache, &delay, &config, cursor, Some(&cancel));
    let elapsed = started.elapsed();

    assert!(result.is_err_and(|error| error.contains("キャンセル")));
    assert!(
        elapsed < Duration::from_secs(1),
        "No Dex boundary scan did not stop promptly: {elapsed:?}"
    );
}

#[test]
fn no_dex_event7_delay_tick_loop_cancels_promptly() {
    use super::super::super::runtime::consider_delay_landing_frame_cancelable;

    let delay = ConsiderDelayConfig {
        // Keep the uncancelled work well beyond the 2 ms canceller delay.
        // The old 400_000F workload could finish before the other thread ran,
        // making this timing test fail despite correct cancellation behavior.
        delay_frames: 20_000_000,
        generator: ConsiderDelayGenerator::Event,
        courier: CourierConfig {
            no_dex: true,
            iv_count: 3,
            ..CourierConfig::default()
        },
    };
    let mut status = TimeleapStatus::new(2);
    let mut cursor = Cursor {
        frame: 500,
        sfmt_position: 500,
        tick: 250,
    };
    let mut sfmt = Sfmt::new(0x7627_5c90);
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_worker = cancel.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let canceller = std::thread::spawn(move || {
        started_rx.recv().expect("search start signal");
        std::thread::sleep(Duration::from_millis(2));
        cancel_worker.store(true, Ordering::Release);
    });

    let started = std::time::Instant::now();
    started_tx.send(()).expect("start cancellation timer");
    let result = consider_delay_landing_frame_cancelable(
        &delay,
        &mut status,
        &mut cursor,
        &mut sfmt,
        2,
        100_000_000,
        Some(cancel.as_ref()),
    );
    let elapsed = started.elapsed();
    canceller.join().expect("cancellation worker");

    assert!(result.is_err_and(|error| error.contains("キャンセル")));
    assert!(
        elapsed < Duration::from_secs(1),
        "No Dex Event7 delay did not stop promptly: {elapsed:?}"
    );
}

#[test]
fn no_dex_frame1_search_cancels_through_the_full_candidate_path() {
    let seed = 0xDE16_5D10;
    let config = TimeleapConfig {
        seed,
        start_frame: 500,
        target_frame: 90_465,
        models: 5,
        min_frame1_elapsed_ticks: 900,
        max_frame1_elapsed_ticks: 1_800,
        mode: TimeleapMode::Menu,
        is_boy: false,
        handoff_range_frames: 0,
        courier: CourierConfig::default(),
    };
    let delay = ConsiderDelayConfig {
        delay_frames: 62,
        generator: ConsiderDelayGenerator::Event,
        courier: CourierConfig {
            no_dex: true,
            your_id: false,
            is_egg: false,
            ec: Some(0),
            pid_type: 2,
            pid: 0,
            iv_count: 3,
            fixed_ivs: [-1; 6],
            ability_locked: true,
            ability: 1,
            nature_locked: false,
            nature: 0,
            gender_locked: true,
            gender_ratio: 0,
        },
    };
    let cache = ObservationCache::from_seed_for_test(
        seed,
        config.target_frame + 311,
        crate::domain::timing::DEFAULT_FPS,
    );
    let timeline_start = TimeleapTimelineStart::initial(config.start_frame, config.models);
    // Cancellation is pre-requested here so this test is deterministic across
    // runner speeds. Mid-operation cancellation is covered by the dedicated
    // cancellation-path tests elsewhere in the suite.
    let cancel = AtomicBool::new(true);
    let started = std::time::Instant::now();
    let result = search_candidates_from_timeline_with_stats(
        &config,
        &timeline_start,
        0,
        &cache,
        Some(&cancel),
        true,
        Some(&delay),
        true,
    );
    let elapsed = started.elapsed();

    assert!(result.is_err_and(|error| error.contains("キャンセル")));
    assert!(
        elapsed < Duration::from_secs(1),
        "Full No Dex Frame1 search did not cancel promptly: {elapsed:?}"
    );
}

#[test]
fn event7_bounded_window_matches_exhaustive_tick_start_scan_with_and_without_no_dex() {
    for no_dex in [false, true] {
        let seed = 0x2345_6789;
        let delay = ConsiderDelayConfig {
            delay_frames: 0,
            generator: ConsiderDelayGenerator::Event,
            courier: CourierConfig {
                no_dex,
                iv_count: 3,
                ..CourierConfig::default()
            },
        };
        let mut config = TimeleapConfig {
            seed,
            start_frame: 300,
            target_frame: 5_000,
            models: 1,
            min_frame1_elapsed_ticks: 0,
            max_frame1_elapsed_ticks: 0,
            mode: TimeleapMode::Courier,
            is_boy: true,
            handoff_range_frames: 0,
            courier: CourierConfig::default(),
        };
        let cache = ObservationCache::from_seed_for_test(
            seed,
            config.target_frame + 311,
            crate::domain::timing::DEFAULT_FPS,
        );
        let mut initial_status = TimeleapStatus::new(config.models);
        // Event7 Delay does not advance menu fidget, but rainy ticks still consume
        // one additional output and must remain inside the narrowed candidate bound.
        initial_status.raining = true;
        let initial_cursor = Cursor {
            frame: config.start_frame,
            sfmt_position: config.start_frame + 23,
            tick: 0,
        };
        let initial_sfmt = cached_sfmt_at(&cache, initial_cursor.sfmt_position)
            .expect("restore initial SFMT state");

        let mut sample_status = initial_status.clone();
        let mut sample_cursor = initial_cursor;
        let mut sample_sfmt = initial_sfmt.clone();
        for _ in 0..8 {
            advance_one_tick(&mut sample_status, &mut sample_cursor, &mut sample_sfmt);
        }
        let sample_input_frame = sample_cursor.frame;
        let landing = consider_delay_landing_frame(
            &delay,
            &mut sample_status,
            &mut sample_cursor,
            &mut sample_sfmt,
            config.models,
            config.target_frame,
        )
        .expect("simulate sample Event7 delay")
        .expect("sample delay must fit before the temporary upper limit");
        config.target_frame = landing;

        let mut expected = Vec::new();
        let mut scan_status = initial_status.clone();
        let mut scan_cursor = initial_cursor;
        let mut scan_sfmt = initial_sfmt.clone();
        while scan_cursor.frame <= config.target_frame {
            let tick_start_cursor = scan_cursor;
            let tick_start_status = scan_status.clone();
            let mut next_status = scan_status.clone();
            let mut next_cursor = scan_cursor;
            let mut next_sfmt = scan_sfmt.clone();
            advance_one_tick(&mut next_status, &mut next_cursor, &mut next_sfmt);

            let input_cursor = tick_start_cursor;
            if (tick_start_cursor.frame..next_cursor.frame.min(config.target_frame + 1))
                .contains(&input_cursor.frame)
            {
                let mut landing_status = tick_start_status.clone();
                let mut landing_cursor = input_cursor;
                let mut landing_sfmt = cached_sfmt_at(&cache, input_cursor.sfmt_position)
                    .expect("restore exhaustive Event7 input state");
                if apply_consider_delay(
                    &delay,
                    &mut landing_status,
                    &mut landing_cursor,
                    &mut landing_sfmt,
                    config.models,
                    config.target_frame,
                )
                .expect("exhaustively apply Event7 delay")
                {
                    expected.push((
                        input_cursor.frame,
                        input_cursor.sfmt_position,
                        landing_cursor.frame,
                        landing_cursor.sfmt_position,
                    ));
                }
            }
            if scan_cursor.frame >= config.target_frame {
                break;
            }
            scan_status = next_status;
            scan_cursor = next_cursor;
            scan_sfmt = next_sfmt;
        }

        let mut suffixes = HashMap::new();
        let mut window_hits = HashMap::new();
        let actual = search_consider_delay_inputs_with_window_cache(
            &delay,
            2,
            &config,
            &initial_status,
            &initial_cursor,
            &initial_sfmt,
            &cache,
            &mut suffixes,
            &mut window_hits,
            None,
            &mut TimeleapSearchStats::default(),
        )
        .expect("search exact SFMT-derived candidate window")
        .into_iter()
        .map(|hit| {
            (
                hit.input_cursor.frame,
                hit.input_cursor.sfmt_position,
                hit.landing_cursor.frame,
                hit.landing_cursor.sfmt_position,
            )
        })
        .collect::<Vec<_>>();
        assert!(
            expected.iter().any(|hit| hit.0 == sample_input_frame),
            "the constructed sample tick-start must be a positive Event7 fixture"
        );
        assert_eq!(actual, expected);
    }
}

#[test]
fn event_consider_delay_window_uses_fidget_free_tick_bound() {
    let config = TimeleapConfig {
        seed: 1,
        start_frame: 500,
        target_frame: 90_465,
        models: 5,
        min_frame1_elapsed_ticks: 900,
        max_frame1_elapsed_ticks: 1_800,
        mode: TimeleapMode::Menu,
        is_boy: false,
        handoff_range_frames: 0,
        courier: CourierConfig::default(),
    };
    let delay = ConsiderDelayConfig {
        delay_frames: 62,
        generator: ConsiderDelayGenerator::Event,
        courier: CourierConfig {
            no_dex: true,
            ..CourierConfig::default()
        },
    };

    assert_eq!(
        consider_delay_initial_window_frames(&delay, 33, &config, 18, true),
        (216, 0, true), // maximum includes 33 ticks * (5 models + 1 rain) + 18 outputs
    );
    assert_eq!(
        consider_delay_initial_window_frames(&delay, 33, &config, 18, false),
        (183, 0, true), // maximum includes 33 ticks * 5 models + 18 outputs
    );
}

#[test]
fn sm_no_dex_tapu_lele_normal_timeline_returns_reference_target_frames() {
    let seed = 0xDE16_5D10;
    let target_frame = 90_465;
    let cache = ObservationCache::from_seed_for_test(
        seed,
        target_frame + 311,
        crate::domain::timing::DEFAULT_FPS,
    );
    // Akala shiny Tapu Lele WC7: random EC/PID, three guaranteed IVs,
    // random nature, locked ability and genderless.
    let delay = ConsiderDelayConfig {
        delay_frames: 62,
        generator: ConsiderDelayGenerator::Event,
        courier: CourierConfig {
            no_dex: true,
            your_id: false,
            is_egg: false,
            ec: Some(0),
            pid_type: 2,
            pid: 0,
            iv_count: 3,
            fixed_ivs: [-1; 6],
            ability_locked: true,
            ability: 1,
            nature_locked: false,
            nature: 0,
            gender_locked: true,
            gender_ratio: 0,
        },
    };
    let config = TimeleapConfig {
        seed,
        start_frame: 500,
        target_frame,
        models: 5,
        min_frame1_elapsed_ticks: 0,
        max_frame1_elapsed_ticks: 0,
        mode: TimeleapMode::Menu,
        is_boy: false,
        handoff_range_frames: 0,
        courier: CourierConfig::default(),
    };
    let initial_status = TimeleapStatus::new(config.models);
    let initial_cursor = Cursor {
        frame: config.start_frame,
        sfmt_position: config.start_frame,
        tick: 0,
    };
    let initial_sfmt =
        cached_sfmt_at(&cache, initial_cursor.sfmt_position).expect("restore initial SFMT state");

    // Search7_Normal considers each generated display Frame. During a tick it
    // advances the model status, then tests each consumed Frame using that
    // tick's status snapshot before moving to the next tick.
    let mut reference_status = initial_status.clone();
    let mut reference_cursor = initial_cursor;
    let mut reference_sfmt = initial_sfmt.clone();
    let mut reference_frames = Vec::new();
    while reference_cursor.frame < target_frame {
        let candidate_start = reference_cursor.frame;
        let candidate_tick = reference_cursor.tick;
        let candidate_status = reference_status.clone();
        let mut next_status = reference_status.clone();
        let mut next_cursor = reference_cursor;
        let mut next_sfmt = reference_sfmt.clone();
        advance_one_tick(&mut next_status, &mut next_cursor, &mut next_sfmt);
        for input_frame in candidate_start..next_cursor.frame.min(target_frame + 1) {
            let mut status = candidate_status.clone();
            let mut cursor = Cursor {
                frame: input_frame,
                sfmt_position: input_frame,
                tick: candidate_tick,
            };
            let mut sfmt =
                cached_sfmt_at(&cache, input_frame).expect("restore candidate SFMT state");
            if apply_consider_delay(
                &delay,
                &mut status,
                &mut cursor,
                &mut sfmt,
                config.models,
                target_frame,
            )
            .expect("apply reference Event7 delay")
            {
                reference_frames.push(input_frame);
            }
        }
        reference_status = next_status;
        reference_cursor = next_cursor;
        reference_sfmt = next_sfmt;
    }
    reference_frames.sort_unstable();
    reference_frames.dedup();

    // These values are for ordinary Search7_Normal, which evaluates each
    // displayed Frame. They are deliberately not the post-Timeleap lattice.
    assert_eq!(reference_frames, [90_337, 90_338, 90_365]);
}

#[test]
fn sm_no_dex_post_timeleap_uses_tick_start_grid_for_reference_origin() {
    for origin_frame in [88_967, 89_267] {
        let seed = 0xDE16_5D10;
        let start_frame = 500;
        let target_frame = 90_465;
        let cache = ObservationCache::from_seed_for_test(
            seed,
            target_frame + 311,
            crate::domain::timing::DEFAULT_FPS,
        );
        let delay = ConsiderDelayConfig {
            delay_frames: 62,
            generator: ConsiderDelayGenerator::Event,
            courier: CourierConfig {
                no_dex: true,
                your_id: false,
                is_egg: false,
                ec: Some(0),
                pid_type: 2,
                pid: 0,
                iv_count: 3,
                fixed_ivs: [-1; 6],
                ability_locked: true,
                ability: 1,
                nature_locked: false,
                nature: 0,
                gender_locked: true,
                gender_ratio: 0,
            },
        };
        let config = TimeleapConfig {
            seed,
            start_frame,
            target_frame,
            models: 5,
            min_frame1_elapsed_ticks: 900,
            max_frame1_elapsed_ticks: 1_800,
            mode: TimeleapMode::Menu,
            is_boy: false,
            handoff_range_frames: 0,
            courier: CourierConfig::default(),
        };

        // Recreate the C# reference origin: walk the ordinary model state to the
        // Frame1 origin, then configure menu fidget without changing that origin.
        let mut origin_status = TimeleapStatus::new(config.models);
        let mut origin_cursor = Cursor {
            frame: start_frame,
            sfmt_position: start_frame,
            tick: 0,
        };
        let mut origin_sfmt = cached_sfmt_at(&cache, origin_cursor.sfmt_position)
            .expect("restore ordinary Timeline start");
        while origin_cursor.frame < origin_frame {
            advance_one_tick_without_blink_list(
                &mut origin_status,
                &mut origin_cursor,
                &mut origin_sfmt,
            );
        }
        assert_eq!(origin_cursor.frame, origin_frame);
        origin_status.fidget_cd = 3;

        // The reference marks this origin as valid because its post-Timeleap
        // Timeline reaches TargetFrame exactly.
        let mut reachable_status = origin_status.clone();
        let mut reachable_cursor = origin_cursor;
        let mut reachable_sfmt = cached_sfmt_at(&cache, origin_cursor.sfmt_position)
            .expect("restore post-Timeleap Timeline origin");
        while reachable_cursor.frame < target_frame {
            advance_one_tick(
                &mut reachable_status,
                &mut reachable_cursor,
                &mut reachable_sfmt,
            );
        }
        assert_eq!(reachable_cursor.frame, target_frame);

        let mut suffixes = HashMap::new();
        let mut input_windows = HashMap::new();
        let hits = search_consider_delay_inputs_with_window_cache(
            &delay,
            62 / 2 + 2,
            &config,
            &origin_status,
            &origin_cursor,
            &cached_sfmt_at(&cache, origin_cursor.sfmt_position)
                .expect("restore Event7 candidate SFMT origin"),
            &cache,
            &mut suffixes,
            &mut input_windows,
            None,
            &mut TimeleapSearchStats::default(),
        )
        .expect("search Timeleap post-event tick-start candidates");

        // 3DSRNGTool v1.0.6 Search7_TimelineLeap1 returns no candidate here.
        // Search7_Normal-like inner-Frame enumeration incorrectly added 90363.
        assert!(
            hits.is_empty(),
            "tick-start reference has no hit at origin {origin_frame}; got {:?}",
            hits.iter()
                .map(|hit| hit.input_cursor.frame)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn sm_no_dex_boundary_hits_match_independent_tick_start_simulation() {
    let seed = 0xDE16_5D10;
    let target_frame = 90_465;
    let cache = ObservationCache::from_seed_for_test(
        seed,
        target_frame + 311,
        crate::domain::timing::DEFAULT_FPS,
    );
    let delay = ConsiderDelayConfig {
        delay_frames: 62,
        generator: ConsiderDelayGenerator::Event,
        courier: CourierConfig {
            no_dex: true,
            your_id: false,
            is_egg: false,
            ec: Some(0),
            pid_type: 2,
            pid: 0,
            iv_count: 3,
            fixed_ivs: [-1; 6],
            ability_locked: true,
            ability: 1,
            nature_locked: false,
            nature: 0,
            gender_locked: true,
            gender_ratio: 0,
        },
    };
    let config = TimeleapConfig {
        seed,
        start_frame: 500,
        target_frame,
        models: 5,
        min_frame1_elapsed_ticks: 900,
        max_frame1_elapsed_ticks: 1_800,
        mode: TimeleapMode::Menu,
        is_boy: false,
        handoff_range_frames: 0,
        courier: CourierConfig::default(),
    };
    let initial_cursor = Cursor {
        frame: config.start_frame,
        sfmt_position: config.start_frame,
        tick: 0,
    };
    let timeline_start = TimeleapTimelineStart::initial(config.start_frame, config.models);
    let (groups, _) = search_candidates_from_timeline_with_stats(
        &config,
        &timeline_start,
        0,
        &cache,
        None,
        true,
        Some(&delay),
        true,
    )
    .expect("production candidate search with No Dex");
    let (serial_groups, _) = search_candidates_from_timeline_with_stats(
        &config,
        &timeline_start,
        0,
        &cache,
        None,
        true,
        Some(&delay),
        false,
    )
    .expect("serial, per-origin candidate search with No Dex");
    assert_eq!(
        groups, serial_groups,
        "parallel merge must preserve each origin"
    );
    let mut production = HashMap::new();
    for group in groups {
        for candidate in group.candidates {
            production.entry(candidate.frame1_tick).or_insert(candidate);
        }
    }
    let mut production_ticks = production.keys().copied().collect::<Vec<_>>();
    production_ticks.sort_unstable();
    let mut max_contiguous_ticks = 0_i64;
    let mut contiguous_ticks = 0_i64;
    let mut previous_tick = None;
    for tick in &production_ticks {
        contiguous_ticks = if previous_tick.is_some_and(|previous| *tick == previous + 1) {
            contiguous_ticks + 1
        } else {
            1
        };
        max_contiguous_ticks = max_contiguous_ticks.max(contiguous_ticks);
        previous_tick = Some(*tick);
    }
    assert_eq!(production_ticks.len(), 301);
    assert_eq!(
        production
            .values()
            .map(|candidate| candidate.target_inputs.len())
            .sum::<usize>(),
        301
    );
    assert_eq!(max_contiguous_ticks, 7);

    let bounds = max_event_generation_consumption(&cache, &delay, &config, initial_cursor, None)
        .expect("derive safe Event7 window");
    let (window_frames, window_end_offset, _) =
        consider_delay_initial_window_frames(&delay, 62 / 2 + 2, &config, bounds, false);
    let window_start = target_frame
        .saturating_sub(window_frames)
        .max(config.start_frame);
    let window_end = target_frame
        .saturating_sub(window_end_offset)
        .max(window_start);

    // Independently simulate sampled origins inside the configured Frame1
    // range, plus its edges. The old 0..11095 span came from the invalid
    // per-Frame post-Timeleap candidate scan and is not a search input range.
    let mut sample_ticks = (900_i64..=1_800).step_by(64).collect::<Vec<_>>();
    sample_ticks.extend([899, 900, 901, 1_799, 1_800, 1_801]);
    sample_ticks.sort_unstable();
    sample_ticks.dedup();
    let mut ordinary_status = TimeleapStatus::new(config.models);
    let mut ordinary_cursor = initial_cursor;
    let mut ordinary_sfmt = cached_sfmt_at(&cache, initial_cursor.sfmt_position)
        .expect("restore ordinary Timeline start");
    let mut sample_index = 0;
    while sample_index < sample_ticks.len() {
        let frame1_tick = ordinary_cursor.tick;
        if frame1_tick == sample_ticks[sample_index] {
            let mut event_status = ordinary_status.clone();
            let mut event_cursor = ordinary_cursor;
            let mut event_sfmt = ordinary_sfmt.clone();
            apply_event(
                &mut event_status,
                &mut event_cursor,
                &mut event_sfmt,
                config.mode,
                &config.courier,
            );

            // Direct oracle: no origin merge, shared memo, or cached candidate
            // result. Rebuild the post-event Timeline and test each A input.
            let mut scan_status = event_status;
            let mut scan_cursor = event_cursor;
            let mut scan_sfmt = event_sfmt;
            let mut direct_hits = Vec::new();
            while (config.min_frame1_elapsed_ticks..=config.max_frame1_elapsed_ticks)
                .contains(&frame1_tick)
                && scan_cursor.frame <= window_end
            {
                let tick_start_cursor = scan_cursor;
                let tick_start_status = scan_status.clone();
                let mut next_status = scan_status.clone();
                let mut next_cursor = scan_cursor;
                let mut next_sfmt = scan_sfmt.clone();
                advance_one_tick_without_blink_list(
                    &mut next_status,
                    &mut next_cursor,
                    &mut next_sfmt,
                );

                let input_frame = tick_start_cursor.frame;
                if (window_start..=window_end).contains(&input_frame) {
                    let mut input_status = tick_start_status.clone();
                    let mut input_cursor = tick_start_cursor;
                    let mut input_sfmt = cached_sfmt_at(&cache, input_cursor.sfmt_position)
                        .expect("restore direct A-input SFMT state");
                    if apply_consider_delay(
                        &delay,
                        &mut input_status,
                        &mut input_cursor,
                        &mut input_sfmt,
                        config.models,
                        target_frame,
                    )
                    .expect("apply direct No Dex Event7 delay")
                    {
                        direct_hits.push((input_frame, input_cursor, input_status));
                    }
                }

                scan_status = next_status;
                scan_cursor = next_cursor;
                scan_sfmt = next_sfmt;
            }

            let direct_input_frames = direct_hits
                .iter()
                .map(|(frame, _, _)| *frame)
                .collect::<Vec<_>>();
            let optimized = production.get(&frame1_tick);
            assert_eq!(
                optimized.map(|candidate| {
                    candidate
                        .target_inputs
                        .iter()
                        .map(|input| input.frame)
                        .collect::<Vec<_>>()
                }),
                (!direct_input_frames.is_empty()).then_some(direct_input_frames.clone()),
                "No Dex optimized/direct hit set differs at Frame1 tick {frame1_tick}"
            );
            if let Some(candidate) = optimized {
                let (_, landing_cursor, landing_status) = &direct_hits[direct_hits.len() / 2];
                assert_eq!(candidate.post_frame, landing_cursor.frame);
                assert_eq!(candidate.post_sfmt_position, landing_cursor.sfmt_position);
                assert_eq!(candidate.post_status, *landing_status);
            }
            sample_index += 1;
        }

        if sample_index == sample_ticks.len() {
            break;
        }

        advance_one_tick_without_blink_list(
            &mut ordinary_status,
            &mut ordinary_cursor,
            &mut ordinary_sfmt,
        );
    }
    assert_eq!(
        sample_index,
        sample_ticks.len(),
        "all sample ticks were visited"
    );
}

#[test]
#[ignore = "Measure the production SM No Dex A-input search without its exhaustive test oracle"]
fn benchmark_sm_no_dex_tapu_lele_production_search() {
    let seed = 0xDE16_5D10;
    let start_frame = 500;
    let target_frame = 90_465;
    let cache_started = std::time::Instant::now();
    let cache = ObservationCache::from_seed_for_test(
        seed,
        target_frame + 311,
        crate::domain::timing::DEFAULT_FPS,
    );
    let cache_elapsed = cache_started.elapsed();
    let config = TimeleapConfig {
        seed,
        start_frame,
        target_frame,
        models: 5,
        min_frame1_elapsed_ticks: 900,
        max_frame1_elapsed_ticks: 1_800,
        mode: TimeleapMode::Menu,
        is_boy: false,
        handoff_range_frames: 0,
        courier: CourierConfig::default(),
    };
    let cursor = Cursor {
        frame: start_frame,
        sfmt_position: start_frame,
        tick: 0,
    };
    let status = TimeleapStatus::new(config.models);
    let sfmt = cached_sfmt_at(&cache, cursor.sfmt_position).expect("restore starting SFMT state");
    let timeline_start = TimeleapTimelineStart::initial(start_frame, config.models);
    let mut diagnostic_status = status.clone();
    let mut diagnostic_cursor = cursor;
    let mut diagnostic_sfmt = sfmt.clone();
    let mut immediate_event_states = std::collections::HashSet::new();
    for _ in 0..1_801 {
        let mut event_status = diagnostic_status.clone();
        let mut event_cursor = diagnostic_cursor;
        let mut event_sfmt = diagnostic_sfmt.clone();
        apply_event(
            &mut event_status,
            &mut event_cursor,
            &mut event_sfmt,
            config.mode,
            &config.courier,
        );
        immediate_event_states.insert(exact_future_state_key(event_cursor, &event_status));
        advance_one_tick_without_blink_list(
            &mut diagnostic_status,
            &mut diagnostic_cursor,
            &mut diagnostic_sfmt,
        );
    }
    eprintln!(
        "SM Tapu Lele immediate post-event future states: {} across 1801 Frame1 starts",
        immediate_event_states.len()
    );
    let delay_for = |no_dex| ConsiderDelayConfig {
        delay_frames: 62,
        generator: ConsiderDelayGenerator::Event,
        courier: CourierConfig {
            no_dex,
            your_id: false,
            is_egg: false,
            ec: Some(0),
            pid_type: 2,
            pid: 0,
            iv_count: 3,
            fixed_ivs: [-1; 6],
            ability_locked: true,
            ability: 1,
            nature_locked: false,
            nature: 0,
            gender_locked: true,
            gender_ratio: 0,
        },
    };
    let run_a_input = |no_dex| {
        let delay = delay_for(no_dex);
        let mut suffixes = HashMap::new();
        let mut input_windows = HashMap::new();
        let mut stats = TimeleapSearchStats::default();
        let bound_started = std::time::Instant::now();
        let event_generation_max =
            max_event_generation_consumption(&cache, &delay, &config, cursor, None)
                .expect("derive Event7 output bound");
        let bound_elapsed = bound_started.elapsed();
        let values = candidate_transition_values(&cache, cursor, config.target_frame)
            .expect("read production transition values")
            .expect("bounded fixture fits the cached-value window");
        let scan_started = std::time::Instant::now();
        let hits = search_consider_delay_inputs_with_event_bound_and_values(
            &delay,
            62 / 2 + 2,
            &config,
            &status,
            &cursor,
            &sfmt,
            &cache,
            &mut suffixes,
            &mut input_windows,
            event_generation_max,
            None,
            None,
            cursor.sfmt_position,
            &values,
            None,
            &mut stats,
        )
        .expect("production SM Tapu Lele A-input search");
        let scan_elapsed = scan_started.elapsed();
        let elapsed = bound_elapsed.saturating_add(scan_elapsed);
        let mut frames = hits
            .iter()
            .map(|hit| hit.input_cursor.frame)
            .collect::<Vec<_>>();
        frames.sort_unstable();
        frames.dedup();
        (
            elapsed,
            bound_elapsed,
            scan_elapsed,
            event_generation_max.max,
            frames,
            stats,
        )
    };
    let run_full_search = |no_dex| {
        let delay = delay_for(no_dex);
        let started = std::time::Instant::now();
        let (groups, stats) = search_candidates_from_timeline_with_stats(
            &config,
            &timeline_start,
            0,
            &cache,
            None,
            true,
            Some(&delay),
            true,
        )
        .expect("full production SM Tapu Lele Frame1 search");
        let elapsed = started.elapsed();
        let rows = groups
            .iter()
            .map(|group| group.candidates.len())
            .sum::<usize>();
        let frame1_ticks = groups
            .iter()
            .flat_map(|group| {
                group
                    .candidates
                    .iter()
                    .map(|candidate| candidate.frame1_tick)
            })
            .collect::<std::collections::BTreeSet<_>>();
        let frame1_tick_range = frame1_ticks
            .first()
            .zip(frame1_ticks.last())
            .map(|(first, last)| (*first, *last));
        let target_frames = groups
            .iter()
            .flat_map(|group| group.candidates.iter())
            .flat_map(|candidate| candidate.target_inputs.iter())
            .map(|input| input.frame)
            .collect::<std::collections::BTreeSet<_>>();
        (
            elapsed,
            rows,
            stats,
            frame1_ticks.len(),
            frame1_tick_range,
            target_frames,
        )
    };

    // Isolate the search-window boundary from Event7 generation itself. The
    // ordinary (No Dex off) delay is identical in both runs; only its safe
    // candidate window is widened by the observed No Dex output bound.
    let mut control_results = Vec::new();
    for event_generation_max in [0, 18] {
        let delay = delay_for(false);
        let started = std::time::Instant::now();
        let (candidates, stats) = search_consider_delay_candidates_parallel(
            &config,
            &delay,
            0,
            1_801,
            status.clone(),
            cursor,
            sfmt.clone(),
            &cache,
            event_generation_max,
            None,
        )
        .expect("ordinary Event7 search with controlled output bound");
        let elapsed = started.elapsed();
        let input_pairs = candidates
            .iter()
            .flat_map(|candidate| {
                candidate
                    .target_inputs
                    .iter()
                    .map(move |input| (candidate.frame1, input.frame))
            })
            .collect::<std::collections::BTreeSet<_>>();
        eprintln!(
            "SM Tapu Lele bound-only control bound={event_generation_max}: elapsed={elapsed:?}, candidates={}, merged_timelines={}, candidate_ticks={}, prep={}us, A_search={}us, input_pairs={}",
            candidates.len(),
            stats.merged_timeline_work_items,
            stats.candidate_ticks_simulated,
            stats.frame1_enumeration_micros,
            stats.a_input_search_micros,
            input_pairs.len(),
        );
        control_results.push((input_pairs, stats));
    }
    assert_eq!(control_results[0].0, control_results[1].0);

    let no_dex_fixed_window_delay = delay_for(true);
    let started = std::time::Instant::now();
    let (no_dex_fixed_window_candidates, no_dex_fixed_window_stats) =
        search_consider_delay_candidates_parallel(
            &config,
            &no_dex_fixed_window_delay,
            0,
            1_801,
            status.clone(),
            cursor,
            sfmt.clone(),
            &cache,
            18,
            None,
        )
        .expect("No Dex search with the same fixed 0..1800 candidate window");
    let no_dex_fixed_window_elapsed = started.elapsed();
    eprintln!(
        "SM Tapu Lele No Dex fixed-window control 0..1800: elapsed={no_dex_fixed_window_elapsed:?}, candidate_origins={}, merged_timelines={}, candidate_ticks={}, prep={}us, A_search={}us",
        no_dex_fixed_window_candidates.len(),
        no_dex_fixed_window_stats.merged_timeline_work_items,
        no_dex_fixed_window_stats.candidate_ticks_simulated,
        no_dex_fixed_window_stats.frame1_enumeration_micros,
        no_dex_fixed_window_stats.a_input_search_micros,
    );

    let no_dex_delay = delay_for(true);
    let started = std::time::Instant::now();
    let (no_dex_window_candidates, no_dex_window_stats) =
        search_consider_delay_candidates_parallel(
            &config,
            &no_dex_delay,
            900,
            1_801,
            status.clone(),
            cursor,
            sfmt.clone(),
            &cache,
            18,
            None,
        )
        .expect("No Dex search limited to the original candidate window");
    let no_dex_window_elapsed = started.elapsed();
    eprintln!(
        "SM Tapu Lele No Dex original-window-only control: elapsed={no_dex_window_elapsed:?}, candidate_origins={}, merged_timelines={}, candidate_ticks={}",
        no_dex_window_candidates.len(),
        no_dex_window_stats.merged_timeline_work_items,
        no_dex_window_stats.candidate_ticks_simulated,
    );

    // Compare both algorithms over the same full configured Frame1 tick run.
    // Record candidate counts and the longest consecutive origin run so the
    // benchmark detects a regression to fabricated per-Frame continuity.
    let mut expanded_config = config.clone();
    expanded_config.min_frame1_elapsed_ticks = 0;
    expanded_config.max_frame1_elapsed_ticks = 11_095;
    let mut expanded_search_samples: [Vec<std::time::Duration>; 2] = [Vec::new(), Vec::new()];
    for round in 0..3 {
        let order = if round % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        };
        for no_dex in order {
            let delay = delay_for(no_dex);
            let started = std::time::Instant::now();
            let (groups, stats) = search_candidates_from_timeline_with_stats(
                &expanded_config,
                &timeline_start,
                0,
                &cache,
                None,
                true,
                Some(&delay),
                true,
            )
            .expect("same full contiguous Frame1 tick run");
            let elapsed = started.elapsed();
            let rows = groups
                .iter()
                .map(|group| group.candidates.len())
                .sum::<usize>();
            let mut origin_ticks = groups
                .iter()
                .flat_map(|group| {
                    group
                        .candidates
                        .iter()
                        .map(|candidate| candidate.frame1_tick)
                })
                .collect::<Vec<_>>();
            origin_ticks.sort_unstable();
            origin_ticks.dedup();
            let mut max_contiguous_origin_ticks = 0_i64;
            let mut contiguous_origin_ticks = 0_i64;
            let mut previous_origin_tick = None;
            for tick in &origin_ticks {
                contiguous_origin_ticks =
                    if previous_origin_tick.is_some_and(|previous| *tick == previous + 1) {
                        contiguous_origin_ticks + 1
                    } else {
                        1
                    };
                max_contiguous_origin_ticks =
                    max_contiguous_origin_ticks.max(contiguous_origin_ticks);
                previous_origin_tick = Some(*tick);
            }
            eprintln!(
                "SM Tapu Lele expanded-window run {round} no_dex={no_dex}: elapsed={elapsed:?}, rows={rows}, origin_ticks={}, max_contiguous_origin_ticks={max_contiguous_origin_ticks}, merged_timelines={}, candidate_ticks={}, prep={}us, A_search={}us, candidate_restore={}us",
                origin_ticks.len(),
                stats.merged_timeline_work_items,
                stats.candidate_ticks_simulated,
                stats.frame1_enumeration_micros,
                stats.a_input_search_micros,
                stats.candidate_restore_micros,
            );
            if no_dex {
                assert_eq!(rows, 4_062);
                assert_eq!(max_contiguous_origin_ticks, 11);
            }
            expanded_search_samples[usize::from(no_dex)].push(elapsed);
        }
    }
    for samples in &mut expanded_search_samples {
        samples.sort_unstable();
    }
    let expanded_off_median = expanded_search_samples[0][1].as_secs_f64();
    let expanded_on_median = expanded_search_samples[1][1].as_secs_f64();
    let normalized_ratio = expanded_on_median / expanded_off_median;
    eprintln!(
        "SM Tapu Lele equal-span NoDex ON/OFF median ratio: {normalized_ratio:.2}x (3 alternating Release runs)"
    );
    assert!(
        normalized_ratio <= 2.0,
        "No Dex incremental cost over the same contiguous candidate span exceeded 2x: {normalized_ratio:.2}x"
    );

    // Alternate the order to reduce warm-cache/order bias. These are the same
    // game/search inputs; only CourierConfig.no_dex changes between runs.
    let mut a_input_samples: [Vec<std::time::Duration>; 2] = [Vec::new(), Vec::new()];
    for round in 0..3 {
        let order = if round % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        };
        for no_dex in order {
            let (elapsed, bound_elapsed, scan_elapsed, event_bound, frames, stats) =
                run_a_input(no_dex);
            eprintln!(
                "SM Tapu Lele A-input run {round} no_dex={no_dex}: elapsed={elapsed:?} (bound={bound_elapsed:?}, scan={scan_elapsed:?}), inputs={}, frames={frames:?}, candidate_ticks={}, event_bound_outputs={event_bound}, scan_states={}",
                frames.len(),
                stats.candidate_ticks_simulated,
                stats.consider_delay_scan_cached_states,
            );
            a_input_samples[usize::from(no_dex)].push(elapsed);
        }
    }

    let mut full_search_samples: [Vec<std::time::Duration>; 2] = [Vec::new(), Vec::new()];
    for round in 0..3 {
        let order = if round % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        };
        for no_dex in order {
            let (elapsed, rows, stats, frame1_tick_count, frame1_tick_range, target_frames) =
                run_full_search(no_dex);
            let normal_ticks_to_target = normal_target_ticks(&config, &sfmt)
                .expect("measure the reference normal Timeline distance");
            eprintln!(
                "SM Tapu Lele full Frame1 run {round} no_dex={no_dex}: elapsed={elapsed:?}, candidates={rows}, frame1_ticks={frame1_tick_count}/{frame1_tick_range:?}, target_frames={}, normal_ticks_scanned={}/{}, core_origins={}, boundary_origins={}, boundary_probes={}, scope_resolution_ticks={}, merged_timelines={}, candidate_ticks={} (prep={}, A_window={}), prep={}us (snapshot={}us, replay_wall={}us), A_search={}us, candidate_restore={}us, grouping={}us, event_bound={}us, event_outputs={}, result_hits={}, result_misses={}, result_waits={}, result_bypasses={}, suffix_hits={}, suffix_misses={}, suffix_bypasses={}, merge_hits={}, suffix_states={}, event_suffix_hits={}, event_suffix_misses={}, event_suffix_waits={}, event_suffix_bypasses={}, event_suffix_states={}, event_suffix_peak={}MiB",
                target_frames.len(),
                stats.normal_ticks_scanned,
                normal_ticks_to_target,
                stats.core_frame1_origin_ticks,
                stats.boundary_frame1_origin_ticks,
                stats.boundary_probe_ticks,
                stats.scope_resolution_ticks,
                stats.merged_timeline_work_items,
                stats.candidate_ticks_simulated,
                stats.frame1_to_a_ticks_simulated,
                stats.a_window_ticks_simulated,
                stats.frame1_enumeration_micros,
                stats.candidate_snapshot_micros,
                stats.candidate_preparation_wall_micros,
                stats.a_input_search_micros,
                stats.candidate_restore_micros,
                stats.candidate_grouping_micros,
                stats.event_generation_bound_micros,
                stats.event_generation_max_outputs,
                stats.shared_consider_delay_input_result_hits,
                stats.shared_consider_delay_input_result_misses,
                stats.shared_consider_delay_input_result_waits,
                stats.shared_consider_delay_input_result_bypasses,
                stats.shared_suffix_cache_hits,
                stats.shared_suffix_cache_misses,
                stats.shared_suffix_cache_bypasses,
                stats.merge_hits,
                stats.shared_suffix_cache_states,
                stats.shared_consider_delay_input_suffix_hits,
                stats.shared_consider_delay_input_suffix_misses,
                stats.shared_consider_delay_input_suffix_waits,
                stats.shared_consider_delay_input_suffix_bypasses,
                stats.shared_consider_delay_input_suffix_states,
                stats.shared_consider_delay_input_suffix_peak_bytes / (1024 * 1024),
            );
            assert!(
                stats.normal_ticks_scanned < normal_ticks_to_target as u64,
                "candidate scanning should stop before replaying the entire Timeline to Target"
            );
            assert_eq!(
                stats.core_frame1_origin_ticks, 901,
                "No Dex must not widen the configured core Frame1 origin range"
            );
            if no_dex {
                assert_eq!(frame1_tick_range, Some((901, 1_799)));
                assert_eq!(frame1_tick_count, 301);
                assert_eq!(rows, 301);
                assert_eq!(target_frames.len(), 2);
            }
            full_search_samples[usize::from(no_dex)].push(elapsed);
        }
    }

    for (label, samples) in [
        ("A-input", &mut a_input_samples),
        ("full Frame1", &mut full_search_samples),
    ] {
        for (no_dex, times) in samples.iter_mut().enumerate() {
            times.sort_unstable();
            eprintln!(
                "SM Tapu Lele {label} median no_dex={}: {:?} (3 runs)",
                no_dex == 1,
                times[times.len() / 2],
            );
        }
        let off = samples[0][samples[0].len() / 2].as_secs_f64();
        let on = samples[1][samples[1].len() / 2].as_secs_f64();
        eprintln!(
            "SM Tapu Lele {label} NoDex ON/OFF median ratio: {:.2}x",
            on / off
        );
    }
    eprintln!("Observation cache setup (excluded from search times): {cache_elapsed:?}");
}

#[test]
fn courier_timeleap_transition_is_independent_of_consider_delay_flags() {
    let simulate = |courier: CourierConfig| {
        let mut status = TimeleapStatus::new(4);
        let mut cursor = Cursor {
            frame: 480,
            sfmt_position: 480,
            tick: 240,
        };
        let mut sfmt = Sfmt::new(0x7627_5c90);
        apply_event(
            &mut status,
            &mut cursor,
            &mut sfmt,
            TimeleapMode::Courier,
            &courier,
        );
        (cursor, status, sfmt.next_u64())
    };

    let cases = [
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
    let expected = simulate(cases[0].clone());
    assert!(expected.0.frame > 480);
    for courier in cases.into_iter().skip(1) {
        assert_eq!(
            simulate(courier),
            expected,
            "No Dex, Your ID, and egg flags must not change the Timeleap transition"
        );
    }
}

#[test]
fn consider_delay_search_reverse_scans_fixed_symbol_and_event_profiles() {
    let seed = 0x7627_5c90;
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=4_000,
        4_000,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate cache for Consider Delay tests");
    let profiles = [
        ConsiderDelayConfig {
            delay_frames: 0,
            generator: ConsiderDelayGenerator::FixedSymbol(
                crate::domain::fixed_rng_catalog::StationaryDelayType::CommonLegendary,
            ),
            courier: CourierConfig::default(),
        },
        ConsiderDelayConfig {
            delay_frames: 0,
            generator: ConsiderDelayGenerator::Event,
            courier: CourierConfig {
                no_dex: true,
                ..CourierConfig::default()
            },
        },
        ConsiderDelayConfig {
            delay_frames: 0,
            generator: ConsiderDelayGenerator::Event,
            courier: CourierConfig {
                your_id: true,
                ..CourierConfig::default()
            },
        },
        ConsiderDelayConfig {
            delay_frames: 0,
            generator: ConsiderDelayGenerator::Event,
            courier: CourierConfig {
                your_id: true,
                is_egg: true,
                ..CourierConfig::default()
            },
        },
    ];

    for profile in profiles {
        let mut start_status = TimeleapStatus::new(1);
        let mut start_cursor = Cursor {
            frame: 500,
            sfmt_position: 500,
            tick: 0,
        };
        let mut start_sfmt = cached_sfmt_at(&cache, start_cursor.sfmt_position)
            .expect("restore search start from cache");
        apply_event(
            &mut start_status,
            &mut start_cursor,
            &mut start_sfmt,
            TimeleapMode::Menu,
            &CourierConfig::default(),
        );
        for _ in 0..8 {
            advance_one_tick(&mut start_status, &mut start_cursor, &mut start_sfmt);
        }
        let mut expected_status = start_status.clone();
        let mut expected_cursor = start_cursor;
        let mut expected_sfmt = start_sfmt.clone();
        assert!(!apply_consider_delay(
            &profile,
            &mut expected_status,
            &mut expected_cursor,
            &mut expected_sfmt,
            1,
            4_000,
        )
        .expect("simulate selected Consider Delay profile"));
        let target = expected_cursor.frame;
        assert!(target > start_cursor.frame);

        let config = TimeleapConfig {
            seed,
            start_frame: 500,
            target_frame: target,
            models: 1,
            min_frame1_elapsed_ticks: 0,
            max_frame1_elapsed_ticks: 30,
            mode: TimeleapMode::Menu,
            is_boy: true,
            handoff_range_frames: 0,
            courier: CourierConfig::default(),
        };
        let mut window_suffixes = HashMap::new();
        let mut stats = TimeleapSearchStats::default();
        let found = search_consider_delay_input(
            &profile,
            2,
            &config,
            start_cursor.tick,
            &mut start_status,
            &mut start_cursor,
            &mut start_sfmt,
            &cache,
            &mut window_suffixes,
            None,
            None,
            &mut stats,
        )
        .expect("search finite reverse window");
        let (found_cursor, _) = found.expect("the constructed A input must be found");
        assert_eq!(found_cursor.frame, target);
    }
}

#[test]
fn identical_consider_delay_window_states_reuse_the_exact_scan_result() {
    use crate::domain::fixed_rng_catalog::StationaryDelayType as DelayType;

    let seed = 0x1234_5678;
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=20_000,
        20_000,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate cache");
    let delay = ConsiderDelayConfig {
        delay_frames: 0,
        generator: ConsiderDelayGenerator::FixedSymbol(DelayType::FirstKartana),
        courier: CourierConfig::default(),
    };
    let mut start_status = TimeleapStatus::new(5);
    let mut start_cursor = Cursor {
        frame: 500,
        sfmt_position: 500,
        tick: 20,
    };
    let mut start_sfmt = cached_sfmt_at(&cache, start_cursor.sfmt_position).expect("SFMT");
    apply_event(
        &mut start_status,
        &mut start_cursor,
        &mut start_sfmt,
        TimeleapMode::Menu,
        &CourierConfig::default(),
    );
    let mut generated_status = start_status.clone();
    let mut generated_cursor = start_cursor;
    let mut generated_sfmt = start_sfmt.clone();
    for _ in 0..160 {
        advance_one_tick(
            &mut generated_status,
            &mut generated_cursor,
            &mut generated_sfmt,
        );
    }
    let _ = apply_consider_delay(
        &delay,
        &mut generated_status,
        &mut generated_cursor,
        &mut generated_sfmt,
        5,
        20_000,
    )
    .expect("construct known landing");
    assert!(generated_cursor.frame > start_cursor.frame);

    let config = TimeleapConfig {
        seed,
        start_frame: 500,
        target_frame: generated_cursor.frame,
        models: 5,
        min_frame1_elapsed_ticks: 0,
        max_frame1_elapsed_ticks: 0,
        mode: TimeleapMode::Menu,
        is_boy: true,
        handoff_range_frames: 0,
        courier: CourierConfig::default(),
    };
    let mut result_cache = HashMap::new();
    let search_from =
        |tick_offset: i64,
         reuse_results: bool,
         result_cache: &mut HashMap<ConsiderDelayScanKey, Option<CachedConsiderDelayHit>>| {
            let mut status = start_status.clone();
            let mut cursor = start_cursor;
            cursor.tick = cursor.tick.saturating_add(tick_offset);
            let mut sfmt = start_sfmt.clone();
            let mut suffixes = HashMap::new();
            let mut stats = TimeleapSearchStats::default();
            let found = search_consider_delay_input(
                &delay,
                2,
                &config,
                cursor.tick,
                &mut status,
                &mut cursor,
                &mut sfmt,
                &cache,
                &mut suffixes,
                reuse_results.then_some(result_cache),
                None,
                &mut stats,
            )
            .expect("search target");
            (found, cursor, status, stats)
        };

    let baseline_started = std::time::Instant::now();
    let (baseline, baseline_input, baseline_status, baseline_stats) =
        search_from(0, false, &mut result_cache);
    let baseline_elapsed = baseline_started.elapsed();
    let _prime = search_from(0, true, &mut result_cache);
    let reuse_started = std::time::Instant::now();
    let (reused, reused_input, reused_status, reused_stats) =
        search_from(37, true, &mut result_cache);
    let reuse_elapsed = reuse_started.elapsed();

    let (baseline_landing, baseline_landing_status) = baseline.expect("baseline hit");
    let (reused_landing, reused_landing_status) = reused.expect("cached hit");
    assert_eq!(baseline_landing.frame, reused_landing.frame);
    assert_eq!(baseline_landing.sfmt_position, reused_landing.sfmt_position);
    assert_eq!(
        reused_landing.tick,
        baseline_landing.tick.saturating_add(37)
    );
    assert_eq!(baseline_input.frame, reused_input.frame);
    assert_eq!(baseline_input.sfmt_position, reused_input.sfmt_position);
    assert_eq!(reused_input.tick, baseline_input.tick.saturating_add(37));
    assert_eq!(baseline_landing_status, reused_landing_status);
    assert_eq!(baseline_status, reused_status);
    assert_eq!(baseline_stats.consider_delay_scan_reuse_hits, 0);
    assert_eq!(reused_stats.consider_delay_scan_reuse_hits, 1);
    eprintln!(
            "identical A-window state: baseline={baseline_elapsed:?}, reused={reuse_elapsed:?}, saved_ticks={}",
            baseline_stats.candidate_ticks_simulated
                .saturating_sub(reused_stats.candidate_ticks_simulated)
        );
}

#[test]
fn repeated_event7_input_scan_preserves_results_without_state_memo() {
    let seed = 0x7627_5c90;
    let cache =
        ObservationCache::from_seed_for_test(seed, 20_000, crate::domain::timing::DEFAULT_FPS);
    let delay = ConsiderDelayConfig {
        delay_frames: 42,
        generator: ConsiderDelayGenerator::Event,
        courier: CourierConfig {
            no_dex: true,
            ..CourierConfig::default()
        },
    };
    let status = TimeleapStatus::new(1);
    let cursor = Cursor {
        frame: 500,
        sfmt_position: 500,
        tick: 20,
    };
    let sfmt = cached_sfmt_at(&cache, cursor.sfmt_position).expect("restore SFMT");
    let mut expected_status = status.clone();
    let mut expected_cursor = cursor;
    let mut expected_sfmt = sfmt.clone();
    let max_frame = 20_000;
    let _matched_max_frame = apply_consider_delay(
        &delay,
        &mut expected_status,
        &mut expected_cursor,
        &mut expected_sfmt,
        1,
        max_frame,
    )
    .expect("simulate No Dex Event7");

    let config = TimeleapConfig {
        seed,
        start_frame: cursor.frame,
        target_frame: expected_cursor.frame,
        models: 1,
        min_frame1_elapsed_ticks: 0,
        max_frame1_elapsed_ticks: 0,
        mode: TimeleapMode::Menu,
        is_boy: true,
        handoff_range_frames: 0,
        courier: CourierConfig::default(),
    };
    let mut window_suffixes = HashMap::new();
    let mut input_window_cache = HashMap::new();
    let mut baseline_stats = TimeleapSearchStats::default();
    let baseline = search_consider_delay_inputs_with_window_cache(
        &delay,
        42,
        &config,
        &status,
        &cursor,
        &sfmt,
        &cache,
        &mut window_suffixes,
        &mut input_window_cache,
        None,
        &mut baseline_stats,
    )
    .expect("scan initial Event7 A-input window");
    assert!(
        !baseline.is_empty(),
        "the constructed input must hit the target"
    );
    assert_eq!(baseline_stats.consider_delay_input_window_cache_misses, 1);
    assert_eq!(baseline_stats.consider_delay_input_window_cache_hits, 0);

    let shifted_cursor = Cursor {
        tick: cursor.tick + 37,
        ..cursor
    };
    let mut second_window_cache = HashMap::new();
    let mut reused_stats = TimeleapSearchStats::default();
    let reused = search_consider_delay_inputs_with_window_cache(
        &delay,
        42,
        &config,
        &status,
        &shifted_cursor,
        &sfmt,
        &cache,
        &mut window_suffixes,
        &mut second_window_cache,
        None,
        &mut reused_stats,
    )
    .expect("reuse identical Event7 A-input window");

    assert_eq!(reused_stats.consider_delay_input_window_cache_hits, 0);
    // Both generators use the same tick-start scan and whole-window cache.
    // Candidate-level memoization remains disabled; shared suffix reuse is the
    // cross-origin cache for this optimized route.
    assert_eq!(reused_stats.consider_delay_input_suffix_cache_hits, 0);
    assert_eq!(baseline.len(), reused.len());
    for (baseline, reused) in baseline.iter().zip(&reused) {
        assert_eq!(baseline.input_cursor.frame, reused.input_cursor.frame);
        assert_eq!(
            baseline.input_cursor.sfmt_position,
            reused.input_cursor.sfmt_position
        );
        assert_eq!(reused.input_cursor.tick, baseline.input_cursor.tick + 37);
        assert_eq!(baseline.landing_cursor.frame, reused.landing_cursor.frame);
        assert_eq!(
            baseline.landing_cursor.sfmt_position,
            reused.landing_cursor.sfmt_position
        );
        assert_eq!(
            reused.landing_cursor.tick,
            baseline.landing_cursor.tick + 37
        );
        assert_eq!(baseline.landing_status, reused.landing_status);
    }
}

#[test]
fn shared_tick_start_scan_matches_direct_fixed_symbol_oracle() {
    use crate::domain::fixed_rng_catalog::StationaryDelayType as DelayType;

    let seed = 0x5343_414e;
    let delay_types = [
        DelayType::Normal,
        DelayType::SunMoonSolgaleo,
        DelayType::SunMoonLunala,
        DelayType::SunMoonExeggutor,
        DelayType::SunMoonStarter,
        DelayType::Solgaleo,
        DelayType::Lunala,
        DelayType::Nihilego,
        DelayType::Buzzwole,
        DelayType::Pheromosa,
        DelayType::Xurkitree,
        DelayType::Celesteela,
        DelayType::Kartana,
        DelayType::Guzzlord,
        DelayType::FirstNihilego,
        DelayType::FirstBuzzwole,
        DelayType::FirstPheromosa,
        DelayType::FirstXurkitree,
        DelayType::FirstCelesteela,
        DelayType::CommonLegendary,
        DelayType::FirstKartana,
        DelayType::FirstGuzzlord,
    ];
    let cache =
        ObservationCache::from_seed_for_test(seed, 20_000, crate::domain::timing::DEFAULT_FPS);

    for delay_type in delay_types {
        let delay_frames = if matches!(
            delay_type,
            DelayType::FirstKartana | DelayType::FirstGuzzlord
        ) {
            42
        } else {
            8
        };
        let delay = ConsiderDelayConfig {
            delay_frames,
            generator: ConsiderDelayGenerator::FixedSymbol(delay_type),
            courier: CourierConfig::default(),
        };
        let mut initial_status = TimeleapStatus::new(1);
        initial_status.fidget_cd = 2;
        initial_status.raining = true;
        let initial_cursor = Cursor {
            frame: 500,
            sfmt_position: 500,
            tick: 20,
        };
        let initial_sfmt = cached_sfmt_at(&cache, initial_cursor.sfmt_position)
            .expect("restore fixed-symbol starting SFMT");

        let mut sample_status = initial_status.clone();
        let mut sample_cursor = initial_cursor;
        let mut sample_sfmt = initial_sfmt.clone();
        for _ in 0..3 {
            advance_one_tick(&mut sample_status, &mut sample_cursor, &mut sample_sfmt);
        }
        let _ = apply_consider_delay(
            &delay,
            &mut sample_status,
            &mut sample_cursor,
            &mut sample_sfmt,
            1,
            20_000,
        )
        .expect("create direct-oracle Target");
        assert!(sample_cursor.frame < 20_000, "{delay_type:?} setup target");
        let target_frame = sample_cursor.frame;
        let config = TimeleapConfig {
            seed,
            start_frame: initial_cursor.frame,
            target_frame,
            models: 1,
            min_frame1_elapsed_ticks: 0,
            max_frame1_elapsed_ticks: 0,
            mode: TimeleapMode::Menu,
            is_boy: true,
            handoff_range_frames: 0,
            courier: CourierConfig::default(),
        };

        let delay_ticks = i32::try_from(delay_frames / 2 + 2).expect("bounded delay ticks");
        let (window_frames, window_end_offset, _) = consider_delay_initial_window_frames(
            &delay,
            delay_ticks,
            &config,
            EventGenerationFrameBounds::default(),
            initial_status.raining,
        );
        let window_start = target_frame
            .saturating_sub(window_frames)
            .max(initial_cursor.frame);
        let window_end = target_frame
            .saturating_sub(window_end_offset)
            .max(window_start);
        let mut expected = Vec::new();
        let mut reference_status = initial_status.clone();
        let mut reference_cursor = initial_cursor;
        let mut reference_sfmt = initial_sfmt.clone();
        while reference_cursor.frame <= window_end {
            let input_cursor = reference_cursor;
            if input_cursor.frame >= window_start {
                let mut landing_status = reference_status.clone();
                let mut landing_cursor = input_cursor;
                let mut landing_sfmt = cached_sfmt_at(&cache, input_cursor.sfmt_position)
                    .expect("restore direct fixed-symbol candidate SFMT");
                if apply_consider_delay(
                    &delay,
                    &mut landing_status,
                    &mut landing_cursor,
                    &mut landing_sfmt,
                    config.models,
                    config.target_frame,
                )
                .expect("apply direct fixed-symbol delay")
                {
                    expected.push((input_cursor, landing_cursor, landing_status));
                }
            }
            if reference_cursor.frame == window_end {
                break;
            }
            advance_one_tick(
                &mut reference_status,
                &mut reference_cursor,
                &mut reference_sfmt,
            );
        }

        let mut suffixes = HashMap::new();
        let mut window_cache = HashMap::new();
        let actual = search_consider_delay_inputs_with_window_cache(
            &delay,
            delay_ticks,
            &config,
            &initial_status,
            &initial_cursor,
            &initial_sfmt,
            &cache,
            &mut suffixes,
            &mut window_cache,
            None,
            &mut TimeleapSearchStats::default(),
        )
        .expect("run shared optimized scan for fixed-symbol DelayType");

        assert!(
            !expected.is_empty(),
            "{delay_type:?} fixture must hit Target"
        );
        assert_eq!(actual.len(), expected.len(), "{delay_type:?} hit count");
        for (actual, (expected_input, expected_landing, expected_status)) in
            actual.iter().zip(expected)
        {
            assert_eq!(actual.input_cursor, expected_input, "{delay_type:?} input");
            assert_eq!(
                actual.landing_cursor, expected_landing,
                "{delay_type:?} landing"
            );
            assert_eq!(
                actual.landing_status, expected_status,
                "{delay_type:?} state"
            );
        }
    }
}

#[test]
#[ignore = "30-60 second Frame1-range Consider Delay benchmark"]
fn benchmark_consider_delay_scan_for_wide_frame1_window() {
    use crate::domain::fixed_rng_catalog::StationaryDelayType as DelayType;

    let seed = 0x6a31_9d27;
    let target = 100_000;
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=target + 311,
        target + 311,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate wide-search cache");
    let config = TimeleapConfig {
        seed,
        start_frame: 50_000,
        target_frame: target,
        models: 15,
        min_frame1_elapsed_ticks: 900,
        max_frame1_elapsed_ticks: 1_800,
        mode: TimeleapMode::Menu,
        is_boy: true,
        handoff_range_frames: 0,
        courier: CourierConfig::default(),
    };
    let delay = ConsiderDelayConfig {
        delay_frames: 0,
        generator: ConsiderDelayGenerator::FixedSymbol(DelayType::FirstKartana),
        courier: CourierConfig::default(),
    };
    let start = TimeleapTimelineStart::initial(config.start_frame, config.models);

    for parallelize_consider_delay in [false, true] {
        let started = std::time::Instant::now();
        let (groups, stats) = search_candidates_from_timeline_with_stats(
            &config,
            &start,
            0,
            &cache,
            None,
            true,
            Some(&delay),
            parallelize_consider_delay,
        )
        .expect("search candidate range");
        let elapsed = started.elapsed();

        assert!(stats.candidates_tested >= 900);
        eprintln!(
            "Frame1 window=30-60s, models=15, target={target}, parallelize={parallelize_consider_delay}: elapsed={elapsed:?}, candidate_ticks={}, merge_hits={}, A_scan_cache_states={}, input_result_hits={}, input_suffix_hits={}, shared_suffix_states={}, shared_suffix_peak_mib={}, output_candidates={}",
            stats.candidate_ticks_simulated,
            stats.merge_hits,
            stats.consider_delay_scan_cached_states,
            stats.shared_consider_delay_input_result_hits,
            stats.shared_consider_delay_input_suffix_hits,
            stats.shared_consider_delay_input_suffix_states,
            stats.shared_consider_delay_input_suffix_peak_bytes / (1024 * 1024),
            groups.iter().map(|group| group.candidates.len()).sum::<usize>()
        );
    }
}

#[test]
fn odd_common_legendary_delay_uses_type_six_minimum_schedule() {
    let config = ConsiderDelayConfig {
        delay_frames: 1,
        generator: ConsiderDelayGenerator::FixedSymbol(
            crate::domain::fixed_rng_catalog::StationaryDelayType::CommonLegendary,
        ),
        courier: CourierConfig::default(),
    };
    assert_eq!(consider_delay_type_id(&config), 6);
    assert_eq!(delay_elapsed_tick_bound(&config, 2), 27);
}

#[test]
fn stationary_delay_profiles_match_reference_tick_and_advance_schedules() {
    let profiles = [
        (0, 0),
        (1, 1),
        (2, 1),
        (3, 1),
        (5, 0),
        (4, 2),
        (6, 3),
        (9, 1),
        (10, 1),
        (11, 1),
        (12, 1),
        (13, 1),
        (14, 1),
        (15, 1),
        (16, 1),
        (17, 1),
        (21, 1),
        (22, 3),
        (23, 1),
        (24, 1),
        (25, 1),
        (26, 1),
        (27, 1),
    ];
    for (delay_type, expected_extra) in profiles {
        let mut status = TimeleapStatus::new(9);
        status.model.remain.fill(10_000);
        let mut cursor = Cursor {
            frame: 1_000,
            sfmt_position: 1_000,
            tick: 0,
        };
        let mut sfmt = Sfmt::new(0x1234_5678);
        assert!(
            apply_stationary_delay(
                delay_type,
                1_000,
                &mut status,
                &mut cursor,
                &mut sfmt,
                9,
                100_000,
            ),
            "DelayType {delay_type}"
        );
        assert_eq!(cursor.tick, 1_000, "DelayType {delay_type} elapsed ticks");
        // Some profiles deliberately reactivate/reset NPC cooldown state, so the
        // per-NPC blink RNG draws are part of the SFMT delta as well as explicit
        // Advance calls. The table is the minimum explicit Advance count.
        assert!(
            cursor.sfmt_position - 1_000 >= expected_extra,
            "DelayType {delay_type} explicit Advance lower bound"
        );
        assert_eq!(cursor.frame, cursor.sfmt_position);
    }
}

#[test]
fn sun_moon_stationary_delay_types_use_source_minimum_tick_bounds() {
    let cases = [
        (
            crate::domain::fixed_rng_catalog::StationaryDelayType::SunMoonSolgaleo,
            98,
            1,
        ),
        (
            crate::domain::fixed_rng_catalog::StationaryDelayType::SunMoonLunala,
            95,
            1,
        ),
        (
            crate::domain::fixed_rng_catalog::StationaryDelayType::SunMoonExeggutor,
            45,
            1,
        ),
        (
            crate::domain::fixed_rng_catalog::StationaryDelayType::SunMoonStarter,
            2,
            0,
        ),
    ];
    let timeleap_config = crate::domain::timeleap::TimeleapConfig {
        seed: 1,
        start_frame: 100,
        target_frame: 10_000,
        models: 7,
        min_frame1_elapsed_ticks: 0,
        max_frame1_elapsed_ticks: 0,
        mode: TimeleapMode::Menu,
        is_boy: true,
        handoff_range_frames: 0,
        courier: CourierConfig::default(),
    };
    for (delay_type, minimum, direct_extra) in cases {
        let delay_config = ConsiderDelayConfig {
            delay_frames: 0,
            generator: ConsiderDelayGenerator::FixedSymbol(delay_type),
            courier: CourierConfig::default(),
        };
        assert_eq!(
            consider_delay_type_id(&delay_config),
            delay_type as u8 as i32
        );
        assert_eq!(delay_elapsed_tick_bound(&delay_config, 2), minimum);
        assert_eq!(
            consider_delay_initial_window_frames(&delay_config, 2, &timeleap_config, 0, false),
            (i64::from(minimum) * 11 + direct_extra, 0, false)
        );
    }
}

#[test]
fn sun_moon_solgaleo_reorders_seven_models_before_cry() {
    for (delay_type, delay_time, tail_ticks) in [(1, 146, 78), (2, 143, 75)] {
        let mut status = TimeleapStatus::new(7);
        for (index, remain) in status.model.remain.iter_mut().enumerate() {
            *remain = 10_000 + index as i32 * 1_000;
        }
        let mut cursor = Cursor {
            frame: 10_000,
            sfmt_position: 10_000,
            tick: 0,
        };
        let mut sfmt = Sfmt::new(0x1234_5678);
        assert!(apply_stationary_delay(
            delay_type,
            delay_time,
            &mut status,
            &mut cursor,
            &mut sfmt,
            7,
            100_000,
        ));
        assert_eq!(cursor.tick, i64::from(delay_time));
        let total_elapsed = delay_time;
        let before_remap_elapsed = delay_time - tail_ticks - 20;
        assert_eq!(status.model.remain[0], 10_000 - total_elapsed);
        assert_eq!(status.model.remain[1], 11_000 - total_elapsed);
        assert_eq!(status.model.remain[2], 12_000 - total_elapsed);
        assert_eq!(status.model.remain[3], 15_000 - total_elapsed);
        assert_eq!(status.model.remain[4], 16_000 - total_elapsed);
        assert_eq!(status.model.remain[5], 15_000 - before_remap_elapsed);
        assert_eq!(status.model.remain[6], 16_000 - before_remap_elapsed);
        assert_eq!(cursor.sfmt_position - 10_000, 1, "DelayType {delay_type}");
    }
}

#[test]
fn sun_moon_exeggutor_and_starter_profiles_have_expected_transition_boundaries() {
    let mut exeggutor = TimeleapStatus::new(9);
    exeggutor.model.remain.fill(10_000);
    let mut exeggutor_cursor = Cursor {
        frame: 1_000,
        sfmt_position: 1_000,
        tick: 0,
    };
    let mut exeggutor_sfmt = Sfmt::new(0x1234_5678);
    assert!(apply_stationary_delay(
        3,
        46,
        &mut exeggutor,
        &mut exeggutor_cursor,
        &mut exeggutor_sfmt,
        9,
        100_000,
    ));
    assert_eq!(exeggutor_cursor.tick, 46);
    assert_eq!(exeggutor_cursor.sfmt_position - 1_000, 1);
    assert_eq!(exeggutor.model.remain[0], 10_000 - 46);
    assert_eq!(exeggutor.model.remain[1], 10_000 - 46);
    assert!(exeggutor.model.remain[2..]
        .iter()
        .all(|remain| *remain == 9_997));

    let mut starter = TimeleapStatus::new(7);
    starter.model.remain.fill(10_000);
    let mut starter_cursor = Cursor {
        frame: 2_000,
        sfmt_position: 2_000,
        tick: 0,
    };
    let mut starter_sfmt = Sfmt::new(0x1234_5678);
    assert!(apply_stationary_delay(
        5,
        22,
        &mut starter,
        &mut starter_cursor,
        &mut starter_sfmt,
        7,
        100_000,
    ));
    assert_eq!(starter_cursor.tick, 22);
    assert!((4..=8).contains(&(starter_cursor.sfmt_position - 2_000)));
    for index in [1, 3, 4, 5] {
        assert!(
            matches!(starter.model.remain[index], 0 | -4),
            "starter model {index}"
        );
    }
    for index in [0, 2, 6] {
        assert_eq!(starter.model.remain[index], 10_000 - 22, "model {index}");
    }
}

#[test]
fn sun_moon_stationary_profiles_stop_when_the_target_is_crossed() {
    for (delay_type, delay_time) in [(1, 146), (2, 143), (3, 46), (5, 22)] {
        let mut status = TimeleapStatus::new(7);
        status.model.remain.fill(0);
        let mut cursor = Cursor {
            frame: 10_000,
            sfmt_position: 10_000,
            tick: 0,
        };
        let mut sfmt = Sfmt::new(0x1234_5678);
        assert!(
            !apply_stationary_delay(
                delay_type,
                delay_time,
                &mut status,
                &mut cursor,
                &mut sfmt,
                7,
                10_001,
            ),
            "DelayType {delay_type} should stop when the target is crossed"
        );
        assert!(cursor.frame > 10_001, "DelayType {delay_type}");
    }
}

#[test]
fn event_consider_delay_matches_no_dex_and_your_id_generation_conditions() {
    let cases = [
        (CourierConfig::default(), 0),
        (
            CourierConfig {
                no_dex: true,
                ..CourierConfig::default()
            },
            11,
        ),
        (
            CourierConfig {
                your_id: true,
                ..CourierConfig::default()
            },
            11,
        ),
        (
            CourierConfig {
                your_id: true,
                is_egg: true,
                ..CourierConfig::default()
            },
            0,
        ),
    ];
    for (courier, expected_generation_frames) in cases {
        let mut status = TimeleapStatus::new(4);
        status.model.remain.fill(10_000);
        let mut cursor = Cursor {
            frame: 1_000,
            sfmt_position: 1_000,
            tick: 0,
        };
        let mut sfmt = Sfmt::new(0x1234_5678);
        let config = ConsiderDelayConfig {
            delay_frames: 1_996,
            generator: ConsiderDelayGenerator::Event,
            courier,
        };
        assert!(apply_consider_delay(
            &config,
            &mut status,
            &mut cursor,
            &mut sfmt,
            4,
            1_000 + expected_generation_frames,
        )
        .expect("event Delay transition"));
        assert_eq!(cursor.tick, 1_000);
        assert_eq!(cursor.sfmt_position - 1_000, expected_generation_frames);
        assert_eq!(cursor.frame, cursor.sfmt_position);
    }
}

#[test]
fn menu_fidget_cooldown_sets_reference_delay_after_three_ticks() {
    let mut status = TimeleapStatus::new(1);
    status.fidget_cd = 3;
    let mut cursor = Cursor {
        frame: 0,
        sfmt_position: 0,
        tick: 0,
    };
    let mut sfmt = Sfmt::new(0x1234);
    advance_one_tick(&mut status, &mut cursor, &mut sfmt);
    advance_one_tick(&mut status, &mut cursor, &mut sfmt);
    assert_eq!(status.fidget_cd, 1);
    advance_one_tick(&mut status, &mut cursor, &mut sfmt);
    assert!(status.fidget_cd >= 402);
}

#[test]
fn event7_delay_preserves_menu_fidget_with_or_without_no_dex() {
    fn run_event_delay(initial_fidget_cd: i32, no_dex: bool) -> (TimeleapStatus, Cursor, Sfmt) {
        let mut status = TimeleapStatus::new(1);
        status.fidget_cd = initial_fidget_cd;
        let mut cursor = Cursor {
            frame: 0,
            sfmt_position: 0,
            tick: 0,
        };
        let mut sfmt = Sfmt::new(0x1234);
        let config = ConsiderDelayConfig {
            delay_frames: 4,
            generator: ConsiderDelayGenerator::Event,
            courier: CourierConfig {
                no_dex,
                ..CourierConfig::default()
            },
        };

        assert!(
            consider_delay_landing_frame(&config, &mut status, &mut cursor, &mut sfmt, 1, 200,)
                .expect("Event7 Consider Delay calculation")
                .is_some()
        );
        (status, cursor, sfmt)
    }

    for no_dex in [false, true] {
        let (mut with_menu_fidget, mut cursor_with_fidget, mut sfmt_with_fidget) =
            run_event_delay(1, no_dex);
        let (without_menu_fidget, cursor_without_fidget, _) = run_event_delay(-1, no_dex);

        // Event7 time_elapse7 advances the model state but holds the ordinary
        // menu-close fidget countdown and consumes no fidget RNG during Delay.
        assert_eq!(with_menu_fidget.fidget_cd, 1);
        assert_eq!(cursor_with_fidget, cursor_without_fidget);
        assert_eq!(with_menu_fidget.model, without_menu_fidget.model);
        assert_eq!(with_menu_fidget.phase, without_menu_fidget.phase);

        // Once Event7 Delay has ended, the normal Timeline resumes fidget
        // processing, regardless of whether No Dex generated during Delay.
        advance_one_tick(
            &mut with_menu_fidget,
            &mut cursor_with_fidget,
            &mut sfmt_with_fidget,
        );
        assert!(with_menu_fidget.fidget_cd >= 402);
    }
}

#[test]
fn fixed_symbol_delay_preserves_menu_fidget_and_rng_until_normal_timeline_resumes() {
    fn run_fixed_delay(initial_fidget_cd: i32) -> (TimeleapStatus, Cursor, Sfmt) {
        let mut status = TimeleapStatus::new(2);
        status.fidget_cd = initial_fidget_cd;
        let mut cursor = Cursor {
            frame: 0,
            sfmt_position: 0,
            tick: 0,
        };
        let mut sfmt = Sfmt::new(0x1234);
        let config = ConsiderDelayConfig {
            delay_frames: 12,
            generator: ConsiderDelayGenerator::FixedSymbol(
                crate::domain::fixed_rng_catalog::StationaryDelayType::CommonLegendary,
            ),
            courier: CourierConfig::default(),
        };

        assert!(
            consider_delay_landing_frame(&config, &mut status, &mut cursor, &mut sfmt, 2, 200)
                .expect("fixed-symbol Consider Delay calculation")
                .is_some()
        );
        (status, cursor, sfmt)
    }

    for initial_fidget_cd in [1, 2, 3] {
        let (mut with_fidget, mut cursor_with_fidget, mut sfmt_with_fidget) =
            run_fixed_delay(initial_fidget_cd);
        let (without_fidget, cursor_without_fidget, sfmt_without_fidget) = run_fixed_delay(-1);

        // Fixed-symbol Delay follows RNGPool.time_elapse7: it advances the
        // model but neither decrements menu fidget nor consumes its RNG.
        assert_eq!(with_fidget.fidget_cd, initial_fidget_cd);
        assert_eq!(cursor_with_fidget, cursor_without_fidget);
        assert_eq!(with_fidget.model, without_fidget.model);
        assert_eq!(with_fidget.phase, without_fidget.phase);
        assert_eq!(sfmt_with_fidget.state, sfmt_without_fidget.state);
        assert_eq!(sfmt_with_fidget.index, sfmt_without_fidget.index);

        // After Delay, ordinary Timeline ticks resume the preserved fidget.
        advance_one_tick(
            &mut with_fidget,
            &mut cursor_with_fidget,
            &mut sfmt_with_fidget,
        );
        match initial_fidget_cd {
            1 => assert!(with_fidget.fidget_cd >= 402),
            2 => assert_eq!(with_fidget.fidget_cd, 1),
            3 => assert_eq!(with_fidget.fidget_cd, 2),
            _ => unreachable!(),
        }
    }
}

#[test]
fn fixed_symbol_delay_matches_pinned_csharp_input_and_landing_matrix() {
    use crate::domain::fixed_rng_catalog::StationaryDelayType as DelayType;

    // Captured by invoking 3DSRNGTool v1.0.6 ModelStatus.NextState and
    // RNGPool.StationaryDelay7 directly. Each triple is the tick before, at,
    // and after the selected A input; these are not Rust-generated oracles.
    struct Vector {
        name: &'static str,
        seed: u32,
        start: i64,
        models: usize,
        frame1_tick: i64,
        a_tick_after_event: i64,
        mode: TimeleapMode,
        is_boy: bool,
        delay_type: DelayType,
        delay_frames: i64,
        frame1: i64,
        post_event: i64,
        inputs: [i64; 3],
        landings: [i64; 3],
    }

    let vectors = [
        Vector {
            name: "first Kartana / old main false positive 1",
            seed: 0x6a31_9d27,
            start: 50_000,
            models: 15,
            frame1_tick: 1156,
            a_tick_after_event: 2932,
            mode: TimeleapMode::Menu,
            is_boy: true,
            delay_type: DelayType::FirstKartana,
            delay_frames: 0,
            frame1: 63_772,
            post_event: 63_772,
            inputs: [97_889, 97_900, 97_911],
            landings: [99_892, 99_942, 99_923],
        },
        Vector {
            name: "first Kartana / old main false positive 2",
            seed: 0x6a31_9d27,
            start: 50_000,
            models: 15,
            frame1_tick: 1599,
            a_tick_after_event: 2493,
            mode: TimeleapMode::Menu,
            is_boy: true,
            delay_type: DelayType::FirstKartana,
            delay_frames: 0,
            frame1: 69_067,
            post_event: 69_067,
            inputs: [97_923, 97_935, 97_946],
            landings: [100_014, 100_007, 100_030],
        },
        Vector {
            name: "normal / one model",
            seed: 0x1234_5678,
            start: 480,
            models: 1,
            frame1_tick: 200,
            a_tick_after_event: 20,
            mode: TimeleapMode::Menu,
            is_boy: true,
            delay_type: DelayType::Normal,
            delay_frames: 0,
            frame1: 647,
            post_event: 647,
            inputs: [655, 655, 655],
            landings: [655, 655, 655],
        },
        Vector {
            name: "common legendary / female / 17 models",
            seed: 0x9e37_79b9,
            start: 487,
            models: 17,
            frame1_tick: 300,
            a_tick_after_event: 40,
            mode: TimeleapMode::Menu,
            is_boy: false,
            delay_type: DelayType::CommonLegendary,
            delay_frames: 20,
            frame1: 4486,
            post_event: 4486,
            inputs: [5006, 5020, 5034],
            landings: [5151, 5165, 5179],
        },
        Vector {
            name: "dialogue / one model",
            seed: 0x436f_7e6c,
            start: 486,
            models: 1,
            frame1_tick: 100,
            a_tick_after_event: 50,
            mode: TimeleapMode::Dialogue,
            is_boy: true,
            delay_type: DelayType::Normal,
            delay_frames: 0,
            frame1: 586,
            post_event: 590,
            inputs: [639, 640, 641],
            landings: [641, 642, 643],
        },
        Vector {
            name: "SM Solgaleo",
            seed: 0x7627_5c90,
            start: 492,
            models: 7,
            frame1_tick: 100,
            a_tick_after_event: 20,
            mode: TimeleapMode::Menu,
            is_boy: true,
            delay_type: DelayType::SunMoonSolgaleo,
            delay_frames: 300,
            frame1: 1063,
            post_event: 1063,
            inputs: [1189, 1196, 1203],
            landings: [1939, 1944, 1949],
        },
        Vector {
            name: "UM Lunala",
            seed: 0x2468_1357,
            start: 500,
            models: 9,
            frame1_tick: 120,
            a_tick_after_event: 30,
            mode: TimeleapMode::Menu,
            is_boy: false,
            delay_type: DelayType::Lunala,
            delay_frames: 170,
            frame1: 1427,
            post_event: 1427,
            inputs: [1637, 1645, 1653],
            landings: [2117, 2125, 2132],
        },
        Vector {
            name: "first Kartana / five models",
            seed: 0xde16_5d10,
            start: 500,
            models: 5,
            frame1_tick: 100,
            a_tick_after_event: 30,
            mode: TimeleapMode::Menu,
            is_boy: true,
            delay_type: DelayType::FirstKartana,
            delay_frames: 0,
            frame1: 913,
            post_event: 913,
            inputs: [985, 988, 991],
            landings: [2531, 2534, 2539],
        },
        Vector {
            name: "SM Exeggutor / dialogue",
            seed: 0x1122_3344,
            start: 490,
            models: 5,
            frame1_tick: 160,
            a_tick_after_event: 25,
            mode: TimeleapMode::Dialogue,
            is_boy: true,
            delay_type: DelayType::SunMoonExeggutor,
            delay_frames: 160,
            frame1: 1023,
            post_event: 1032,
            inputs: [1145, 1149, 1153],
            landings: [1225, 1227, 1226],
        },
        Vector {
            name: "SM starter",
            seed: 0xabcd_ef12,
            start: 500,
            models: 6,
            frame1_tick: 130,
            a_tick_after_event: 25,
            mode: TimeleapMode::Menu,
            is_boy: true,
            delay_type: DelayType::SunMoonStarter,
            delay_frames: 100,
            frame1: 1237,
            post_event: 1237,
            inputs: [1338, 1342, 1346],
            landings: [1611, 1616, 1621],
        },
        Vector {
            name: "first Buzzwole",
            seed: 0x3141_5926,
            start: 500,
            models: 5,
            frame1_tick: 140,
            a_tick_after_event: 40,
            mode: TimeleapMode::Menu,
            is_boy: true,
            delay_type: DelayType::FirstBuzzwole,
            delay_frames: 0,
            frame1: 1026,
            post_event: 1026,
            inputs: [1220, 1225, 1230],
            landings: [1695, 1700, 1705],
        },
    ];

    for vector in vectors {
        let mut status = TimeleapStatus::new(vector.models);
        status.is_boy = vector.is_boy;
        let mut cursor = Cursor {
            frame: vector.start,
            sfmt_position: vector.start,
            tick: 0,
        };
        let mut sfmt = Sfmt::new(vector.seed);
        for _ in 0..vector.start {
            sfmt.next_u64();
        }
        for _ in 0..vector.frame1_tick {
            advance_one_tick(&mut status, &mut cursor, &mut sfmt);
        }
        assert_eq!(cursor.frame, vector.frame1, "{} Frame1", vector.name);
        apply_event(
            &mut status,
            &mut cursor,
            &mut sfmt,
            vector.mode,
            &CourierConfig::default(),
        );
        assert_eq!(
            cursor.frame, vector.post_event,
            "{} post-event",
            vector.name
        );
        for _ in 0..vector.a_tick_after_event - 1 {
            advance_one_tick(&mut status, &mut cursor, &mut sfmt);
        }
        let delay = ConsiderDelayConfig {
            delay_frames: vector.delay_frames,
            generator: ConsiderDelayGenerator::FixedSymbol(vector.delay_type),
            courier: CourierConfig::default(),
        };
        for index in 0..3 {
            if index > 0 {
                advance_one_tick(&mut status, &mut cursor, &mut sfmt);
            }
            assert_eq!(
                cursor.frame, vector.inputs[index],
                "{} A input {index}",
                vector.name
            );
            assert_eq!(
                cursor.frame, cursor.sfmt_position,
                "{} SFMT before delay",
                vector.name
            );
            let mut trial_status = status.clone();
            let mut trial_cursor = cursor;
            let mut trial_sfmt = sfmt.clone();
            let landing = consider_delay_landing_frame(
                &delay,
                &mut trial_status,
                &mut trial_cursor,
                &mut trial_sfmt,
                vector.models,
                vector.landings[index] + 10_000,
            )
            .unwrap_or_else(|error| panic!("{} A input {index}: {error}", vector.name));
            assert_eq!(
                landing,
                Some(vector.landings[index]),
                "{} landing {index}",
                vector.name
            );
            assert_eq!(
                trial_cursor.frame, vector.landings[index],
                "{} cursor {index}",
                vector.name
            );
            assert_eq!(
                trial_cursor.sfmt_position, trial_cursor.frame,
                "{} SFMT after delay {index}",
                vector.name
            );
            assert_eq!(
                trial_status.fidget_cd, status.fidget_cd,
                "{} fidget during delay {index}",
                vector.name
            );
        }
    }
}

#[test]
fn fixed_symbol_frame1_search_retains_csharp_reachable_a_inputs() {
    use crate::domain::fixed_rng_catalog::StationaryDelayType as DelayType;

    // These targets are the direct C# landings in the matrix above. In
    // addition to checking the delay itself, exercise the complete Frame1
    // search (origin enumeration, window bounds, and result restoration).
    let cases = [
        (
            "common legendary",
            0x9e37_79b9,
            487,
            17,
            300,
            false,
            DelayType::CommonLegendary,
            20,
            5165,
            5020,
        ),
        (
            "SM Solgaleo",
            0x7627_5c90,
            492,
            7,
            100,
            true,
            DelayType::SunMoonSolgaleo,
            300,
            1944,
            1196,
        ),
        (
            "UM Lunala",
            0x2468_1357,
            500,
            9,
            120,
            false,
            DelayType::Lunala,
            170,
            2125,
            1645,
        ),
        (
            "first Buzzwole",
            0x3141_5926,
            500,
            5,
            140,
            true,
            DelayType::FirstBuzzwole,
            0,
            1700,
            1225,
        ),
    ];
    for (
        name,
        seed,
        start_frame,
        models,
        frame1_tick,
        is_boy,
        delay_type,
        delay_frames,
        target_frame,
        known_a_input,
    ) in cases
    {
        let config = TimeleapConfig {
            seed,
            start_frame,
            target_frame,
            models,
            min_frame1_elapsed_ticks: frame1_tick,
            max_frame1_elapsed_ticks: frame1_tick,
            mode: TimeleapMode::Menu,
            is_boy,
            handoff_range_frames: 0,
            courier: CourierConfig::default(),
        };
        let delay = ConsiderDelayConfig {
            delay_frames,
            generator: ConsiderDelayGenerator::FixedSymbol(delay_type),
            courier: CourierConfig::default(),
        };
        let cache = crate::domain::rng::generate_observation_cache(
            seed,
            0..=target_frame + 1000,
            target_frame + 1000,
            crate::domain::timing::DEFAULT_FPS,
        )
        .unwrap_or_else(|error| panic!("{name}: cache: {error}"));
        let start = TimeleapTimelineStart::initial(start_frame, models);
        let serial = search_candidates_from_timeline_with_stats(
            &config,
            &start,
            0,
            &cache,
            None,
            true,
            Some(&delay),
            false,
        )
        .unwrap_or_else(|error| panic!("{name}: serial: {error}"))
        .0;
        let parallel = search_candidates_from_timeline_with_stats(
            &config,
            &start,
            0,
            &cache,
            None,
            true,
            Some(&delay),
            true,
        )
        .unwrap_or_else(|error| panic!("{name}: parallel: {error}"))
        .0;
        assert_eq!(parallel, serial, "{name}: all candidate rows and states");
        assert!(
            serial
                .iter()
                .flat_map(|group| &group.candidates)
                .any(|candidate| {
                    candidate.frame1_tick == frame1_tick
                        && candidate
                            .target_inputs
                            .iter()
                            .any(|input| input.frame == known_a_input)
                        && candidate.post_frame == target_frame
                }),
            "{name}: C# reference A input {known_a_input} must be retained"
        );
    }
}

#[test]
fn fixed_symbol_search_excludes_old_main_false_positive_a_inputs() {
    use crate::domain::fixed_rng_catalog::StationaryDelayType as DelayType;

    // On main these two A inputs were reported as landing at 100000. The
    // pinned C# execution lands at 99942 and 100007, respectively.
    let config = TimeleapConfig {
        seed: 0x6a31_9d27,
        start_frame: 50_000,
        target_frame: 100_000,
        models: 15,
        min_frame1_elapsed_ticks: 1156,
        max_frame1_elapsed_ticks: 1599,
        mode: TimeleapMode::Menu,
        is_boy: true,
        handoff_range_frames: 0,
        courier: CourierConfig::default(),
    };
    let delay = ConsiderDelayConfig {
        delay_frames: 0,
        generator: ConsiderDelayGenerator::FixedSymbol(DelayType::FirstKartana),
        courier: CourierConfig::default(),
    };
    let cache = crate::domain::rng::generate_observation_cache(
        config.seed,
        0..=config.target_frame + 1000,
        config.target_frame + 1000,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate first-Kartana regression cache");
    let start = TimeleapTimelineStart::initial(config.start_frame, config.models);
    let (groups, _) = search_candidates_from_timeline_with_stats(
        &config,
        &start,
        0,
        &cache,
        None,
        true,
        Some(&delay),
        true,
    )
    .expect("search first-Kartana regression window");
    for candidate in groups.iter().flat_map(|group| &group.candidates) {
        let old_false_input = match candidate.frame1_tick {
            1156 => Some(97_900),
            1599 => Some(97_935),
            _ => None,
        };
        if let Some(old_false_input) = old_false_input {
            assert!(
                !candidate
                    .target_inputs
                    .iter()
                    .any(|input| input.frame == old_false_input),
                "Frame1 tick {} incorrectly retains old main A input {old_false_input}",
                candidate.frame1_tick
            );
        }
    }
}

#[test]
fn event7_delay_flags_match_pinned_csharp_landings() {
    // Direct Event7.Delay() execution in 3DSRNGTool v1.0.6, with the same
    // seed, starting state, A input, and 62F delay for all four flag sets.
    let seed = 0xde16_5d10;
    let mut status = TimeleapStatus::new(5);
    status.is_boy = true;
    let mut cursor = Cursor {
        frame: 500,
        sfmt_position: 500,
        tick: 0,
    };
    let mut sfmt = Sfmt::new(seed);
    for _ in 0..500 {
        sfmt.next_u64();
    }
    for _ in 0..100 {
        advance_one_tick(&mut status, &mut cursor, &mut sfmt);
    }
    apply_event(
        &mut status,
        &mut cursor,
        &mut sfmt,
        TimeleapMode::Menu,
        &CourierConfig::default(),
    );
    for _ in 0..30 {
        advance_one_tick(&mut status, &mut cursor, &mut sfmt);
    }
    assert_eq!(cursor.frame, 988);
    assert_eq!(status.fidget_cd, 427);

    for (name, no_dex, your_id, is_egg, expected_landing) in [
        ("neither", false, false, false, 1101),
        ("No Dex", true, false, false, 1114),
        ("Your ID non-egg", false, true, false, 1114),
        ("Your ID egg", false, true, true, 1101),
    ] {
        let delay = ConsiderDelayConfig {
            delay_frames: 62,
            generator: ConsiderDelayGenerator::Event,
            courier: CourierConfig {
                no_dex,
                your_id,
                is_egg,
                iv_count: 3,
                ..CourierConfig::default()
            },
        };
        let mut trial_status = status.clone();
        let mut trial_cursor = cursor;
        let mut trial_sfmt = sfmt.clone();
        let landing = consider_delay_landing_frame(
            &delay,
            &mut trial_status,
            &mut trial_cursor,
            &mut trial_sfmt,
            5,
            2000,
        )
        .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(landing, Some(expected_landing), "{name}");
        assert_eq!(trial_cursor.frame, expected_landing, "{name} cursor");
        assert_eq!(trial_status.fidget_cd, 427, "{name} fidget");
    }
}

#[test]
#[ignore = "No Dex Event7 search performance measurement; run with --release --ignored --nocapture"]
fn benchmark_no_dex_event7_candidate_search() {
    let seed = 0xb248_ea3b;
    let start_frame = 490;
    let target_frame = 70_000;
    let cache_end = target_frame + 5_000;
    let cache_started = std::time::Instant::now();
    let cache = ObservationCache::from_seed_for_test(
        seed,
        cache_end + 311,
        crate::domain::timing::DEFAULT_FPS,
    );
    let cache_elapsed = cache_started.elapsed();
    let config = TimeleapConfig {
        seed,
        start_frame,
        target_frame,
        models: 15,
        min_frame1_elapsed_ticks: 900,
        max_frame1_elapsed_ticks: 1_800,
        mode: TimeleapMode::Courier,
        is_boy: true,
        handoff_range_frames: 0,
        courier: CourierConfig::default(),
    };
    let start = TimeleapTimelineStart::initial(start_frame, config.models);

    for (name, no_dex, delay_frames) in [
        ("Event7 without No Dex / 42F", false, 42),
        ("Event7 No Dex / 42F", true, 42),
        ("Event7 No Dex / 62F", true, 62),
    ] {
        let delay = ConsiderDelayConfig {
            delay_frames,
            generator: ConsiderDelayGenerator::Event,
            courier: CourierConfig {
                no_dex,
                iv_count: 3,
                ..CourierConfig::default()
            },
        };
        let started = std::time::Instant::now();
        let (groups, stats) = search_candidates_from_timeline_with_stats(
            &config,
            &start,
            0,
            &cache,
            None,
            true,
            Some(&delay),
            true,
        )
        .unwrap_or_else(|error| panic!("{name}: {error}"));
        let elapsed = started.elapsed();
        eprintln!(
            "{name}: cache={cache_elapsed:?}, search={elapsed:?}, frame1_candidates={}, merged_timelines={}, frame1_to_a_ticks={}, total_candidate_ticks={}, prep={}us, A_search={}us, Event7_bound={}us/{} outputs, A_window_cache_hits={}, A_window_cache_misses={}, A_suffix_cache_hits={}, shared_suffix_states={}, shared_suffix_peak_mib={}, scan_cache_states={}, output_candidates={}",
            stats.candidates_tested,
            stats.merged_timeline_work_items,
            stats.frame1_to_a_ticks_simulated,
            stats.candidate_ticks_simulated,
            stats.frame1_enumeration_micros,
            stats.a_input_search_micros,
            stats.event_generation_bound_micros,
            stats.event_generation_max_outputs,
            stats.consider_delay_input_window_cache_hits,
            stats.consider_delay_input_window_cache_misses,
            stats.consider_delay_input_suffix_cache_hits,
            stats.shared_consider_delay_input_suffix_states,
            stats.shared_consider_delay_input_suffix_peak_bytes / (1024 * 1024),
            stats.consider_delay_scan_cached_states,
            groups.iter().map(|group| group.candidates.len()).sum::<usize>(),
        );
    }
}
