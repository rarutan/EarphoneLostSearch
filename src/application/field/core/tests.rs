use super::*;

fn target_metrics_from_result(
    result: &FieldResult,
    target: i64,
    values: &SearchValues,
) -> (i64, usize, bool) {
    let cursor = result.target_cursor.as_ref().unwrap();
    let mut status = cursor.status.clone();
    let mut stream_cursor = cursor.stream_cursor;
    let mut consumption = result.last_consumption;
    let mut elapsed_ticks = 0_i64;
    let mut blink_count = 0;
    while consumption < target {
        let (used, consumed) =
            next_state_with_positions_values(&mut status, &mut stream_cursor, values)
                .expect("synthetic stream must cover the target");
        consumption += used as i64;
        elapsed_ticks += 1;
        if consumed
            .iter()
            .any(|event| event.model == result.observed_model && event.blink)
        {
            blink_count += 1;
        }
    }
    (elapsed_ticks * 2, blink_count, consumption == target)
}

#[test]
fn far_field_pattern_at_49m_is_unique_and_reaches_target() {
    let seed = 0xF4BA_E999;
    let base = 49_000_000_i64;
    let range_end = base + 20_000;
    let models = 16;
    let target_model = 8;
    let values = SearchValues::new(
        base,
        crate::domain::rng::generate_u64_values_from(seed, base as usize, 1_000_000),
    );
    let mut status = ModelStatus::new(models);
    let mut stream_cursor = base;
    let mut tick = 0_i64;
    let mut blink_frames = Vec::new();
    while blink_frames.len() < 6 {
        let Some((_, consumed)) =
            next_state_with_positions_values(&mut status, &mut stream_cursor, &values)
        else {
            panic!("synthetic SFMT stream ended before six blinks");
        };
        tick += 1;
        blink_frames.extend(
            consumed
                .into_iter()
                .filter(|event| event.model == target_model && event.blink)
                .map(|_| tick * 2),
        );
    }
    let intervals = blink_frames
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect::<Vec<_>>();
    let mut target_status = status.clone();
    let mut target_cursor = stream_cursor;
    let mut target_blinks = 0;
    for _ in 0..50_000 {
        let (_, consumed) =
            next_state_with_positions_values(&mut target_status, &mut target_cursor, &values)
                .expect("synthetic SFMT stream must cover the target");
        if consumed
            .iter()
            .any(|event| event.model == target_model && event.blink)
        {
            target_blinks += 1;
        }
    }
    let config = FieldConfig {
        seed,
        range_start: base,
        range_end,
        models,
        target_model,
        observed_intervals: intervals.iter().take(2).copied().collect(),
        tolerance: 15,
        target_consumption: None,
    };
    let raw_config = reference_tick_config(&config);
    let starts = candidate_start_positions_with_span(
        base,
        range_end,
        DEFAULT_INITIAL_BRANCH_SCAN_SPAN,
        &values,
    );
    let results = starts
        .iter()
        .flat_map(|start| {
            search_v12_start_all_models(*start, &raw_config, &config, &values, false, true, None)
        })
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].start_consumption, base);
    assert_eq!(results[0].observed_model, target_model);
    let (wait_frames, target_blink_count, target_exact) =
        target_metrics_from_result(&results[0], target_cursor, &values);
    assert_eq!(wait_frames, 101_176);
    let cursor = results[0].target_cursor.as_ref().unwrap();
    let event = BlinkEvent {
        wait_frame: results[0].last_wait_frame,
        consumption: results[0].last_consumption,
        status: cursor.status.clone(),
        stream_cursor: cursor.stream_cursor,
        model_consumption: results[0].target_consumptions.last().copied(),
    };
    let target_timing = find_target_timing_cancelable(&event, target_cursor, &values, None)
        .expect("target generated from the same timeline must be reachable");
    assert!(target_exact);
    assert_eq!(target_timing.exact, target_exact);
    assert!(target_cursor <= MAX_SFMT_FRAME);
    assert_eq!(
        target_timing.absolute_wait_frame - results[0].last_wait_frame,
        wait_frames
    );
    assert!(target_blink_count > 100);
    assert!(target_blinks > 100);
}

#[test]
fn retarget_confirmed_result_keeps_the_observed_timeline() {
    let seed = 0x8322_38A9;
    let cursor = TimelineCursor {
        status: ModelStatus::new(6),
        stream_cursor: 0,
        consumption: 0,
        tick: 0,
        timeleap_runtime: None,
    };
    let result = FieldResult {
        start_consumption: 0,
        first_wait_frame: 0,
        last_wait_frame: 0,
        first_consumption: 0,
        last_consumption: 0,
        intervals: vec![12, 20],
        target_consumptions: vec![0, 3, 8],
        next_blink_wait_frame: 0,
        target_wait_frame: None,
        target_exact: None,
        target_frame_before: None,
        target_frame_after: None,
        observed_model: 0,
        models: 6,
        timeline: vec![FieldTimelineRow {
            frame: 8,
            timeline_frame: 32,
            duration_frames: 18,
        }],
        candidate_blinks: Vec::new(),
        target_blink_count: None,
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: Some(cursor.clone()),
        target_cursor: Some(cursor),
    };
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=100,
        0,
        crate::domain::timing::DEFAULT_FPS,
    )
    .unwrap();
    let original_timeline = result.timeline.clone();

    let retargeted = retarget_confirmed_result(result, seed, 80, &cache, None).unwrap();

    assert_eq!(retargeted.timeline, original_timeline);
    assert_eq!(retargeted.start_consumption, 0);
    assert!(retargeted.target_wait_frame.is_some());
    assert!(retargeted.target_blink_count.is_some());
}

#[test]
fn far_field_pattern_reaches_max_frame_without_exact_landing() {
    let seed = 0x8322_38A9;
    let base = 49_950_000_i64;
    let range_end = base + 15_000;
    let models = 23;
    let target_model = 22;
    let values = SearchValues::new(
        base,
        crate::domain::rng::generate_u64_values_from(seed, base as usize, 250_000),
    );
    let mut status = ModelStatus::new(models);
    let mut stream_cursor = base;
    let mut tick = 0_i64;
    let mut blink_frames = Vec::new();
    while blink_frames.len() < 6 {
        let Some((_, consumed)) =
            next_state_with_positions_values(&mut status, &mut stream_cursor, &values)
        else {
            panic!("synthetic SFMT stream ended before six blinks");
        };
        tick += 1;
        blink_frames.extend(
            consumed
                .into_iter()
                .filter(|event| event.model == target_model && event.blink)
                .map(|_| tick * 2),
        );
    }
    let intervals = blink_frames
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect::<Vec<_>>();
    let mut target_status = status.clone();
    let mut target_cursor = stream_cursor;
    let mut target_blinks = 0;
    while target_cursor < MAX_SFMT_FRAME {
        let (_, consumed) =
            next_state_with_positions_values(&mut target_status, &mut target_cursor, &values)
                .expect("synthetic SFMT stream must cover the target");
        if consumed
            .iter()
            .any(|event| event.model == target_model && event.blink)
        {
            target_blinks += 1;
        }
    }
    let config = FieldConfig {
        seed,
        range_start: base,
        range_end,
        models,
        target_model,
        observed_intervals: intervals.iter().take(2).copied().collect(),
        tolerance: 15,
        target_consumption: None,
    };
    let raw_config = reference_tick_config(&config);
    let starts = candidate_start_positions_with_span(
        base,
        range_end,
        DEFAULT_INITIAL_BRANCH_SCAN_SPAN,
        &values,
    );
    let results = starts
        .iter()
        .flat_map(|start| {
            search_v12_start_all_models(*start, &raw_config, &config, &values, false, true, None)
        })
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].start_consumption, base);
    assert_eq!(results[0].observed_model, target_model);
    let matched_blinks = results[0]
        .candidate_blinks
        .iter()
        .filter(|blink| blink.observed)
        .map(|blink| blink.timeline_frame)
        .collect::<Vec<_>>();
    assert_eq!(matched_blinks, blink_frames[..3]);
    assert_eq!(
        matched_blinks
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .collect::<Vec<_>>(),
        vec![528, 358]
    );
    let continuation_cursor = results[0].target_cursor.as_ref().unwrap();
    let mut continuation_status = continuation_cursor.status.clone();
    let mut continuation_stream_cursor = continuation_cursor.stream_cursor;
    let mut continuation_tick = continuation_cursor.tick;
    let mut direct_following_blinks = Vec::new();
    while direct_following_blinks.len() < 10 {
        let Some((_, consumed)) = next_state_with_positions_values(
            &mut continuation_status,
            &mut continuation_stream_cursor,
            &values,
        ) else {
            panic!("synthetic SFMT stream ended during post-match continuation");
        };
        continuation_tick += 1;
        if consumed
            .iter()
            .any(|event| event.model == target_model && event.blink)
        {
            direct_following_blinks.push(continuation_tick * 2);
        }
    }
    let cached_following_blinks = results[0]
        .candidate_blinks
        .iter()
        .filter(|blink| !blink.observed)
        .take(10)
        .map(|blink| blink.timeline_frame)
        .collect::<Vec<_>>();
    assert_eq!(cached_following_blinks, direct_following_blinks);
    let mut previous_blink = *matched_blinks.last().unwrap();
    let following_intervals = cached_following_blinks
        .iter()
        .map(|blink_frame| {
            let interval = *blink_frame - previous_blink;
            previous_blink = *blink_frame;
            interval
        })
        .collect::<Vec<_>>();
    println!("case2 intervals after observed blinks (F): {following_intervals:?}");
    let target_frame = MAX_SFMT_FRAME;
    let (wait_frames, target_blink_count, target_exact) =
        target_metrics_from_result(&results[0], target_frame, &values);
    let cursor = results[0].target_cursor.as_ref().unwrap();
    let event = BlinkEvent {
        wait_frame: results[0].last_wait_frame,
        consumption: results[0].last_consumption,
        status: cursor.status.clone(),
        stream_cursor: cursor.stream_cursor,
        model_consumption: results[0].target_consumptions.last().copied(),
    };
    let target_timing = find_target_timing_cancelable(&event, target_frame, &values, None)
        .expect("target generated from the same timeline must be reachable");
    assert_eq!(target_timing.exact, target_exact);
    assert!(!target_exact);
    assert_eq!(wait_frames, 4_528);
    assert_eq!(
        target_timing.absolute_wait_frame - results[0].last_wait_frame,
        wait_frames
    );
    assert!(target_blink_count > 0);
    assert!(target_blinks > 0);
    assert_eq!(target_cursor, MAX_SFMT_FRAME + 8);
}

#[test]
fn timeline_display_history_is_bounded_without_truncating_candidate_history() {
    let observed_events = (0..(MAX_RETAINED_TIMELINE_ROWS + 44))
        .map(|index| BlinkEvent {
            wait_frame: index as i64 * 2,
            consumption: index as i64,
            status: ModelStatus::new(3),
            stream_cursor: index as i64,
            model_consumption: Some(index as i64),
        })
        .collect::<Vec<_>>();
    let expected_last = observed_events.last().unwrap().clone();
    let values = SearchValues::new(0, Vec::new());

    let (rows, candidate_blinks, cursor) =
        build_timeline(&observed_events, 0, Some(50_000_000), &values);

    assert_eq!(rows.len(), MAX_RETAINED_TIMELINE_ROWS);
    assert_eq!(
        rows.first().map(|row| row.timeline_frame),
        Some(observed_events[44].wait_frame)
    );
    assert_eq!(candidate_blinks.len(), observed_events.len());
    assert_eq!(cursor.status, expected_last.status);
    assert_eq!(cursor.stream_cursor, expected_last.stream_cursor);
    assert_eq!(cursor.consumption, expected_last.consumption);
    assert_eq!(cursor.tick, expected_last.wait_frame / 2);
}

#[test]
fn correction_uses_the_timeline_tick_delta_for_frame_boundaries() {
    let cursor = TimelineCursor {
        status: ModelStatus::new(6),
        stream_cursor: 0,
        consumption: 0,
        tick: 0,
        timeleap_runtime: None,
    };
    let result = FieldResult {
        start_consumption: 0,
        first_wait_frame: 0,
        last_wait_frame: 0,
        first_consumption: 0,
        last_consumption: 0,
        intervals: Vec::new(),
        target_consumptions: Vec::new(),
        next_blink_wait_frame: 0,
        target_wait_frame: None,
        target_exact: None,
        target_frame_before: None,
        target_frame_after: None,
        observed_model: 0,
        models: 6,
        timeline: Vec::new(),
        candidate_blinks: Vec::new(),
        target_blink_count: None,
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: Some(cursor.clone()),
        target_cursor: Some(cursor),
    };
    let cache = crate::domain::rng::generate_observation_cache(
        0,
        0..=100,
        100,
        crate::domain::timing::DEFAULT_FPS,
    )
    .unwrap();
    let corrected = suggest_field_offset_from_result_with_cache_cancelable(
        0,
        &result,
        6,
        12,
        4,
        Some(&cache),
        None,
    )
    .unwrap();
    assert_eq!(corrected, Some(2));
}

#[test]
fn correction_maps_an_exact_consumption_boundary_to_its_reaching_tick() {
    let seed = 0;
    let models = 6;
    let mut status = ModelStatus::new(models);
    let mut stream_cursor = 0;
    let values = SearchValues::new(0, crate::domain::rng::generate_u64_values(seed, 100));
    let (used, _) = next_state_with_positions_values(&mut status, &mut stream_cursor, &values)
        .expect("the synthetic SFMT range covers the first transition");
    assert!(used > 0);

    let cursor = TimelineCursor {
        status: ModelStatus::new(models),
        stream_cursor: 0,
        consumption: 0,
        tick: 0,
        timeleap_runtime: None,
    };
    let event = FieldResult {
        start_consumption: 0,
        first_wait_frame: 0,
        last_wait_frame: 0,
        first_consumption: 0,
        last_consumption: 0,
        intervals: Vec::new(),
        target_consumptions: Vec::new(),
        next_blink_wait_frame: 0,
        target_wait_frame: None,
        target_exact: None,
        target_frame_before: None,
        target_frame_after: None,
        observed_model: 0,
        models,
        timeline: Vec::new(),
        candidate_blinks: Vec::new(),
        target_blink_count: None,
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: Some(cursor.clone()),
        target_cursor: Some(cursor),
    };
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=100,
        100,
        crate::domain::timing::DEFAULT_FPS,
    )
    .unwrap();

    assert_eq!(
        find_correction_tick_from_cursor(seed, &event, used as i64, Some(&cache), None).unwrap(),
        Some(1)
    );
}

#[test]
fn reported_field_offset_case_keeps_the_one_tick_difference() {
    let seed = 0x7627_5C90;
    let models = 17;
    let start = 492;
    let cursor = TimelineCursor {
        status: ModelStatus::new(models),
        stream_cursor: start,
        consumption: start,
        tick: 0,
        timeleap_runtime: None,
    };
    let event = FieldResult {
        start_consumption: start,
        first_wait_frame: 0,
        last_wait_frame: 0,
        first_consumption: start,
        last_consumption: start,
        intervals: Vec::new(),
        target_consumptions: Vec::new(),
        next_blink_wait_frame: 0,
        target_wait_frame: None,
        target_exact: None,
        target_frame_before: None,
        target_frame_after: None,
        observed_model: 16,
        models,
        timeline: Vec::new(),
        candidate_blinks: Vec::new(),
        target_blink_count: None,
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: Some(cursor.clone()),
        target_cursor: Some(cursor),
    };
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=112_993,
        112_993,
        crate::domain::timing::DEFAULT_FPS,
    )
    .unwrap();

    assert_eq!(
        suggest_field_offset_from_result_with_cache_cancelable(
            seed,
            &event,
            112_993,
            112_980,
            0,
            Some(&cache),
            None,
        )
        .unwrap(),
        Some(2)
    );
}

#[test]
fn candidate_starts_include_only_special_boundaries_near_the_base() {
    let mut values = vec![1_u64; 64];
    values[5] = 128;
    values[10] = 256;
    values[40] = 384;
    let search_values = SearchValues::new(100, values);

    assert_eq!(
        candidate_start_positions(100, 200, &search_values),
        vec![100, 106, 111]
    );
}

#[test]
fn configured_initial_branch_scan_span_controls_special_boundaries() {
    let mut values = vec![1_u64; 64];
    values[40] = 128;
    let search_values = SearchValues::new(100, values);

    assert_eq!(
        candidate_start_positions_with_span(100, 200, 10, &search_values),
        vec![100]
    );
    assert_eq!(
        candidate_start_positions_with_span(100, 200, 40, &search_values),
        vec![100, 141]
    );
}

#[test]
fn reference_tick_config_uses_gui_tolerance_in_display_frames() {
    let mut config = FieldConfig {
        seed: 1,
        range_start: 0,
        range_end: 100,
        models: 2,
        target_model: 0,
        observed_intervals: vec![15, 20],
        tolerance: 5,
        target_consumption: None,
    };
    assert_eq!(reference_tick_config(&config).tolerance, 3);

    config.tolerance = 20;
    assert_eq!(reference_tick_config(&config).tolerance, 10);
}

#[test]
fn npc_model_conversion_only_moves_forward_from_candidate_start() {
    assert_eq!(shifted_start_for_requested_model(480, 2, 0), Some(482));
    assert_eq!(shifted_start_for_requested_model(480, 9, 6), Some(483));
    assert_eq!(shifted_start_for_requested_model(485, 4, 6), None);
    assert_eq!(shifted_start_for_requested_model(485, 7, 9), None);
}

#[test]
fn npc_model_conversion_rebuilds_a_matching_timeline() {
    let base = 480_i64;
    let models = 13_usize;
    let mut converted = None;

    for seed in 0_u32..500 {
        let traces = generated_target_streams(seed, base, models, 240);
        let (positions, waits) = &traces[2];
        if positions.len() < 4 {
            continue;
        }
        let positions = positions[..4].to_vec();
        let intervals = waits[..4]
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .collect::<Vec<_>>();
        let config = FieldConfig {
            seed,
            range_start: base,
            range_end: base + 240,
            models,
            target_model: 0,
            observed_intervals: intervals.clone(),
            tolerance: 0,
            target_consumption: None,
        };
        let raw_config = reference_tick_config(&config);
        let stream_end = reference_search_stream_end(&raw_config);
        let values = SearchValues::new(
            base,
            generate_u64_values_from(
                seed,
                base as usize,
                stream_end.saturating_sub(base).saturating_add(1) as usize,
            ),
        );
        let raw =
            search_v12_start_all_models(base, &raw_config, &config, &values, true, true, None);
        if !raw.iter().any(|result| {
            result.observed_model == 2
                && result.intervals == intervals
                && result.target_consumptions == positions
        }) {
            continue;
        }
        let resolved = resolve_observed_model_candidates(raw, &config, &raw_config, &values, None);
        if resolved.iter().any(|result| {
            result.start_consumption == base + 2
                && result.observed_model == 0
                && result.intervals == intervals
                && result.target_consumptions == positions
        }) {
            converted = Some(seed);
            break;
        }
    }

    assert!(
        converted.is_some(),
        "NPC番号変換後に再生成したTimelineが観測列へ一致しませんでした"
    );
    println!("NPC番号変換の実Timeline検証Seed: {:?}", converted);
}

#[test]
fn multiple_initial_special_boundaries_keep_all_branch_starts() {
    let base = 480_i64;
    let scan_end = base + INITIAL_BRANCH_SCAN_SPAN;
    let mut examples = Vec::new();

    // 実際のSFMT列から、基準位置の近傍に特殊値が2つ以上あるSeedを探す。
    // ここで見つけた開始位置は、通常の基準Timelineと各-5分岐Timelineの候補になる。
    for seed in 0_u32..50_000 {
        let values = generate_u64_values_from(seed, base as usize, (scan_end - base + 1) as usize);
        let search_values = SearchValues::new(base, values.clone());
        let starts = candidate_start_positions(base, base + 100, &search_values);
        if starts.len() < 3 {
            continue;
        }
        let special_starts = (base..=scan_end)
            .filter(|position| {
                search_values
                    .get(*position)
                    .is_some_and(|value| value & 0x7f == 0)
            })
            .map(|position| position + 1)
            .collect::<Vec<_>>();
        assert_eq!(starts, {
            let mut expected = vec![base];
            expected.extend(special_starts.iter().copied());
            expected.sort_unstable();
            expected.dedup();
            expected
        });
        examples.push((seed, special_starts, starts));
        if examples.len() == 3 {
            break;
        }
    }

    assert!(
        examples.len() == 3,
        "近傍に-5分岐が2つ以上あるSeedを見つけられませんでした"
    );
    println!("複数-5分岐の検証Seed: {examples:#?}");
}

fn synthetic_result(
    start_consumption: i64,
    observed_model: usize,
    intervals: Vec<i64>,
    target_consumptions: Vec<i64>,
) -> FieldResult {
    FieldResult {
        start_consumption,
        first_wait_frame: 50,
        last_wait_frame: 50 + intervals.iter().sum::<i64>(),
        first_consumption: target_consumptions[0],
        last_consumption: *target_consumptions.last().unwrap(),
        intervals,
        target_consumptions,
        next_blink_wait_frame: 0,
        target_wait_frame: None,
        target_exact: None,
        target_frame_before: None,
        target_frame_after: None,
        observed_model,
        models: 13,
        timeline: Vec::new(),
        candidate_blinks: Vec::new(),
        target_blink_count: None,
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: None,
    }
}

#[test]
fn frame1_row_counts_from_one_boundary_match_the_suffix_reference() {
    let mut result = synthetic_result(0, 0, vec![1, 1, 1], vec![0, 1, 2, 3]);
    result.timeline = [0, 2, 10, 12]
        .into_iter()
        .map(|frame| FieldTimelineRow {
            frame,
            timeline_frame: frame,
            duration_frames: 2,
        })
        .collect();

    for frame1 in [0, 1, 2, 3, 10, 12, 13] {
        let boundary = result.frame_boundary_index(frame1);
        for row in 0..result.timeline.len() {
            let optimized = boundary.and_then(|boundary| {
                result.blinks_through_frame_after_row_at_boundary(row, frame1, boundary)
            });
            assert_eq!(
                optimized,
                result.blinks_through_frame_after_row(row, frame1),
                "row={row}, Frame1={frame1}"
            );
        }
    }
    assert_eq!(result.frame_boundary_index(12), Some(4));
    assert_eq!(result.frame_boundary_index(13), None);
}

#[test]
fn observable_candidates_keep_different_sfmt_positions() {
    let candidates = vec![
        synthetic_result(1_000, 5, vec![506, 362], vec![1288, 3739, 5522]),
        synthetic_result(1_020, 12, vec![502, 362], vec![1284, 3739, 5522]),
    ];
    let merged = merge_observable_candidates(candidates);
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0].start_consumption, 1_000);
}

#[test]
fn timeline_extension_snapshot_omits_candidate_blink_history() {
    let mut result = synthetic_result(1_000, 0, vec![20, 24], vec![1_001, 1_010, 1_022]);
    result.candidate_blinks = vec![
        CandidateBlink {
            frame: 1_001,
            timeline_frame: 20,
            observed: true,
            target_consumption: Some(1_010),
        },
        CandidateBlink {
            frame: 1_022,
            timeline_frame: 44,
            observed: false,
            target_consumption: Some(1_022),
        },
    ];

    let snapshot = result.timeline_extension_snapshot();

    assert!(snapshot.candidate_blinks.is_empty());
    assert_eq!(snapshot.intervals, result.intervals);
    assert_eq!(snapshot.target_consumptions, result.target_consumptions);
    assert_eq!(snapshot.timeline, result.timeline);
}

#[test]
fn observable_candidates_stay_separate_when_target_stream_diverges() {
    let candidates = vec![
        synthetic_result(1_000, 5, vec![506, 362], vec![1288, 3739, 5522]),
        synthetic_result(1_020, 12, vec![502, 362], vec![1284, 3739, 5523]),
    ];
    let merged = merge_observable_candidates(candidates);
    assert_eq!(merged.len(), 2);
}

fn generated_target_streams(
    seed: u32,
    start_consumption: i64,
    models: usize,
    ticks: usize,
) -> Vec<(Vec<i64>, Vec<i64>)> {
    let values = generate_u64_values_from(
        seed,
        start_consumption as usize,
        models.saturating_mul(ticks).saturating_add(64),
    );
    let mut status = ModelStatus::new(models);
    let mut stream_cursor = start_consumption;
    let mut value_index = 0;
    let mut streams = vec![(Vec::new(), Vec::new()); models];
    for tick in 1..=ticks {
        let (_used, consumed) = status.next_state_with_positions(|| {
            let position = stream_cursor;
            let value = values[value_index];
            value_index += 1;
            stream_cursor += 1;
            (position, value)
        });
        for event in consumed.into_iter().filter(|event| event.blink) {
            streams[event.model].0.push(event.position);
            streams[event.model].1.push(tick as i64 * 2);
        }
    }
    streams
}

#[test]
fn generated_sfmt_patterns_merge_converged_candidates_and_keep_divergence() {
    let mut tested_converged_pairs = 0;
    let mut pseudo_seed = 0x1357_9bdf_u32;
    for _ in 0..24 {
        pseudo_seed = pseudo_seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        let seed = pseudo_seed;
        for &models in &[5_usize, 10, 13, 20] {
            let starts = [480_i64, 481, 482, 483, 484];
            let traces = starts
                .iter()
                .map(|start| generated_target_streams(seed, *start, models, 400))
                .collect::<Vec<_>>();
            for left_index in 0..starts.len() {
                for right_index in (left_index + 1)..starts.len() {
                    for left_model in 0..models {
                        let (left_positions, left_waits) = &traces[left_index][left_model];
                        let left_intervals = left_waits
                            .windows(2)
                            .map(|pair| pair[1] - pair[0])
                            .collect::<Vec<_>>();
                        if left_intervals.len() < 2 {
                            continue;
                        }
                        for right_model in 0..models {
                            let (right_positions, right_waits) = &traces[right_index][right_model];
                            let right_intervals = right_waits
                                .windows(2)
                                .map(|pair| pair[1] - pair[0])
                                .collect::<Vec<_>>();
                            if left_intervals.get(1..) != right_intervals.get(1..)
                                || left_positions.get(1..) != right_positions.get(1..)
                                || (left_intervals.first() == right_intervals.first()
                                    && left_positions.first() == right_positions.first())
                            {
                                continue;
                            }
                            let candidates = vec![
                                synthetic_result(
                                    starts[left_index],
                                    left_model,
                                    left_intervals.clone(),
                                    left_positions.clone(),
                                ),
                                synthetic_result(
                                    starts[right_index],
                                    right_model,
                                    right_intervals,
                                    right_positions.clone(),
                                ),
                            ];
                            assert_eq!(merge_observable_candidates(candidates).len(), 2);
                            tested_converged_pairs += 1;
                        }
                    }
                }
            }
        }
    }

    assert!(
        tested_converged_pairs > 0,
        "ランダムに生成した範囲で収束する候補を見つけられませんでした"
    );

    let streams_a = generated_target_streams(1, 1_000, 13, 400);
    let streams_b = generated_target_streams(1, 1_020, 13, 400);
    let left = &streams_a[5];
    let right = &streams_b[12];
    let left_intervals = left
        .1
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect::<Vec<_>>();
    let right_intervals = right
        .1
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect::<Vec<_>>();
    assert_eq!(left_intervals.get(1..), right_intervals.get(1..));
    assert_eq!(left.0.get(1..), right.0.get(1..));
    assert_ne!(left.0.first(), right.0.first());
    assert_eq!(
        merge_observable_candidates(vec![
            synthetic_result(1_000, 5, left_intervals.clone(), left.0.clone()),
            synthetic_result(1_020, 12, right_intervals.clone(), right.0.clone()),
        ])
        .len(),
        2
    );

    let mut divergent_positions = right.0.clone();
    let last = divergent_positions
        .last_mut()
        .expect("収束ケースには複数の瞬きが必要です");
    *last += 1;
    assert_eq!(
        merge_observable_candidates(vec![
            synthetic_result(1_000, 5, left_intervals.clone(), left.0.clone()),
            synthetic_result(1_020, 12, right_intervals, divergent_positions),
        ])
        .len(),
        2
    );
}
