use super::*;

#[test]
fn candidate_snapshot_scan_stops_at_target_without_a_separate_tick_pass() {
    let seed = 0x436F_7E6C;
    let initial_status = TimeleapStatus::new(2);
    let initial_cursor = Cursor {
        frame: 0,
        sfmt_position: 0,
        tick: 0,
    };
    let initial_sfmt = Sfmt::new(seed);
    let mut sequence_status = initial_status.clone();
    let mut sequence_cursor = initial_cursor;
    let mut sequence_sfmt = initial_sfmt.clone();
    let mut frames = vec![sequence_cursor.frame];
    for _ in 0..8 {
        advance_one_tick_without_blink_list(
            &mut sequence_status,
            &mut sequence_cursor,
            &mut sequence_sfmt,
        );
        frames.push(sequence_cursor.frame);
    }
    let make_source = |target_frame, end_tick| CandidateSnapshotSource {
        cursor: initial_cursor,
        status: initial_status.clone(),
        sfmt: initial_sfmt.clone(),
        start_tick: 0,
        end_tick,
        target_frame,
        target_reached: false,
        mode: TimeleapMode::Menu,
        courier: CourierConfig::default().generation_profile(),
    };

    let exact_target_tick = 4;
    let mut exact_source = make_source(frames[exact_target_tick], 8);
    let mut exact_ticks = Vec::new();
    while let Some(candidate) = exact_source.next(None).expect("exact target scan") {
        exact_ticks.push(candidate.frame1_tick);
    }
    assert_eq!(
        exact_ticks,
        (0..exact_target_tick as i64).collect::<Vec<_>>()
    );
    assert!(exact_source.target_reached);

    let (overshoot_tick, pair) = frames
        .windows(2)
        .enumerate()
        .find(|(_, pair)| pair[1] - pair[0] > 1)
        .expect("the normal Timeline has a skipped displayed Frame");
    let between_frames_target = pair[0] + 1;
    let mut overshoot_source = make_source(between_frames_target, 8);
    let mut overshoot_ticks = Vec::new();
    while let Some(candidate) = overshoot_source.next(None).expect("overshoot target scan") {
        overshoot_ticks.push(candidate.frame1_tick);
    }
    assert_eq!(
        overshoot_ticks,
        (0..overshoot_tick as i64).collect::<Vec<_>>()
    );
    assert!(overshoot_source.target_reached);

    let mut bounded_source = make_source(frames[5], 2);
    let mut bounded_ticks = Vec::new();
    while let Some(candidate) = bounded_source.next(None).expect("bounded scan") {
        bounded_ticks.push(candidate.frame1_tick);
    }
    assert_eq!(bounded_ticks, vec![0, 1]);
    assert!(!bounded_source.target_reached);
    assert_eq!(bounded_source.cursor.tick, 2);
}

#[test]
fn shared_cache_owner_unwind_wakes_waiters_and_releases_the_key() {
    let cache = Arc::new(SharedConsiderDelayScanCache::new(1));
    let key = shared_scan_test_key(100);
    let owner = match cache.lookup_or_reserve(&key) {
        SharedMemoLookup::Owner(entry) => entry,
        _ => panic!("first lookup must reserve the key"),
    };
    let (waiter_started_tx, waiter_started_rx) = std::sync::mpsc::channel();
    let waiter_cache = cache.clone();
    let waiter_key = key.clone();
    let waiter = std::thread::spawn(move || {
        let entry = match waiter_cache.lookup_or_reserve(&waiter_key) {
            SharedMemoLookup::Wait(entry) => entry,
            _ => panic!("same-key lookup must wait for its owner"),
        };
        waiter_started_tx.send(()).expect("signal waiter readiness");
        waiter_cache.wait(&entry, None)
    });

    waiter_started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("waiter reached the in-flight entry");
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut path = PendingSharedScanPath::default();
        path.claim(owner, 100);
        panic!("simulate owner unwinding during computation");
    }));
    assert!(unwind.is_err());
    assert!(matches!(
        waiter.join().expect("waiter thread").expect("wait result"),
        SharedMemoWait::Retry
    ));
    assert!(matches!(
        cache.lookup_or_reserve(&key),
        SharedMemoLookup::Owner(_)
    ));
}

#[test]
fn manual_frame1_preparation_does_not_require_a_target_candidate() {
    use std::sync::atomic::AtomicBool;

    let seed = 0x436F_7E6C;
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=1_000,
        1_000,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate cache for manual Frame1 preparation");
    let mut config = TimeleapConfig {
        seed,
        start_frame: 0,
        target_frame: 0,
        models: 17,
        min_frame1_elapsed_ticks: 0,
        max_frame1_elapsed_ticks: 0,
        mode: TimeleapMode::Menu,
        is_boy: false,
        handoff_range_frames: 20,
        courier: CourierConfig::default(),
    };
    let mut sfmt = cached_sfmt_at(&cache, 0).expect("start SFMT");
    let mut status = TimeleapStatus::new(config.models);
    let mut cursor = Cursor {
        frame: 0,
        sfmt_position: 0,
        tick: 0,
    };
    advance_one_tick(&mut status, &mut cursor, &mut sfmt);
    let requested_frame1 = cursor.frame;
    assert!(requested_frame1 > config.target_frame);

    config.target_frame = requested_frame1;
    let start = TimeleapTimelineStart::initial(config.start_frame, config.models);
    let candidate = run_timeleap_frame1_preparation(
        &config,
        requested_frame1,
        &start,
        &cache,
        &AtomicBool::new(false),
    )
    .expect("manual Frame1 is prepared independently of candidate search");

    assert_eq!(candidate.nominal_frame1, requested_frame1);
    assert_eq!(candidate.nominal_tick, 1);
    let nominal = candidate
        .candidates
        .iter()
        .find(|candidate| candidate.frame1 == requested_frame1)
        .expect("the nominal frame is included in the window");
    assert_eq!(nominal.frame1_tick, 1);
    assert_eq!(nominal.pre_frame, requested_frame1);
    assert_eq!(nominal.post_frame, requested_frame1);
    assert_eq!(nominal.post_status.fidget_cd, 3);
    assert!(candidate
        .candidates
        .iter()
        .any(|candidate| candidate.frame1 > config.target_frame));
}

#[test]
fn frame1_candidates_continue_from_the_confirmed_normal_timeline_state() {
    use std::sync::atomic::AtomicBool;

    let seed = 0x436F_7E6C;
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=1_000,
        1_000,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate cache");
    let config = TimeleapConfig {
        seed,
        start_frame: 100,
        target_frame: 100,
        models: 3,
        min_frame1_elapsed_ticks: 0,
        max_frame1_elapsed_ticks: 0,
        mode: TimeleapMode::Menu,
        is_boy: true,
        handoff_range_frames: 0,
        courier: CourierConfig::default(),
    };
    let mut start = TimeleapTimelineStart::initial(100, config.models);
    start.sfmt_position = 100;
    start.tick = 50;
    start.model.remain = vec![-2, 1, 35];

    let preparation =
        run_timeleap_frame1_preparation(&config, 100, &start, &cache, &AtomicBool::new(false))
            .expect("prepare from the confirmed state");

    assert_eq!(preparation.nominal_tick, 50);
    assert_eq!(preparation.candidates.len(), 1);
    assert_eq!(preparation.candidates[0].pre_status.model, start.model);
    assert_eq!(preparation.candidates[0].pre_sfmt_position, 100);
}

#[test]
fn candidate_search_and_set_preparation_share_the_confirmed_pre_event_timeline() {
    use std::sync::atomic::AtomicBool;

    let seed = 0x436F_7E6C;
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=10_000,
        10_000,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("cache");
    let mut start = TimeleapTimelineStart::initial(500, 3);
    start.tick = 40;
    start.model.remain = vec![-5, 2, 17];

    let mut sfmt = cached_sfmt_at(&cache, start.sfmt_position).expect("anchor SFMT");
    let mut status = TimeleapStatus::new(3);
    status.model = start.model.clone();
    status.is_boy = true;
    let mut cursor = Cursor {
        frame: start.frame,
        sfmt_position: start.sfmt_position,
        tick: start.tick,
    };
    for _ in 0..15 {
        advance_one_tick(&mut status, &mut cursor, &mut sfmt);
    }
    let current_frame = cursor.frame;
    cursor.tick = 2;
    advance_one_tick(&mut status, &mut cursor, &mut sfmt);
    advance_one_tick(&mut status, &mut cursor, &mut sfmt);
    apply_event(
        &mut status,
        &mut cursor,
        &mut sfmt,
        TimeleapMode::Menu,
        &CourierConfig::default(),
    );
    for _ in 0..12 {
        advance_one_tick(&mut status, &mut cursor, &mut sfmt);
    }
    let target_frame = cursor.frame;

    let config = TimeleapConfig {
        seed,
        start_frame: current_frame,
        target_frame,
        models: 3,
        min_frame1_elapsed_ticks: 2,
        max_frame1_elapsed_ticks: 2,
        mode: TimeleapMode::Menu,
        is_boy: true,
        handoff_range_frames: 0,
        courier: CourierConfig::default(),
    };
    let groups = search_candidates_from_timeline_cancelable(
        &config,
        &start,
        55,
        &cache,
        Some(&AtomicBool::new(false)),
    )
    .expect("search from the same confirmed Timeline used by Set");
    let candidate_frame1 = groups
        .iter()
        .flat_map(|group| group.candidates.iter())
        .find(|candidate| candidate.frame1_tick == 2)
        .expect("the deliberately constructed candidate reaches the target")
        .frame1;

    let mut preparation_config = config;
    preparation_config.target_frame = candidate_frame1;
    let preparation = run_timeleap_frame1_preparation(
        &preparation_config,
        candidate_frame1,
        &start,
        &cache,
        &AtomicBool::new(false),
    )
    .expect("the table candidate must be reachable on its pre-event Timeline");
    assert_eq!(preparation.nominal_frame1, candidate_frame1);
    assert_eq!(preparation.nominal_tick, 57);
}

#[test]
fn frame1_window_uses_tick_radius_even_when_npc_frame_consumption_is_large() {
    let seed = 0x436F_7E6C;
    let cache = crate::domain::rng::generate_observation_cache(seed, 0..=6_000, 6_000, 60.0)
        .expect("cache");
    let mut config = TimeleapConfig {
        seed,
        start_frame: 0,
        target_frame: 0,
        models: 50,
        min_frame1_elapsed_ticks: 0,
        max_frame1_elapsed_ticks: 0,
        mode: TimeleapMode::Menu,
        is_boy: false,
        handoff_range_frames: 20,
        courier: CourierConfig::default(),
    };
    let mut sfmt = cached_sfmt_at(&cache, 0).expect("start SFMT");
    let mut status = TimeleapStatus::new(config.models);
    let mut cursor = Cursor {
        frame: 0,
        sfmt_position: 0,
        tick: 0,
    };
    for _ in 0..30 {
        advance_one_tick(&mut status, &mut cursor, &mut sfmt);
    }
    let nominal = cursor.frame;
    config.target_frame = nominal;
    let start = TimeleapTimelineStart::initial(config.start_frame, config.models);

    let prepared =
        run_timeleap_frame1_preparation(&config, nominal, &start, &cache, &AtomicBool::new(false))
            .expect("build the full reachable Frame1 window");

    assert_eq!(prepared.candidates.len(), 21);
    assert!(prepared
        .candidates
        .iter()
        .any(|candidate| candidate.frame1 == nominal));
    assert!(prepared
        .candidates
        .iter()
        .all(
            |candidate| (prepared.nominal_tick - 10..=prepared.nominal_tick + 10)
                .contains(&candidate.frame1_tick)
        ));
    assert!(prepared
        .candidates
        .iter()
        .any(|candidate| (candidate.frame1 - nominal).abs() > 20));
    assert!(prepared
        .candidates
        .iter()
        .any(|candidate| candidate.frame1 > config.target_frame));
    assert!(prepared
        .candidates
        .windows(2)
        .all(|pair| pair[0].frame1 < pair[1].frame1));
}

#[test]
fn representative_uses_later_middle_for_even_width() {
    assert_eq!(representative_frame(10, 17), 14);
    assert_eq!(representative_frame(10, 16), 13);
}

#[test]
fn candidate_rows_keep_each_frame1_and_share_contiguous_grace() {
    let status = TimeleapStatus::new(1);
    let make = |frame1, frame1_tick| TimeleapCandidate {
        frame1,
        frame1_tick,
        post_tick: frame1_tick + 100,
        grace_frames: 0,
        pre_frame: 0,
        pre_sfmt_position: 0,
        pre_status: status.clone(),
        post_frame: 0,
        post_sfmt_position: 0,
        post_status: status.clone(),
        target_inputs: Vec::new().into(),
    };
    let groups = group_candidates(vec![make(10, 0), make(12, 1), make(20, 3)]);
    assert_eq!(groups.len(), 3);
    assert_eq!(groups[0].grace_frames, 4);
    assert_eq!(groups[1].grace_frames, 4);
    assert_eq!(groups[0].representative_frame1, 10);
    assert_eq!(groups[1].representative_frame1, 12);
    assert_eq!(groups[0].frame2_ticks, 100);
    assert_eq!(groups[2].grace_frames, 2);
}

#[test]
fn seconds_window_includes_the_entire_intersecting_frame1_run() {
    let status = TimeleapStatus::new(1);
    let make = |tick| TimeleapCandidate {
        frame1: 1_000 + tick * 2,
        frame1_tick: tick,
        post_tick: tick + 20,
        grace_frames: 0,
        pre_frame: 1_000 + tick * 2,
        pre_sfmt_position: 0,
        pre_status: status.clone(),
        post_frame: 2_000,
        post_sfmt_position: 20,
        post_status: status.clone(),
        target_inputs: vec![TimeleapTargetInput {
            frame: 2_000,
            tick: tick + 20,
        }]
        .into(),
    };

    // The selected seconds window [2, 3) touches the run [0, 3),
    // while the disconnected run [5, 6) must remain excluded.
    let groups =
        groups_intersecting_tick_window(vec![make(0), make(1), make(2), make(5)], 2, 3).unwrap();

    assert_eq!(groups.len(), 3);
    assert!(groups.iter().all(|row| {
        (row.frame1_tick_start, row.frame1_tick_end, row.grace_frames) == (0, 2, 6)
    }));
    assert_eq!(
        groups
            .iter()
            .map(|row| row.candidates[0].frame1_tick)
            .collect::<Vec<_>>(),
        vec![0, 1, 2],
        "rows outside the requested seconds window remain visible when connected"
    );
}

#[test]
fn grace_counts_each_contiguous_candidate_tick_as_two_frames() {
    let status = TimeleapStatus::new(1);
    let candidates = (0..4)
        .map(|tick| TimeleapCandidate {
            frame1: 10_000 + tick * 10,
            frame1_tick: tick,
            post_tick: tick + 100,
            grace_frames: 0,
            pre_frame: 0,
            pre_sfmt_position: 0,
            pre_status: status.clone(),
            post_frame: 0,
            post_sfmt_position: 0,
            post_status: status.clone(),
            target_inputs: Vec::new().into(),
        })
        .collect();

    let groups = group_candidates(candidates);

    assert_eq!(groups.len(), 4);
    assert!(groups.iter().all(|row| row.grace_frames == 8));
    assert_eq!(
        groups
            .iter()
            .map(|row| row.representative_frame1)
            .collect::<Vec<_>>(),
        vec![10_000, 10_010, 10_020, 10_030]
    );
}

#[test]
fn candidate_rows_keep_frame1_and_a_input_grace_independent() {
    let status = TimeleapStatus::new(1);
    let candidate = TimeleapCandidate {
        frame1: 500,
        frame1_tick: 10,
        post_tick: 50,
        grace_frames: 0,
        pre_frame: 500,
        pre_sfmt_position: 500,
        pre_status: status.clone(),
        post_frame: 1_000,
        post_sfmt_position: 1_000,
        post_status: status,
        target_inputs: vec![
            TimeleapTargetInput {
                frame: 900,
                tick: 40,
            },
            TimeleapTargetInput {
                frame: 904,
                tick: 41,
            },
            TimeleapTargetInput {
                frame: 910,
                tick: 42,
            },
            TimeleapTargetInput {
                frame: 920,
                tick: 45,
            },
        ]
        .into(),
    };

    let rows = group_candidates(vec![candidate]);

    assert_eq!(rows.len(), 2, "disjoint A-input windows stay separate rows");
    assert_eq!(rows[0].grace_frames, 2, "Frame1 grace is independent");
    assert_eq!(rows[0].target_grace_frames, 6);
    assert_eq!(rows[0].representative_target_frame, 904);
    assert_eq!(rows[0].representative_frame1, 500);
    assert_eq!(rows[0].frame2_ticks, 31);
    assert_eq!(
        (rows[0].target_tick_start, rows[0].target_tick_end),
        (40, 42)
    );
    assert_eq!(rows[1].target_grace_frames, 2);
    assert_eq!(rows[1].representative_target_frame, 920);
    assert_eq!(rows[1].representative_frame1, 500);
    assert_eq!(rows[1].frame2_ticks, 35);
    assert!(rows.iter().all(|row| row.candidates.len() == 1));
}

#[test]
fn candidate_grouping_falls_back_for_unsorted_duplicate_target_inputs() {
    let status = TimeleapStatus::new(1);
    let candidate = TimeleapCandidate {
        frame1: 500,
        frame1_tick: 10,
        post_tick: 50,
        grace_frames: 0,
        pre_frame: 500,
        pre_sfmt_position: 500,
        pre_status: status.clone(),
        post_frame: 1_000,
        post_sfmt_position: 1_000,
        post_status: status,
        target_inputs: vec![
            TimeleapTargetInput {
                frame: 904,
                tick: 42,
            },
            TimeleapTargetInput {
                frame: 900,
                tick: 40,
            },
            TimeleapTargetInput {
                frame: 920,
                tick: 45,
            },
            TimeleapTargetInput {
                frame: 902,
                tick: 41,
            },
            TimeleapTargetInput {
                frame: 901,
                tick: 40,
            },
        ]
        .into(),
    };
    let expected = target_input_intervals(&candidate);
    let rows = group_candidates(vec![candidate.clone()]);

    assert_eq!(expected.len(), 2);
    assert_eq!(rows.len(), expected.len());
    for (row, interval) in rows.iter().zip(expected) {
        assert_eq!(row.target_frame_start, interval[0].frame);
        assert_eq!(row.target_frame_end, interval[interval.len() - 1].frame);
        assert_eq!(row.target_tick_start, interval[0].tick);
        assert_eq!(row.target_tick_end, interval[interval.len() - 1].tick);
        assert_eq!(
            row.representative_target_frame,
            interval[interval.len() / 2].frame
        );
        assert_eq!(row.target_grace_frames, interval.len() as i64 * 2);
        assert_eq!(row.candidates.as_slice(), &[candidate.clone()]);
    }
}

#[test]
fn contiguous_frame1_candidates_keep_distinct_target_pairs_and_shared_grace() {
    let status = TimeleapStatus::new(1);
    let make = |frame1, frame1_tick, target_frame, target_tick| TimeleapCandidate {
        frame1,
        frame1_tick,
        post_tick: target_tick,
        grace_frames: 0,
        pre_frame: frame1,
        pre_sfmt_position: frame1,
        pre_status: status.clone(),
        post_frame: target_frame,
        post_sfmt_position: target_frame,
        post_status: status.clone(),
        target_inputs: vec![TimeleapTargetInput {
            frame: target_frame,
            tick: target_tick,
        }]
        .into(),
    };

    let rows = group_candidates(vec![make(500, 10, 900, 40), make(502, 11, 910, 41)]);

    assert_eq!(rows.len(), 2);
    assert_eq!((rows[0].frame1_start, rows[0].frame1_end), (500, 502));
    assert_eq!(
        (rows[0].grace_frames, rows[0].representative_target_frame),
        (4, 900)
    );
    assert_eq!((rows[1].frame1_start, rows[1].frame1_end), (500, 502));
    assert_eq!(
        (rows[1].grace_frames, rows[1].representative_target_frame),
        (4, 910)
    );
    assert_eq!(rows[0].representative_frame1, 500);
    assert_eq!(rows[1].representative_frame1, 502);
    assert!(rows.iter().all(|row| row.candidates.len() == 1));
}

#[test]
fn expanding_large_contiguous_group_keeps_each_candidate_without_copying_group_payloads() {
    const CANDIDATE_COUNT: i64 = 4_096;
    let status = TimeleapStatus::new(1);
    let candidates = (0..CANDIDATE_COUNT)
        .map(|tick| TimeleapCandidate {
            frame1: 1_000 + tick * 2,
            frame1_tick: tick,
            post_tick: 10_000 + tick,
            grace_frames: 0,
            pre_frame: 1_000 + tick * 2,
            pre_sfmt_position: tick,
            pre_status: status.clone(),
            post_frame: 20_000 + tick,
            post_sfmt_position: 10_000 + tick,
            post_status: status.clone(),
            target_inputs: vec![TimeleapTargetInput {
                frame: 20_000 + tick,
                tick: 10_000 + tick,
            }]
            .into(),
        })
        .collect();

    let rows = group_candidates(candidates);

    assert_eq!(rows.len(), CANDIDATE_COUNT as usize);
    assert!(rows.iter().all(|row| {
        row.frame1_tick_start == 0
            && row.frame1_tick_end == CANDIDATE_COUNT - 1
            && row.grace_frames == CANDIDATE_COUNT * 2
            && row.candidates.len() == 1
    }));
    assert_eq!(rows[0].representative_frame1, 1_000);
    assert_eq!(rows[0].candidates[0].frame1_tick, 0);
    assert_eq!(
        rows[CANDIDATE_COUNT as usize - 1].representative_frame1,
        1_000 + (CANDIDATE_COUNT - 1) * 2
    );
    assert_eq!(
        rows[CANDIDATE_COUNT as usize - 1].candidates[0].frame1_tick,
        CANDIDATE_COUNT - 1
    );
}

#[test]
fn candidate_group_expansion_honors_cancellation() {
    const CANDIDATE_COUNT: i64 = 90_000;
    let status = TimeleapStatus::new(0);
    let candidates = (0..CANDIDATE_COUNT)
        .map(|tick| TimeleapCandidate {
            frame1: 1_000 + tick * 2,
            frame1_tick: tick,
            post_tick: 10_000 + tick,
            grace_frames: 0,
            pre_frame: 1_000 + tick * 2,
            pre_sfmt_position: tick,
            pre_status: status.clone(),
            post_frame: 20_000 + tick,
            post_sfmt_position: 10_000 + tick,
            post_status: status.clone(),
            target_inputs: vec![TimeleapTargetInput {
                frame: 20_000 + tick,
                tick: 10_000 + tick,
            }]
            .into(),
        })
        .collect();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        started_tx.send(()).expect("signal grouping start");
        groups_intersecting_tick_window_cancelable(
            candidates,
            0,
            CANDIDATE_COUNT,
            Some(&worker_cancel),
        )
    });

    started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("grouping worker started");
    std::thread::sleep(Duration::from_millis(2));
    let cancelled_at = std::time::Instant::now();
    cancel.store(true, Ordering::Release);
    let result = worker.join().expect("grouping worker must not panic");

    assert!(result
        .expect_err("large grouping must observe cancellation")
        .contains("キャンセル"));
    assert!(cancelled_at.elapsed() < Duration::from_secs(1));
}

#[test]
fn candidate_rows_match_frame1_and_target_for_each_reachable_tick() {
    let status = TimeleapStatus::new(1);
    let make = |frame1, frame1_tick, target_frame, target_tick| TimeleapCandidate {
        frame1,
        frame1_tick,
        post_tick: target_tick,
        grace_frames: 0,
        pre_frame: frame1,
        pre_sfmt_position: frame1,
        pre_status: status.clone(),
        post_frame: target_frame,
        post_sfmt_position: target_frame,
        post_status: status.clone(),
        target_inputs: vec![TimeleapTargetInput {
            frame: target_frame,
            tick: target_tick,
        }]
        .into(),
    };

    let rows = group_candidates(vec![
        make(1_000, 10, 10_000, 50),
        make(1_002, 11, 10_000, 51),
        make(1_004, 12, 10_001, 52),
    ]);

    assert_eq!(
        rows.iter()
            .map(|row| (
                row.representative_frame1,
                row.grace_frames,
                row.representative_target_frame
            ))
            .collect::<Vec<_>>(),
        vec![(1_000, 6, 10_000), (1_002, 6, 10_000), (1_004, 6, 10_001)]
    );
    assert!(rows.iter().all(|row| row.candidates.len() == 1));
}

#[test]
fn candidate_keeps_elapsed_ticks_from_frame1_to_frame2() {
    let status = TimeleapStatus::new(1);
    let candidate = TimeleapCandidate {
        frame1: 10,
        frame1_tick: 12,
        post_tick: 42,
        grace_frames: 0,
        pre_frame: 10,
        pre_sfmt_position: 10,
        pre_status: status.clone(),
        post_frame: 20,
        post_sfmt_position: 20,
        post_status: status,
        target_inputs: Vec::new().into(),
    };
    assert_eq!(candidate.frame1_to_frame2_ticks(), 30);
}

#[test]
fn selection_prefers_set_frame_and_falls_back_to_later_middle() {
    let status = TimeleapStatus::new(1);
    let make = |frame1, frame1_tick, position| TimeleapCandidate {
        frame1,
        frame1_tick,
        post_tick: frame1_tick,
        grace_frames: 0,
        pre_frame: 0,
        pre_sfmt_position: 0,
        pre_status: status.clone(),
        post_frame: 0,
        post_sfmt_position: position,
        post_status: status.clone(),
        target_inputs: Vec::new().into(),
    };
    let groups = group_candidates(vec![make(10, 0, 100), make(12, 1, 102), make(20, 3, 200)]);
    assert_eq!(
        select_candidate(&groups, Some(12))
            .unwrap()
            .post_sfmt_position,
        102
    );
    assert_eq!(
        select_candidate(&groups, Some(11))
            .unwrap()
            .representative_frame1,
        11
    );
    assert_eq!(
        select_candidate(&groups, None)
            .unwrap()
            .representative_frame1,
        12
    );
}

#[test]
fn selection_uses_exact_frame1_row_even_when_grace_runs_overlap() {
    let status = TimeleapStatus::new(1);
    let make = |frame1, frame1_tick, position, target_frame| TimeleapCandidate {
        frame1,
        frame1_tick,
        post_tick: frame1_tick + 10,
        grace_frames: 0,
        pre_frame: frame1,
        pre_sfmt_position: frame1,
        pre_status: status.clone(),
        post_frame: target_frame,
        post_sfmt_position: position,
        post_status: status.clone(),
        target_inputs: vec![TimeleapTargetInput {
            frame: target_frame,
            tick: frame1_tick + 10,
        }]
        .into(),
    };
    let rows = group_candidates(vec![
        make(1_000, 10, 100, 10_000),
        make(1_002, 11, 102, 10_001),
    ]);

    let selected = select_candidate(&rows, Some(1_002)).unwrap();
    assert_eq!(selected.representative_frame1, 1_002);
    assert_eq!(selected.post_sfmt_position, 102);
}

#[test]
fn shift_matching_is_finite_and_applies_correction_and_tolerance() {
    let predicted = vec![vec![100, 200], vec![110, 210]];
    assert_eq!(
        match_shift_times(&predicted, &[102, 202], 2, 0),
        vec![TimeleapShiftMatch {
            candidate_index: 0,
            distance: 0
        }]
    );
    assert!(match_shift_times(&predicted, &[98, 198], 2, 3).is_empty());
    assert!(match_shift_times(&predicted, &[102, 202], 2, -1).is_empty());
}

#[test]
fn reverse_consider_delay_windows_match_an_exhaustive_a_input_oracle() {
    use crate::domain::fixed_rng_catalog::StationaryDelayType as DelayType;

    struct Sample {
        name: &'static str,
        seed: u32,
        models: usize,
        mode: TimeleapMode,
        delay: ConsiderDelayConfig,
        wait_ticks: usize,
    }

    let mut samples = vec![
        Sample {
            name: "normal fixed symbol",
            seed: 0x1020_3040,
            models: 1,
            mode: TimeleapMode::Dialogue,
            delay: ConsiderDelayConfig {
                delay_frames: 0,
                generator: ConsiderDelayGenerator::FixedSymbol(DelayType::Normal),
                courier: CourierConfig::default(),
            },
            wait_ticks: 3,
        },
        Sample {
            name: "common legendary",
            seed: 0x2233_4455,
            models: 5,
            mode: TimeleapMode::Menu,
            delay: ConsiderDelayConfig {
                delay_frames: 20,
                generator: ConsiderDelayGenerator::FixedSymbol(DelayType::CommonLegendary),
                courier: CourierConfig::default(),
            },
            wait_ticks: 11,
        },
        Sample {
            name: "odd delay routed through type six",
            seed: 0x3344_5566,
            models: 15,
            mode: TimeleapMode::Menu,
            delay: ConsiderDelayConfig {
                delay_frames: 1,
                generator: ConsiderDelayGenerator::FixedSymbol(DelayType::CommonLegendary),
                courier: CourierConfig::default(),
            },
            wait_ticks: 17,
        },
        Sample {
            name: "long first Ultra Beast profile",
            seed: 0x4455_6677,
            models: 22,
            mode: TimeleapMode::Menu,
            delay: ConsiderDelayConfig {
                delay_frames: 0,
                generator: ConsiderDelayGenerator::FixedSymbol(DelayType::FirstKartana),
                courier: CourierConfig::default(),
            },
            wait_ticks: 7,
        },
        Sample {
            name: "courier No Dex",
            seed: 0x5566_7788,
            models: 5,
            mode: TimeleapMode::Courier,
            delay: ConsiderDelayConfig {
                delay_frames: 24,
                generator: ConsiderDelayGenerator::Event,
                courier: CourierConfig {
                    no_dex: true,
                    ..CourierConfig::default()
                },
            },
            wait_ticks: 13,
        },
        Sample {
            name: "courier Your ID, non-egg",
            seed: 0x6677_8899,
            models: 15,
            mode: TimeleapMode::Courier,
            delay: ConsiderDelayConfig {
                delay_frames: 10,
                generator: ConsiderDelayGenerator::Event,
                courier: CourierConfig {
                    your_id: true,
                    ..CourierConfig::default()
                },
            },
            wait_ticks: 5,
        },
        Sample {
            name: "courier egg, no delay generation",
            seed: 0x7788_99aa,
            models: 4,
            mode: TimeleapMode::Courier,
            delay: ConsiderDelayConfig {
                delay_frames: 8,
                generator: ConsiderDelayGenerator::Event,
                courier: CourierConfig {
                    your_id: true,
                    is_egg: true,
                    ..CourierConfig::default()
                },
            },
            wait_ticks: 9,
        },
    ];
    for (index, delay_type) in [
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
        DelayType::FirstGuzzlord,
    ]
    .into_iter()
    .enumerate()
    {
        samples.push(Sample {
            name: "fixed Ultra Beast delay type",
            seed: 0x8899_0000 + index as u32,
            models: 9,
            mode: TimeleapMode::Menu,
            delay: ConsiderDelayConfig {
                delay_frames: 0,
                generator: ConsiderDelayGenerator::FixedSymbol(delay_type),
                courier: CourierConfig::default(),
            },
            wait_ticks: index % 5 + 1,
        });
    }

    for sample in samples {
        let cache_end = 20_000;
        let cache = crate::domain::rng::generate_observation_cache(
            sample.seed,
            0..=cache_end,
            cache_end,
            crate::domain::timing::DEFAULT_FPS,
        )
        .unwrap_or_else(|error| panic!("{}: cache generation: {error}", sample.name));

        let mut frame1_status = TimeleapStatus::new(sample.models);
        frame1_status.is_boy = true;
        frame1_status.raining = true;
        let mut frame1_cursor = Cursor {
            frame: 480,
            sfmt_position: 480,
            tick: 0,
        };
        let mut frame1_sfmt = cached_sfmt_at(&cache, frame1_cursor.sfmt_position)
            .unwrap_or_else(|error| panic!("{}: initial SFMT: {error}", sample.name));
        apply_event(
            &mut frame1_status,
            &mut frame1_cursor,
            &mut frame1_sfmt,
            sample.mode,
            &sample.delay.courier,
        );

        let mut known_status = frame1_status.clone();
        let mut known_cursor = frame1_cursor;
        let mut known_sfmt = frame1_sfmt.clone();
        for _ in 0..sample.wait_ticks {
            advance_one_tick(&mut known_status, &mut known_cursor, &mut known_sfmt);
        }
        let known_a_frame = known_cursor.frame;
        let mut generated_status = known_status;
        let mut generated_cursor = known_cursor;
        let mut generated_sfmt = known_sfmt;
        // Finish the simulated delay under a generous cap; the resulting
        // absolute frame is the known sample produced after this A press.
        let _ = apply_consider_delay(
            &sample.delay,
            &mut generated_status,
            &mut generated_cursor,
            &mut generated_sfmt,
            sample.models,
            cache_end,
        )
        .unwrap_or_else(|error| panic!("{}: construct generated sample: {error}", sample.name));
        let target = generated_cursor.frame;

        let exhaustive_hits = |target_frame: i64| {
            let mut hits = Vec::new();
            let mut input_status = frame1_status.clone();
            let mut input_cursor = frame1_cursor;
            let mut input_sfmt = frame1_sfmt.clone();
            while input_cursor.frame <= target_frame
                && input_cursor.tick - frame1_cursor.tick < 10_000
            {
                let mut trial_status = input_status.clone();
                let mut trial_cursor = input_cursor;
                let mut trial_sfmt = cached_sfmt_at(&cache, input_cursor.sfmt_position)
                    .unwrap_or_else(|error| panic!("{}: oracle SFMT: {error}", sample.name));
                if apply_consider_delay(
                    &sample.delay,
                    &mut trial_status,
                    &mut trial_cursor,
                    &mut trial_sfmt,
                    sample.models,
                    target_frame,
                )
                .unwrap_or_else(|error| panic!("{}: oracle simulation: {error}", sample.name))
                {
                    hits.push(input_cursor.frame);
                }
                if input_cursor.frame == target_frame {
                    break;
                }
                advance_one_tick(&mut input_status, &mut input_cursor, &mut input_sfmt);
            }
            hits
        };

        let oracle = exhaustive_hits(target);
        assert!(
            oracle.contains(&known_a_frame),
            "{}: exhaustive scan must include the constructed A input {known_a_frame}",
            sample.name
        );

        let search = |target_frame: i64| {
            let config = TimeleapConfig {
                seed: sample.seed,
                start_frame: frame1_cursor.frame,
                target_frame,
                models: sample.models,
                min_frame1_elapsed_ticks: 0,
                max_frame1_elapsed_ticks: 0,
                mode: sample.mode,
                is_boy: true,
                handoff_range_frames: 0,
                courier: sample.delay.courier.clone(),
            };
            let mut status = frame1_status.clone();
            let mut cursor = frame1_cursor;
            let mut sfmt = frame1_sfmt.clone();
            let mut suffixes = HashMap::new();
            let found = search_consider_delay_input(
                &sample.delay,
                (sample.delay.delay_frames / 2 + 2) as i32,
                &config,
                frame1_cursor.tick,
                &mut status,
                &mut cursor,
                &mut sfmt,
                &cache,
                &mut suffixes,
                None,
                None,
                &mut TimeleapSearchStats::default(),
            )
            .unwrap_or_else(|error| panic!("{}: bounded search: {error}", sample.name));
            found.map(|(landed_cursor, _)| (landed_cursor.frame, cursor.frame))
        };

        assert_eq!(
                search(target),
                Some((target, *oracle.first().expect("oracle hit was asserted above"))),
                "{}: reverse window missed the known generated target {target} from A={known_a_frame}; exhaustive hits={oracle:?}",
                sample.name
            );

        let nearby_target = target.saturating_add(1);
        let nearby_oracle = exhaustive_hits(nearby_target);
        assert_eq!(
                search(nearby_target),
                nearby_oracle
                    .first()
                    .map(|first_a_frame| (nearby_target, *first_a_frame)),
                "{}: bounded search disagrees with exhaustive scan at Target={nearby_target}; hits={nearby_oracle:?}",
                sample.name
            );
    }
}

#[test]
#[ignore = "15-model / 5M Frame, 30-120 second Frame1 Consider Delay benchmark"]
fn benchmark_kartana_models15_target_five_million() {
    use crate::domain::fixed_rng_catalog::StationaryDelayType as DelayType;

    let seed = 0x6a31_9d27;
    let start_frame = 50_000;
    let target = 5_050_000;
    let cache_end = target + 311;
    let cache_started = std::time::Instant::now();
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=cache_end,
        cache_end,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate NPC1 long-range cache");
    let cache_elapsed = cache_started.elapsed();

    let config = TimeleapConfig {
        seed,
        start_frame,
        target_frame: target,
        models: 15,
        min_frame1_elapsed_ticks: 900,
        max_frame1_elapsed_ticks: 3_600,
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
    let start = TimeleapTimelineStart::initial(start_frame, config.models);

    let run = |parallelize| {
        search_candidates_from_timeline_with_stats(
            &config,
            &start,
            0,
            &cache,
            None,
            true,
            Some(&delay),
            parallelize,
        )
        .expect("search NPC1 long-range candidates")
    };

    let baseline_started = std::time::Instant::now();
    let (baseline, baseline_stats) = run(false);
    let baseline_elapsed = baseline_started.elapsed();
    assert!(baseline_stats.candidates_tested >= 2_700);
    eprintln!(
            "Kartana / models=15 / 30-120s Frame1 / target={target}: cache={cache_elapsed:?}; serial={baseline_elapsed:?} (candidate_ticks={}, merge_hits={}, output_candidates={})",
            baseline_stats.candidate_ticks_simulated,
            baseline_stats.merge_hits,
            baseline.iter().map(|group| group.candidates.len()).sum::<usize>()
        );

    for worker_count in [1, 2, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(worker_count)
            .build()
            .expect("build benchmark Rayon pool");
        let started = std::time::Instant::now();
        let (parallel, stats) = pool.install(|| run(true));
        let elapsed = started.elapsed();
        assert_eq!(parallel, baseline, "worker_count={worker_count}");
        eprintln!(
            "workers={worker_count}: elapsed={elapsed:?}, frame1_candidates={}, post_event_snapshots={}, unique_timelines={}, enumeration={}us, A_search={}us, candidate_ticks={}, merge_hits={}, shared_suffix_hits={}, suffix_misses={}, suffix_bypasses={}, suffix_waits={}, suffix_states={}, A_scan_reuse_hits={}, A_scan_misses={}, A_scan_bypasses={}, A_scan_waits={}, A_scan_states={}, cache_peak={} bytes",
                stats.candidates_tested,
                stats.post_event_snapshots,
                stats.merged_timeline_work_items,
                stats.frame1_enumeration_micros,
                stats.a_input_search_micros,
                stats.candidate_ticks_simulated,
                stats.merge_hits,
                stats.shared_suffix_cache_hits,
                stats.shared_suffix_cache_misses,
                stats.shared_suffix_cache_bypasses,
                stats.shared_suffix_cache_waits,
                stats.shared_suffix_cache_states,
                stats.consider_delay_scan_reuse_hits,
                stats.consider_delay_scan_misses,
                stats.consider_delay_scan_bypasses,
                stats.consider_delay_scan_waits,
                stats.consider_delay_scan_cached_states,
                stats.shared_cache_peak_bytes,
            );
    }

    let high_model_config = TimeleapConfig {
        models: 22,
        ..config.clone()
    };
    let high_model_start = TimeleapTimelineStart::initial(start_frame, high_model_config.models);
    let run_high_model = |parallelize| {
        search_candidates_from_timeline_with_stats(
            &high_model_config,
            &high_model_start,
            0,
            &cache,
            None,
            true,
            Some(&delay),
            parallelize,
        )
        .expect("search 22-model long-range candidates")
    };
    let serial_started = std::time::Instant::now();
    let (serial_high_model, serial_high_stats) = run_high_model(false);
    let serial_high_elapsed = serial_started.elapsed();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .expect("build 22-model benchmark Rayon pool");
    let parallel_started = std::time::Instant::now();
    let (parallel_high_model, parallel_high_stats) = pool.install(|| run_high_model(true));
    let parallel_high_elapsed = parallel_started.elapsed();
    assert_eq!(parallel_high_model, serial_high_model);
    eprintln!(
            "models=22 / target={target}: serial={serial_high_elapsed:?} (ticks={}), parallel4={parallel_high_elapsed:?} (ticks={}, prefix_ticks={}, A_window_ticks={}, prefix={}us, A_search={}us), candidates={}, unique_timelines={}, suffix_misses={}, suffix_bypasses={}, A_scan_misses={}, A_scan_bypasses={}, evictions={}, cache_peak={} bytes",
            serial_high_stats.candidate_ticks_simulated,
            parallel_high_stats.candidate_ticks_simulated,
            parallel_high_stats.frame1_to_a_ticks_simulated,
            parallel_high_stats.a_window_ticks_simulated,
            parallel_high_stats.frame1_enumeration_micros,
            parallel_high_stats.a_input_search_micros,
            parallel_high_stats.candidates_tested,
            parallel_high_stats.merged_timeline_work_items,
            parallel_high_stats.shared_suffix_cache_misses,
            parallel_high_stats.shared_suffix_cache_bypasses,
            parallel_high_stats.consider_delay_scan_misses,
            parallel_high_stats.consider_delay_scan_bypasses,
            parallel_high_stats.shared_cache_evictions,
            parallel_high_stats.shared_cache_peak_bytes,
        );
}

#[test]
#[ignore = "manual 50-million-frame Satopika/Bebenom Consider Delay benchmark"]
fn benchmark_satopika_bebenom_models15_target_fifty_million() {
    use crate::domain::fixed_rng_catalog::{
        PresetMode, StationaryDelayType as DelayType, CATEGORIES,
    };
    use std::time::Instant;

    let satopika = CATEGORIES
        .iter()
        .flat_map(|category| category.presets)
        .find(|preset| preset.species_id == 25 && preset.suffix.contains("サトピカ"))
        .expect("Satopika preset");
    let bebenom = CATEGORIES
        .iter()
        .flat_map(|category| category.presets)
        .find(|preset| preset.species_id == 803)
        .expect("Bebenom preset");
    assert_eq!(satopika.delay_frames, 20);
    assert_eq!(bebenom.delay_frames, satopika.delay_frames);
    assert_eq!(satopika.mode, PresetMode::Dialogue);
    assert_eq!(bebenom.mode, satopika.mode);
    assert_eq!(satopika.delay_type, DelayType::Normal);
    assert_eq!(bebenom.delay_type, satopika.delay_type);

    let seed = 0x6a31_9d27;
    let start_frame = 50_000;
    let target = 50_000_000;
    let cache_started = Instant::now();
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=target,
        target,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate 50M-frame SFMT cache");
    let cache_elapsed = cache_started.elapsed();

    let config = TimeleapConfig {
        seed,
        start_frame,
        target_frame: target,
        models: 15,
        min_frame1_elapsed_ticks: 30 * 30,
        max_frame1_elapsed_ticks: 120 * 30,
        mode: TimeleapMode::Dialogue,
        is_boy: true,
        handoff_range_frames: 0,
        courier: CourierConfig::default(),
    };
    let delay = ConsiderDelayConfig {
        delay_frames: i64::from(satopika.delay_frames),
        generator: ConsiderDelayGenerator::FixedSymbol(satopika.delay_type),
        courier: CourierConfig::default(),
    };
    let start = TimeleapTimelineStart::initial(start_frame, config.models);

    let started = Instant::now();
    let (candidates, stats) = search_candidates_from_timeline_with_stats(
        &config,
        &start,
        0,
        &cache,
        None,
        true,
        Some(&delay),
        true,
    )
    .expect("search 50M-frame Satopika/Bebenom candidates");
    let search_elapsed = started.elapsed();
    let output_candidates = candidates
        .iter()
        .map(|group| group.candidates.len())
        .sum::<usize>();
    assert_eq!(stats.candidates_tested, 90 * 30 + 1);
    eprintln!(
            "Satopika/Bebenom shared profile / models=15 / start={start_frame} / target={target} / Frame1=30..120s: cache_build={cache_elapsed:?}, search={search_elapsed:?}, total={:?}, frame1_candidates={}, output_candidates={output_candidates}, merged_timelines={}, prep_ticks={}, A_ticks={}, prep={}us, A_search={}us, cache_accounted={} bytes, cache_states={}, A_scan_misses={}, A_scan_bypasses={}",
            cache_elapsed + search_elapsed,
            stats.candidates_tested,
            stats.merged_timeline_work_items,
            stats.frame1_to_a_ticks_simulated,
            stats.a_window_ticks_simulated,
            stats.frame1_enumeration_micros,
            stats.a_input_search_micros,
            stats.shared_cache_peak_bytes,
            stats.shared_suffix_cache_states,
            stats.consider_delay_scan_misses,
            stats.consider_delay_scan_bypasses,
        );
    assert!(candidates
        .iter()
        .flat_map(|group| &group.candidates)
        .all(|candidate| candidate.post_frame == target
            && (30 * 30..=120 * 30).contains(&candidate.frame1_tick)));
}

#[test]
fn outputs_match_pinned_csharp_timeleap_reference_vectors() {
    // Golden fingerprints were generated with 3DSRNGTool v1.0.6 at
    // ce354abdd17c30a939b3836b0e4bb5e3d09faa92. The elapsed-time windows
    // below are chosen to cover the same Frame1 ticks as the reference
    // DelayMin/DelayMax windows. Records cover Frame1/tick, Target landing,
    // SFMT cursor, and endpoint ModelStatus snapshots. Courier vectors call
    // the reference Event7.Generate configuration paths.
    struct Vector {
        name: &'static str,
        config: TimeleapConfig,
        total_ticks: i64,
        count: usize,
        fingerprint: u64,
        first: Option<(i64, i64)>,
        last: Option<(i64, i64)>,
    }

    let courier_random = CourierConfig::default();
    let courier_constrained = CourierConfig {
        no_dex: false,
        your_id: false,
        is_egg: false,
        ec: Some(2),
        pid_type: 3,
        pid: 0x1234_5678,
        iv_count: 2,
        fixed_ivs: [31, -1, 31, -1, -1, 31],
        ability_locked: false,
        ability: 0,
        nature_locked: true,
        nature: 4,
        gender_locked: true,
        gender_ratio: 127,
    };
    let courier_fully_locked = CourierConfig {
        no_dex: false,
        your_id: false,
        is_egg: false,
        ec: Some(1),
        pid_type: 3,
        pid: 0x8765_4321,
        iv_count: 0,
        fixed_ivs: [31; 6],
        ability_locked: true,
        ability: 1,
        nature_locked: true,
        nature: 3,
        gender_locked: true,
        gender_ratio: 127,
    };
    let make_config = |seed,
                       start_frame,
                       target_frame,
                       models,
                       min_frame1_elapsed_ticks,
                       max_frame1_elapsed_ticks,
                       mode,
                       is_boy,
                       courier| TimeleapConfig {
        seed,
        start_frame,
        target_frame,
        models,
        min_frame1_elapsed_ticks,
        max_frame1_elapsed_ticks,
        mode,
        is_boy,
        handoff_range_frames: 20,
        courier,
    };
    let vectors = [
        Vector {
            name: "menu male / 1 model",
            config: make_config(
                0x1234_5678,
                480,
                1500,
                1,
                1251,
                1310,
                TimeleapMode::Menu,
                true,
                courier_random.clone(),
            ),
            total_ticks: 1341,
            count: 60,
            fingerprint: 8_207_284_081_034_480_084,
            first: Some((1421, 1251)),
            last: Some((1469, 1310)),
        },
        Vector {
            name: "menu female / 17 models",
            config: make_config(
                0x9E37_79B9,
                487,
                6000,
                17,
                357,
                401,
                TimeleapMode::Menu,
                false,
                courier_random.clone(),
            ),
            total_ticks: 417,
            count: 24,
            fingerprint: 5_596_058_291_807_269_299,
            first: Some((5308, 361)),
            last: Some((5616, 384)),
        },
        Vector {
            name: "dialogue / 1 model",
            config: make_config(
                0x436F_7E6C,
                486,
                5000,
                1,
                5279,
                5518,
                TimeleapMode::Dialogue,
                true,
                courier_random.clone(),
            ),
            total_ticks: 5579,
            count: 240,
            fingerprint: 13_248_613_412_207_775_012,
            first: Some((4799, 5279)),
            last: Some((4950, 5518)),
        },
        Vector {
            name: "dialogue / 17 models / no exact landing",
            config: make_config(
                0x7627_5C90,
                492,
                10000,
                17,
                0,
                586,
                TimeleapMode::Dialogue,
                false,
                courier_random.clone(),
            ),
            total_ticks: 687,
            count: 0,
            fingerprint: 14_695_981_039_346_656_037,
            first: None,
            last: None,
        },
        Vector {
            name: "courier / random EC and spread",
            config: make_config(
                0xA1B2_C3D4,
                480,
                5000,
                5,
                237,
                1036,
                TimeleapMode::Courier,
                true,
                courier_random.clone(),
            ),
            total_ticks: 1137,
            count: 611,
            fingerprint: 2_919_221_073_862_314_706,
            first: Some((1483, 237)),
            last: Some((4565, 1036)),
        },
        Vector {
            name: "courier / zero EC uses random generation",
            config: make_config(
                0xA1B2_C3D4,
                480,
                5000,
                5,
                237,
                1036,
                TimeleapMode::Courier,
                true,
                CourierConfig {
                    ec: Some(0),
                    ..courier_random.clone()
                },
            ),
            total_ticks: 1137,
            count: 611,
            fingerprint: 2_919_221_073_862_314_706,
            first: Some((1483, 237)),
            last: Some((4565, 1036)),
        },
        Vector {
            name: "courier / fixed EC, PID, IVs, ability, nature and gender",
            config: make_config(
                0xF4A8_3B0C,
                486,
                8000,
                12,
                0,
                594,
                TimeleapMode::Courier,
                false,
                courier_constrained,
            ),
            total_ticks: 795,
            count: 27,
            fingerprint: 9_683_677_890_030_174_050,
            first: Some((498, 1)),
            last: Some((5697, 536)),
        },
        Vector {
            name: "courier / zero event RNG consumption",
            config: make_config(
                0x0BADC0DE,
                480,
                3000,
                4,
                331,
                830,
                TimeleapMode::Courier,
                true,
                courier_fully_locked,
            ),
            total_ticks: 831,
            count: 0,
            fingerprint: 14_695_981_039_346_656_037,
            first: None,
            last: None,
        },
    ];

    for vector in vectors {
        let cache = crate::domain::rng::generate_observation_cache(
            vector.config.seed,
            0..=vector.config.target_frame,
            vector.config.target_frame,
            crate::domain::timing::DEFAULT_FPS,
        )
        .unwrap_or_else(|error| panic!("{} cache: {error}", vector.name));
        let (groups, stats) =
            search_candidates_from_cache_with_stats(&vector.config, &cache, None, true)
                .unwrap_or_else(|error| panic!("{}: {error}", vector.name));
        let (unmerged_groups, _) = search_candidates_from_cache_unmerged(&vector.config, &cache)
            .unwrap_or_else(|error| panic!("{} unmerged: {error}", vector.name));
        assert_eq!(groups, unmerged_groups, "{} merged output", vector.name);
        assert!(
            stats.normal_ticks_scanned <= vector.total_ticks as u64 + 1,
            "{} must stop normal Timeline enumeration at the target boundary",
            vector.name
        );
        let expected_tested = (vector
            .config
            .max_frame1_elapsed_ticks
            .saturating_add(1)
            .min(vector.total_ticks)
            - vector.config.min_frame1_elapsed_ticks)
            .max(0) as u64;
        assert!(
            stats.candidates_tested >= expected_tested,
            "{} must evaluate the configured window while expanding boundary groups",
            vector.name
        );
        let candidates: Vec<_> = groups
            .into_iter()
            .flat_map(|group| group.candidates)
            .filter(|candidate| {
                candidate.frame1_tick >= vector.config.min_frame1_elapsed_ticks
                    && candidate.frame1_tick <= vector.config.max_frame1_elapsed_ticks
            })
            .collect();
        assert_eq!(
            normal_target_ticks(
                &vector.config,
                &cached_sfmt_at(&cache, vector.config.start_frame).unwrap(),
            ),
            Ok(vector.total_ticks),
            "{} total ticks",
            vector.name
        );
        assert_eq!(
            candidates.len(),
            vector.count,
            "{} candidate count",
            vector.name
        );
        assert_eq!(
            candidate_fingerprint(&candidates),
            vector.fingerprint,
            "{} full output fingerprint",
            vector.name
        );
        assert_eq!(
            candidates
                .first()
                .map(|candidate| (candidate.frame1, candidate.frame1_tick)),
            vector.first,
            "{} first Frame1",
            vector.name
        );
        assert_eq!(
            candidates
                .last()
                .map(|candidate| (candidate.frame1, candidate.frame1_tick)),
            vector.last,
            "{} last Frame1",
            vector.name
        );
        assert!(candidates.iter().all(|candidate| {
            (vector.config.min_frame1_elapsed_ticks..=vector.config.max_frame1_elapsed_ticks)
                .contains(&candidate.frame1_tick)
        }));
        assert!(
            candidates.iter().all(|candidate| {
                candidate.post_frame == vector.config.target_frame
                    && candidate.post_sfmt_position == vector.config.target_frame
            }),
            "{} Frame2/SFMT landing",
            vector.name
        );
    }
}

#[test]
#[ignore = "manual long-range Frame1-search timing; run with --ignored --nocapture"]
fn timeleap_search_scaling_benchmark() {
    use std::time::Instant;

    let seed = 0x436F_7E6C;
    let start_frame = 10_000;
    let max_target = 5_000_000;
    let generation_started = Instant::now();
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=max_target,
        max_target,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate benchmark SFMT cache");
    let generation_elapsed = generation_started.elapsed();
    println!("cache through {max_target}: {generation_elapsed:?}");

    for target_frame in [100_000, 1_000_000, max_target] {
        let config = TimeleapConfig {
            seed,
            start_frame,
            target_frame,
            models: 17,
            min_frame1_elapsed_ticks: 30 * 30,
            max_frame1_elapsed_ticks: 60 * 30,
            mode: TimeleapMode::Menu,
            is_boy: false,
            handoff_range_frames: 20,
            courier: CourierConfig::default(),
        };

        let started = Instant::now();
        let (merged, stats) = search_candidates_from_cache_with_stats(&config, &cache, None, true)
            .unwrap_or_else(|error| panic!("merged target {target_frame}: {error}"));
        let merged_elapsed = started.elapsed();

        let started = Instant::now();
        let (unmerged, _) = search_candidates_from_cache_unmerged(&config, &cache)
            .unwrap_or_else(|error| panic!("unmerged target {target_frame}: {error}"));
        let unmerged_elapsed = started.elapsed();

        assert_eq!(merged, unmerged, "target {target_frame}");
        println!(
                "target={target_frame}, normal_ticks_scanned={}, candidates_tested={}, future_ticks={}, merge_hits={}, checkpoints={}, merged={merged_elapsed:?}, unmerged={unmerged_elapsed:?}",
                stats.normal_ticks_scanned,
                stats.candidates_tested,
                stats.candidate_ticks_simulated,
                stats.merge_hits,
                stats.remembered_checkpoints,
            );
    }
}

#[test]
#[ignore = "manual 50-million-frame normal Timeline timing; run with --ignored --nocapture"]
fn timeleap_50m_normal_timeline_benchmark() {
    use std::time::Instant;

    let seed = 0x436F_7E6C;
    let start_frame = 10_000;
    let target_frame = 50_000_000;
    for models in [1, 2, 17] {
        let config = TimeleapConfig {
            seed,
            start_frame,
            target_frame,
            models,
            min_frame1_elapsed_ticks: 30 * 30,
            max_frame1_elapsed_ticks: 60 * 30,
            mode: TimeleapMode::Menu,
            is_boy: false,
            handoff_range_frames: 20,
            courier: CourierConfig::default(),
        };
        let mut initial_sfmt = Sfmt::new(seed);
        for _ in 0..start_frame {
            initial_sfmt.next_u64();
        }
        let started = Instant::now();
        let ticks = normal_target_ticks_cancelable(&config, &initial_sfmt, None)
            .unwrap_or_else(|error| panic!("{models} models: {error}"));
        println!(
                "models={models}, start={start_frame}, target={target_frame}, normal_ticks={ticks}, elapsed={:?}",
                started.elapsed(),
            );
    }
}

#[test]
#[ignore = "manual 50-million-frame cache extension and Frame1 search timing"]
fn timeleap_50m_current_to_frame1_window_benchmark() {
    use std::time::Instant;

    let seed = 0x436F_7E6C;
    let start_frame = 50_000;
    let target_frame = 50_000_000;
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=start_frame + 1_000,
        start_frame + 1_000,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate SFMT cache through the current position");
    let cache_extension_started = Instant::now();
    cache
        .ensure_through(
            (target_frame + 311).min(crate::domain::rng::MAX_SFMT_FRAME),
            None,
        )
        .expect("extend SFMT cache through the distant Target");
    let cache_extension_elapsed = cache_extension_started.elapsed();
    for mode in [
        TimeleapMode::Menu,
        TimeleapMode::Dialogue,
        TimeleapMode::Courier,
    ] {
        let config = TimeleapConfig {
            seed,
            start_frame,
            target_frame,
            models: 17,
            min_frame1_elapsed_ticks: 60 * 30,
            max_frame1_elapsed_ticks: 120 * 30,
            mode,
            is_boy: false,
            handoff_range_frames: 20,
            courier: CourierConfig::default(),
        };

        let started = Instant::now();
        let (groups, stats) = search_candidates_from_cache_with_stats(&config, &cache, None, true)
            .expect("search each Frame1 in the configured elapsed-time interval");
        let elapsed = started.elapsed();
        let candidate_count: usize = groups.iter().map(|group| group.candidates.len()).sum();
        println!(
                "start={start_frame}, target={target_frame}, models={}, mode={mode:?}, interval=60..=120s, cache_extension={cache_extension_elapsed:?}, normal_ticks_scanned={}, candidates_tested={}, candidates_reaching_target={candidate_count}, replay_ticks={}, merge_hits={}, checkpoints={}, search={elapsed:?}, total={:?}",
                config.models,
                stats.normal_ticks_scanned,
                stats.candidates_tested,
                stats.candidate_ticks_simulated,
                stats.merge_hits,
                stats.remembered_checkpoints,
                cache_extension_elapsed + elapsed,
            );
        assert_eq!(stats.candidates_tested, 60 * 30 + 1);
        assert!(stats.normal_ticks_scanned <= 60 * 30 + 2);
        assert!(groups
            .iter()
            .flat_map(|group| &group.candidates)
            .all(|candidate| {
                candidate.post_frame == target_frame
                    && (60 * 30..=120 * 30).contains(&candidate.frame1_tick)
            }));
    }
}
