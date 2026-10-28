use super::*;

#[test]
fn searches_all_start_consumptions() {
    let config = FieldConfig {
        seed: 1,
        range_start: 0,
        range_end: 50_000,
        models: 1,
        target_model: 0,
        observed_intervals: vec![1],
        tolerance: 1_000,
        target_consumption: None,
    };
    let results = search(&config).unwrap();
    assert!(results.len() > 1);
    // 1つのSFMT開始位置から複数の観測窓が生じるため、順序は一意ではなく
    // 非減少となる。
    assert!(results
        .windows(2)
        .all(|rows| { rows[0].start_consumption <= rows[1].start_consumption }));
    assert!(results.iter().all(|result| {
        result
            .candidate_blinks
            .iter()
            .any(|blink| blink.frame == result.last_consumption && blink.observed)
    }));
}

#[test]
fn shared_progression_cache_answers_target_like_reference_replay() {
    let config = FieldConfig {
        seed: 1,
        range_start: 0,
        range_end: 10,
        models: 1,
        target_model: 0,
        observed_intervals: vec![1],
        tolerance: 1_000,
        target_consumption: None,
    };
    let result = search(&config).unwrap().remove(0);
    let observation_cache = crate::domain::rng::generate_observation_cache(
        config.seed,
        0..=10,
        100_000,
        crate::domain::timing::DEFAULT_FPS,
    )
    .unwrap();
    let mut state = FieldState {
        seed: format!("{:08X}", config.seed),
        results: vec![result],
        observed: vec![Instant::now()],
        ..FieldState::default()
    };
    state.set_observation_cache(observation_cache);

    let observed_result = state.results[0].clone();
    for target in [
        observed_result.last_consumption + 500,
        observed_result.last_consumption + 5_000,
        observed_result.last_consumption + 1_000,
    ] {
        let reference = crate::application::field::retarget_confirmed_result(
            observed_result.clone(),
            config.seed,
            target,
            state.observation_cache.as_ref().unwrap(),
            None,
        )
        .unwrap();
        let cached = state
            .retarget_confirmed_result_from_cache(target, None)
            .unwrap();

        assert_eq!(cached.target_wait_frame, reference.target_wait_frame);
        assert_eq!(cached.target_exact, reference.target_exact);
        assert_eq!(cached.target_frame_before, reference.target_frame_before);
        assert_eq!(cached.target_frame_after, reference.target_frame_after);
        assert_eq!(cached.target_blink_count, reference.target_blink_count);
    }
    let generated_ticks = state
        .resolved_timeline_cache
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .tick_deltas
        .len();
    state
        .retarget_confirmed_result_from_cache(observed_result.last_consumption + 1_000, None)
        .unwrap();
    assert_eq!(
        state
            .resolved_timeline_cache
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .tick_deltas
            .len(),
        generated_ticks,
        "backward Target edits should reuse the generated prefix"
    );

    let anchor = observed_result.target_cursor.as_ref().unwrap();
    let mut reference_sfmt = state
        .observation_cache
        .as_ref()
        .unwrap()
        .sfmt_at_checked(anchor.stream_cursor)
        .unwrap()
        .unwrap();
    let mut reference_status = anchor.status.clone();
    let mut reference_cursor = anchor.stream_cursor;
    let mut reference_consumption = anchor.consumption;
    let mut expected_deltas = Vec::with_capacity(generated_ticks);
    let mut expected_blinks = Vec::new();
    for offset in 0..generated_ticks {
        let (used, consumed) = reference_status.next_state_with_positions(|| {
            let position = reference_cursor;
            let value = reference_sfmt.next_u64();
            reference_cursor += 1;
            (position, value)
        });
        reference_consumption += used as i64;
        expected_deltas.push(used as u8);
        if consumed
            .iter()
            .any(|event| event.model == observed_result.observed_model && event.blink)
        {
            expected_blinks.push(anchor.tick + offset as i64 + 1);
        }
    }
    let shared = state
        .resolved_timeline_cache
        .as_ref()
        .unwrap()
        .lock()
        .unwrap();
    assert_eq!(shared.tick_deltas, expected_deltas);
    assert_eq!(
        shared
            .blinks
            .iter()
            .map(|event| i64::from(event.tick))
            .collect::<Vec<_>>(),
        expected_blinks
    );
    assert_eq!(shared.cursor.consumption, reference_consumption);
    for offset in [0, generated_ticks / 2, generated_ticks] {
        let expected = anchor.consumption
            + expected_deltas[..offset]
                .iter()
                .map(|delta| i64::from(*delta))
                .sum::<i64>();
        assert_eq!(
            shared.consumption_at_tick(anchor.tick + offset as i64),
            expected
        );
    }
}

#[test]
fn observed_intervals_can_start_at_a_later_blink() {
    let values = SearchValues::new(0, generate_u64_stream(1, 10_000));
    let mut status = ModelStatus::new(1);
    let mut stream_cursor = 0;
    let mut events = Vec::new();
    for tick in 0..MAX_WAIT_TICKS {
        let Some((_used, blink)) = next_state_with_values(&mut status, &mut stream_cursor, &values)
        else {
            break;
        };
        if blink.contains(&0) {
            events.push((tick + 1) * 2);
        }
    }
    let window_len = 4;
    let selected = (1..events.len().saturating_sub(window_len - 1))
        .find(|&start| {
            let expected = events[start + 1..start + window_len]
                .windows(2)
                .map(|pair| pair[1] - pair[0])
                .collect::<Vec<_>>();
            !events[..start].windows(window_len).any(|window| {
                window
                    .windows(2)
                    .map(|pair| pair[1] - pair[0])
                    .eq(expected.iter().copied())
            })
        })
        .expect("deterministic SFMT stream should provide a later unique window");
    let config = FieldConfig {
        seed: 1,
        range_start: 0,
        range_end: 10_000,
        models: 1,
        target_model: 0,
        observed_intervals: events[selected..selected + window_len]
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .collect(),
        tolerance: 0,
        target_consumption: None,
    };
    let result = search_candidate(0, &config, &values).expect("later blink window matches");
    assert!(selected > 0);
    assert_eq!(result.first_wait_frame, events[selected]);
}

#[test]
fn targeted_search_refines_interval_matches_after_chunk_scan() {
    let config = FieldConfig {
        seed: 1,
        range_start: 0,
        range_end: 100,
        models: 2,
        target_model: 0,
        observed_intervals: vec![1],
        tolerance: 1_000,
        target_consumption: Some(5_000),
    };
    let results = search(&config).unwrap();
    assert!(!results.is_empty());
    assert!(results
        .iter()
        .all(|result| result.target_wait_frame.is_some() || result.target_exact == Some(false)));
}

#[test]
fn search_values_are_anchored_at_range_start() {
    let full = crate::domain::rng::generate_u64_values(1, 32);
    let anchored = generate_search_values(&FieldConfig {
        seed: 1,
        range_start: 10,
        range_end: 12,
        models: 1,
        target_model: 0,
        observed_intervals: vec![1],
        tolerance: 0,
        target_consumption: None,
    });

    assert_eq!(anchored.base, 10);
    assert_eq!(&anchored.values[..5], &full[10..15]);
}

#[test]
fn search_pool_can_be_prepared_before_blink_intervals_exist() {
    let config = FieldConfig {
        seed: 1,
        range_start: 10,
        range_end: 12,
        models: 2,
        target_model: 0,
        observed_intervals: Vec::new(),
        tolerance: 5,
        target_consumption: Some(20),
    };
    let pool = prepare_search_pool(&config).unwrap();
    assert_eq!(pool.cache.read_values(10, 1).unwrap().len(), 1);
    assert!(pool.covers(&config));
}

#[test]
fn cancelled_pool_generation_returns_without_building_results() {
    let config = FieldConfig {
        seed: 1,
        range_start: 0,
        range_end: 100,
        models: 2,
        target_model: 0,
        observed_intervals: Vec::new(),
        tolerance: 5,
        target_consumption: None,
    };
    let token = AtomicBool::new(true);
    assert!(matches!(
        prepare_search_pool_with_configs_cancelable(&[config], Some(&token)),
        Err(AppError::Cancelled)
    ));
}

#[test]
fn rejects_invalid_target_model() {
    let config = FieldConfig {
        seed: 1,
        range_start: 0,
        range_end: 1_000,
        models: 1,
        target_model: 1,
        observed_intervals: vec![1],
        tolerance: 0,
        target_consumption: None,
    };
    assert!(search(&config).is_err());
}

#[test]
fn ui_idx_controls_observed_model() {
    let mut state = FieldState {
        npc_count: "2".into(),
        target_model: "0".into(),
        ..FieldState::default()
    };
    assert_eq!(state.config().unwrap().models, 3);
    assert_eq!(state.config().unwrap().target_model, 0);
    state.target_model = "1".into();
    assert_eq!(state.config().unwrap().target_model, 1);
}

#[test]
fn normalize_keeps_distinct_target_sfmt_branches_separate() {
    let make_result = |target_consumptions| FieldResult {
        start_consumption: 500,
        first_wait_frame: 100,
        last_wait_frame: 200,
        first_consumption: 600,
        last_consumption: 700,
        intervals: vec![100],
        target_consumptions,
        next_blink_wait_frame: 100,
        target_wait_frame: None,
        target_exact: None,
        target_frame_before: None,
        target_frame_after: None,
        observed_model: 5,
        models: 13,
        timeline: Vec::new(),
        candidate_blinks: Vec::new(),
        target_blink_count: None,
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: None,
    };
    let mut state = FieldState {
        target_model: "5".into(),
        ..FieldState::default()
    };
    let normalized = state.normalize_search_results(vec![
        make_result(vec![610, 710]),
        make_result(vec![611, 710]),
    ]);
    assert_eq!(normalized.len(), 2);
}

#[test]
fn normalize_keeps_candidates_with_different_internal_states() {
    let make_result =
        |start_consumption: i64, tick: i64, target_consumptions: Vec<i64>| FieldResult {
            start_consumption,
            first_wait_frame: 100,
            last_wait_frame: 200,
            first_consumption: target_consumptions[0],
            last_consumption: *target_consumptions.last().unwrap(),
            intervals: vec![100, 100],
            target_consumptions,
            next_blink_wait_frame: 100,
            target_wait_frame: None,
            target_exact: None,
            target_frame_before: None,
            target_frame_after: None,
            observed_model: 10,
            models: 13,
            timeline: Vec::new(),
            candidate_blinks: Vec::new(),
            target_blink_count: None,
            target_blink_count_offset_from_first_row: 0,
            timeline_cursor: None,
            target_cursor: Some(TimelineCursor {
                status: ModelStatus {
                    remain: vec![1, 1, 1, 1, 1, 1, 1, 3, 1, 31, 30, 1, 1],
                },
                stream_cursor: 4_290,
                consumption: 4_290,
                tick,
                timeleap_runtime: None,
            }),
        };
    let mut state = FieldState {
        target_model: "10".into(),
        ..FieldState::default()
    };
    let left = make_result(482, 369, vec![1_005, 4_287, 4_939]);
    let mut right = make_result(508, 364, vec![1_007, 4_287, 4_939]);
    right.target_cursor.as_mut().unwrap().status.remain[1] = 2;
    let normalized = state.normalize_search_results(vec![left, right]);
    assert_eq!(normalized.len(), 2);
}

#[test]
fn normalize_merges_candidates_with_identical_full_state() {
    let make_result = |start_consumption: i64, intervals, target_consumptions| FieldResult {
        start_consumption,
        first_wait_frame: 100,
        last_wait_frame: 200,
        first_consumption: 600,
        last_consumption: 700,
        intervals,
        target_consumptions,
        next_blink_wait_frame: 100,
        target_wait_frame: None,
        target_exact: None,
        target_frame_before: None,
        target_frame_after: None,
        observed_model: 8,
        models: 23,
        timeline: Vec::new(),
        candidate_blinks: Vec::new(),
        target_blink_count: None,
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: Some(TimelineCursor {
            status: ModelStatus {
                remain: vec![1; 23],
            },
            stream_cursor: 800,
            consumption: 800,
            tick: 100,
            timeleap_runtime: None,
        }),
    };
    let mut state = FieldState {
        target_model: "8".into(),
        ..FieldState::default()
    };
    let left = make_result(480, vec![100, 100], vec![600, 700, 800]);
    let mut right = make_result(491, vec![98, 102], vec![604, 696, 800]);
    right.target_cursor.as_mut().unwrap().status.remain[5] = 0;
    right.target_cursor.as_mut().unwrap().tick = 101;
    let normalized = state.normalize_search_results(vec![left, right]);
    assert_eq!(normalized.len(), 1);
}

#[test]
fn normalize_confirms_same_sfmt_series_and_uses_npc_number() {
    let make_result = |start_consumption: i64, observed_model: usize| FieldResult {
        start_consumption,
        first_wait_frame: 100,
        last_wait_frame: 200,
        first_consumption: 600,
        last_consumption: 700,
        intervals: vec![100],
        target_consumptions: vec![610, 710],
        next_blink_wait_frame: 100,
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
    };
    let mut state = FieldState {
        target_model: "0".into(),
        ..FieldState::default()
    };
    let normalized = state.normalize_search_results(vec![make_result(480, 1), make_result(485, 0)]);
    assert_eq!(normalized.len(), 1);
    assert_eq!(normalized[0].start_consumption, 485);
    assert_eq!(normalized[0].observed_model, 0);
}

#[test]
fn normalize_selects_the_matching_start_from_three_special_branches() {
    let make_result = |start_consumption: i64, observed_model: usize| FieldResult {
        start_consumption,
        first_wait_frame: 100,
        last_wait_frame: 300,
        first_consumption: 1_000,
        last_consumption: 1_200,
        intervals: vec![100, 100],
        target_consumptions: vec![1_000, 1_100, 1_200],
        next_blink_wait_frame: 100,
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
    };
    let mut state = FieldState {
        target_model: "6".into(),
        ..FieldState::default()
    };
    let normalized = state.normalize_search_results(vec![
        make_result(480, 2),
        make_result(482, 6),
        make_result(490, 8),
    ]);
    assert_eq!(normalized.len(), 1);
    assert_eq!(normalized[0].start_consumption, 482);
    assert_eq!(normalized[0].observed_model, 6);
}

#[test]
fn normalize_marks_npc_mismatch_and_keeps_representative_start() {
    let result = FieldResult {
        start_consumption: 500,
        first_wait_frame: 100,
        last_wait_frame: 200,
        first_consumption: 600,
        last_consumption: 700,
        intervals: vec![100],
        target_consumptions: vec![610, 710],
        next_blink_wait_frame: 100,
        target_wait_frame: None,
        target_exact: None,
        target_frame_before: None,
        target_frame_after: None,
        observed_model: 5,
        models: 13,
        timeline: Vec::new(),
        candidate_blinks: vec![CandidateBlink {
            frame: 710,
            timeline_frame: 200,
            observed: true,
            target_consumption: Some(710),
        }],
        target_blink_count: None,
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: None,
    };
    let mut state = FieldState {
        target_model: "9".into(),
        ..FieldState::default()
    };
    let normalized = state.normalize_search_results(vec![result]);
    assert_eq!(normalized.len(), 1);
    assert_eq!(normalized[0].start_consumption, 500);
    assert!(normalized[0].candidate_blinks.is_empty());
    assert_eq!(normalized[0].target_consumptions, vec![610, 710]);
}
