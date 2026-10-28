use super::*;

#[test]
fn timeline_table_follows_current_position_by_default() {
    assert!(FieldState::default().table_follow_current);
}

#[test]
fn timeline_history_is_bounded_without_changing_rng_or_observation_state() {
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
    let mut result = search(&config).unwrap().remove(0);
    result.timeline = (0..(MAX_RETAINED_TIMELINE_ROWS + 44))
        .map(|index| FieldTimelineRow {
            frame: index as i64,
            timeline_frame: index as i64 * 2,
            duration_frames: 2,
        })
        .collect();
    result.target_blink_count = Some(500);
    result.target_blink_count_offset_from_first_row = 50;
    let before_trim = result.target_blinks_after_row(44);
    let cursor_before = result.timeline_cursor.clone();
    let target_cursor_before = result.target_cursor.clone();
    let intervals_before = result.intervals.clone();
    let observations_before = result.target_consumptions.clone();

    trim_timeline_history(&mut result);

    assert_eq!(result.timeline.len(), MAX_RETAINED_TIMELINE_ROWS);
    assert_eq!(result.timeline[0].frame, 44);
    assert_eq!(result.timeline.last().unwrap().frame, 299);
    assert_eq!(result.target_blink_count_offset_from_first_row, 6);
    assert_eq!(result.target_blinks_after_row(0), before_trim);
    assert_eq!(before_trim, Some(506));
    assert_eq!(result.timeline_cursor, cursor_before);
    assert_eq!(result.target_cursor, target_cursor_before);
    assert_eq!(result.intervals, intervals_before);
    assert_eq!(result.target_consumptions, observations_before);
}

#[test]
fn correction_mode_does_not_extend_or_schedule_the_timeline() {
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
    let mut state = FieldState {
        results: vec![result],
        observed: vec![Instant::now()],
        ..FieldState::default()
    };
    state.enter_correction_mode(crate::domain::timing::DEFAULT_FPS);

    assert!(!state.timeline_extension_needed(crate::domain::timing::DEFAULT_FPS));
    assert!(!state
        .extend_timeline_if_needed(crate::domain::timing::DEFAULT_FPS)
        .unwrap());
}

#[test]
fn timeline_extension_stops_when_projection_reaches_the_exact_prefetch_horizon() {
    let mut state = FieldState::empty_timeline_for_test();
    state.observed = vec![Instant::now() + Duration::from_secs(60)];
    let current_tick = state.results[0].last_wait_frame.div_euclid(2);
    state.timeline_projection_through_tick = current_tick.saturating_add(TIMELINE_CHUNK_TICKS * 2);

    assert!(!state.timeline_extension_needed(crate::domain::timing::DEFAULT_FPS));
    state.timeline_projection_through_tick -= 1;
    assert!(state.timeline_extension_needed(crate::domain::timing::DEFAULT_FPS));
}

#[test]
fn frame1_outside_visible_rows_is_prepared_async_and_counted_from_cached_blinks() {
    const SEED: u32 = 0xD3E1_7A52;
    let fps = crate::domain::timing::DEFAULT_FPS;
    let frame1 = 100_000_i64;
    let mut state = FieldState::empty_timeline_for_test();
    let cursor = TimelineCursor {
        status: ModelStatus::new(1),
        stream_cursor: 0,
        consumption: 0,
        tick: 0,
        timeleap_runtime: None,
    };
    state.results[0].timeline_cursor = Some(cursor.clone());
    state.results[0].target_cursor = Some(cursor);
    state.seed = format!("{SEED:08X}");
    state.observed = vec![Instant::now() + Duration::from_secs(60)];
    state.timeleap_enabled = true;
    state.timer_kind = FieldTimerKind::NextBlink;
    state.timeleap_frame1_input = frame1.to_string();
    state.timeleap_frame1_reachable = Some(true);
    let identity = FieldState::resolved_timeline_identity(SEED, &state.results[0]);
    state.resolved_timeline_cache = Some(Arc::new(Mutex::new(ResolvedTimelineCache::new(
        identity,
        state.results[0]
            .timeline_cursor
            .as_ref()
            .expect("test cursor")
            .clone(),
        Sfmt::new(SEED),
        0,
    ))));

    assert_eq!(
        state.timeleap_lifecycle_policy().table_mode,
        TimeleapTableMode::Frame1
    );
    assert!(state.timeline_extension_needed(fps));

    let mut extensions = 0;
    while state.timeleap_frame1_table_boundary(frame1).is_none() {
        assert!(state.timeline_extension_needed(fps));
        assert!(state.extend_timeline_if_needed(fps).unwrap());
        extensions += 1;
        assert!(
            extensions < 64,
            "Frame1 extension should make bounded progress"
        );
    }

    let result = &state.results[0];
    assert!(result.frame_boundary_index(frame1).is_none());
    let first_visible_frame = result.timeline.first().unwrap().frame;
    let expected_boundary = state
        .resolved_timeline_cache
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .blinks
        .iter()
        .filter(|event| event.consumption >= first_visible_frame && event.consumption <= frame1)
        .count();
    assert_eq!(
        state.timeleap_frame1_table_boundary(frame1),
        Some(expected_boundary)
    );
    assert!(result
        .blinks_through_frame_after_row_at_boundary(0, frame1, expected_boundary)
        .is_some());
    assert!(!state.timeline_extension_needed(fps));
    assert!(!state.timeline_extension_needed(fps));
}

#[test]
fn session_phase_locks_timer_after_entering_correction() {
    let mut state = FieldState::default();
    assert_eq!(state.session_phase(), SessionPhase::Input);
    assert!(state.can_edit_observation_settings());
    state.mode = FieldMode::Idle;
    state.results.push(FieldResult {
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
        models: 1,
        timeline: Vec::new(),
        candidate_blinks: Vec::new(),
        target_blink_count: None,
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: None,
    });
    assert_eq!(state.session_phase(), SessionPhase::TimelineReady);
    assert!(!state.can_edit_observation_settings());
    state.enter_correction_mode(crate::domain::timing::DEFAULT_FPS);
    assert_eq!(state.session_phase(), SessionPhase::Correction);
    assert!(!state.can_edit_general_settings());
    assert!(state.can_edit_correction_inputs());
    assert!(state
        .start_blink_timer(crate::domain::timing::DEFAULT_FPS)
        .is_err());
}

#[test]
fn target_passed_timeleap_correction_freezes_target_timer_at_zero() {
    let fps = crate::domain::timing::DEFAULT_FPS;
    let anchor = Instant::now() - Duration::from_secs(2);
    let mut state = FieldState {
        timeleap_enabled: true,
        timeleap_timeline_anchor: Some(anchor),
        timer_kind: FieldTimerKind::Target,
        timer_start: Some(anchor),
        timer_seconds: 42.5,
        results: vec![FieldResult {
            start_consumption: 0,
            first_wait_frame: 0,
            last_wait_frame: 0,
            first_consumption: 0,
            last_consumption: 0,
            intervals: Vec::new(),
            target_consumptions: Vec::new(),
            next_blink_wait_frame: 0,
            target_wait_frame: Some(0),
            target_exact: Some(true),
            target_frame_before: Some(0),
            target_frame_after: Some(0),
            observed_model: 0,
            models: 1,
            timeline: Vec::new(),
            candidate_blinks: Vec::new(),
            target_blink_count: None,
            target_blink_count_offset_from_first_row: 0,
            timeline_cursor: None,
            target_cursor: None,
        }],
        ..FieldState::default()
    };

    assert!(state.target_is_passed(fps));
    assert_eq!(state.preview_remaining(FieldTimerKind::Target, fps), None);
    state
        .begin_timeleap_recheck_at(true, fps, Instant::now())
        .expect("Target-passed Timeleap route enters correction mode");

    assert_eq!(state.timer_seconds_value(), 0.0);
    assert!(!state.timer_is_running());
    assert!(!state.timer_is_finished());
}

#[test]
fn correction_freezes_next_blink_remaining_value() {
    let fps = crate::domain::timing::DEFAULT_FPS;
    let timer_start = Instant::now() - Duration::from_secs(2);
    let mut state = FieldState {
        timeleap_enabled: true,
        timeleap_timeline_anchor: Some(timer_start),
        timer_kind: FieldTimerKind::NextBlink,
        timer_start: Some(timer_start),
        timer_seconds: 10.0,
        ..FieldState::default()
    };
    let expected_remaining = state
        .preview_remaining(FieldTimerKind::NextBlink, fps)
        .expect("running blink timer has a remaining value");

    state.enter_correction_mode(fps);

    assert!((state.timer_seconds_value() - expected_remaining).abs() < 0.001);
    assert!(!state.timer_is_running());
}

#[test]
fn npc_count_adds_the_implicit_player_model() {
    let state = FieldState {
        npc_count: "0".into(),
        ..FieldState::default()
    };
    assert_eq!(state.config().unwrap().models, 1);
}

#[test]
fn npc_number_is_required_even_for_the_player() {
    let state = FieldState {
        target_model: String::new(),
        ..FieldState::default()
    };
    assert!(state.configs().is_err());
}

#[test]
fn multiple_npc_search_expands_the_requested_range() {
    let state = FieldState {
        npc_count: "10~20".into(),
        multiple_npc_search: true,
        ..FieldState::default()
    };
    let configs = state.configs().unwrap();
    assert_eq!(configs.len(), 11);
    assert_eq!(configs.first().unwrap().models, 11);
    assert_eq!(configs.last().unwrap().models, 21);
}

#[test]
fn multiple_npc_search_rejects_ranges_wider_than_twenty_npc() {
    let state = FieldState {
        npc_count: "10~31".into(),
        multiple_npc_search: true,
        ..FieldState::default()
    };
    assert!(state.configs().is_err());
}

#[test]
fn multiple_npc_search_allows_a_width_of_twenty() {
    let state = FieldState {
        npc_count: "10~30".into(),
        multiple_npc_search: true,
        ..FieldState::default()
    };
    assert_eq!(state.configs().unwrap().len(), 21);
}

#[test]
fn npc_count_is_limited_to_zero_through_fifty() {
    let mut state = FieldState {
        npc_count: MAX_NPC_COUNT.to_string(),
        ..FieldState::default()
    };
    assert!(state.configs().is_ok());
    state.npc_count = (MAX_NPC_COUNT + 1).to_string();
    assert!(state.configs().is_err());
}

#[test]
fn search_rejects_ranges_wider_than_one_hundred_thousand_frames() {
    let state = FieldState {
        range_start: "9000000".into(),
        range_end: "9200000".into(),
        ..FieldState::default()
    };
    assert!(state.config().is_err());
}

#[test]
fn range_end_can_be_set_from_start_plus_width() {
    let mut state = FieldState {
        range_start: "9000000".into(),
        range_end: "0".into(),
        range_after_start: "10000".into(),
        ..FieldState::default()
    };
    state.set_range_from_start();
    assert_eq!(state.range_end, "9010000");
}

#[test]
fn unique_timeline_extends_from_the_previous_remain_state() {
    let config = FieldConfig {
        seed: 1,
        range_start: 0,
        range_end: 10,
        models: 1,
        target_model: 0,
        observed_intervals: vec![1],
        tolerance: 1_000,
        target_consumption: Some(100_000),
    };
    let mut results = search(&config).unwrap();
    let mut state = FieldState {
        range_start: "0".into(),
        range_end: "10".into(),
        npc_count: "0".into(),
        target_model: "0".into(),
        target_consumption: "100000".into(),
        intervals: vec![1],
        observed: vec![Instant::now() - std::time::Duration::from_secs(170)],
        results: vec![results.remove(0)],
        ..FieldState::default()
    };
    state.set_observation_cache(
        crate::domain::rng::generate_observation_cache(
            1,
            0..=10,
            100_000,
            crate::domain::timing::DEFAULT_FPS,
        )
        .unwrap(),
    );
    let before = state.results[0].timeline.len();
    assert!(state
        .extend_timeline_if_needed(crate::domain::timing::DEFAULT_FPS)
        .unwrap());
    assert!(state.results[0].timeline.len() > before);
    assert!(state.results[0].timeline.last().unwrap().timeline_frame > 10_000);
}

#[test]
fn empty_timeline_chunks_continue_until_the_target_model_can_blink() {
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
    let mut result = search(&config).unwrap().remove(0);
    // Keep the observed model dormant through two complete chunks. Other state
    // remains valid, but neither chunk can add a visible blink row.
    let anchor = result.target_cursor.as_mut().unwrap();
    anchor.status.remain[0] = TIMELINE_CHUNK_TICKS as i32 * 3 + 10;
    let anchor = anchor.clone();
    let original_tick = anchor.tick;
    let original_timeline = vec![FieldTimelineRow {
        frame: anchor.consumption,
        timeline_frame: anchor.tick * 2,
        duration_frames: 0,
    }];
    result.timeline = original_timeline.clone();
    result.timeline_cursor = Some(anchor);
    let mut state = FieldState {
        seed: "00000001".into(),
        range_start: "0".into(),
        range_end: "10".into(),
        npc_count: "0".into(),
        target_model: "0".into(),
        intervals: vec![1],
        observed: vec![Instant::now() - std::time::Duration::from_secs(170)],
        results: vec![result],
        ..FieldState::default()
    };
    state.set_observation_cache(
        crate::domain::rng::generate_observation_cache(
            1,
            0..=10,
            100_000,
            crate::domain::timing::DEFAULT_FPS,
        )
        .unwrap(),
    );

    for chunk in 1..=2 {
        assert!(state.timeline_extension_needed(crate::domain::timing::DEFAULT_FPS));
        let shared_before = state.resolved_timeline_cache.clone();
        assert!(state
            .extend_timeline_if_needed(crate::domain::timing::DEFAULT_FPS)
            .unwrap());
        let shared_after = state
            .resolved_timeline_cache
            .as_ref()
            .expect("extension installs its shared cache");
        if let Some(shared_before) = shared_before {
            assert!(Arc::ptr_eq(&shared_before, shared_after));
        }
        assert_eq!(state.results[0].timeline, original_timeline);
        assert_eq!(
            state.results[0].timeline_cursor.as_ref().unwrap().tick,
            original_tick + TIMELINE_CHUNK_TICKS * chunk
        );
    }
}

#[test]
fn per_tick_work_and_remain_stay_bounded_across_seed_and_model_extremes() {
    let seeds = [0, 1, u32::MAX, 0x367C_4DFE];
    let model_counts = [1, 22, MAX_NPC_COUNT + 1];
    for seed in seeds {
        for models in model_counts {
            let mut sfmt = Sfmt::new(seed);
            let mut status = ModelStatus::new(models);
            let mut total_consumed = 0_usize;
            for _ in 0..20_000 {
                let (used, consumed) = status.next_state_with_positions(|| {
                    let position = total_consumed as i64;
                    let value = sfmt.next_u64();
                    total_consumed += 1;
                    (position, value)
                });
                assert!(used <= models);
                assert_eq!(used, consumed.len());
                assert!(consumed.iter().all(|event| event.model < models));
                assert!(status
                    .remain
                    .iter()
                    .all(|remain| (-5..=36).contains(remain)));
            }
            assert!(total_consumed <= models * 20_000);
        }
    }
}

#[test]
#[ignore = "50,000,000FまでGUIのTimeline延長要求を連続再現する長時間テスト"]
fn field_timeline_worker_requests_stay_bounded_through_max_frame() {
    let seed = 0x367C4DFE;
    let models = MAX_NPC_COUNT + 1;
    let target_model = models - 1;
    let start = 486_i64;
    let target = crate::domain::rng::MAX_SFMT_FRAME;
    let fps = crate::domain::timing::DEFAULT_FPS;
    let mut sfmt = Sfmt::new(seed);
    let mut stream_cursor = 0_i64;
    while stream_cursor < start {
        sfmt.next_u64();
        stream_cursor += 1;
    }
    let mut status = ModelStatus::new(models);
    let mut consumption = start;
    let mut tick = 0_i64;
    let mut observed_events = Vec::new();
    while observed_events.len() < 4 {
        let (used, consumed) = status.next_state_with_positions(|| {
            let position = stream_cursor;
            let value = sfmt.next_u64();
            stream_cursor += 1;
            (position, value)
        });
        consumption += used as i64;
        tick += 1;
        if consumed
            .iter()
            .any(|event| event.model == target_model && event.blink)
        {
            observed_events.push(BlinkEvent {
                wait_frame: tick * 2,
                consumption,
                status: status.clone(),
                stream_cursor,
                model_consumption: consumed
                    .iter()
                    .find(|event| event.model == target_model && event.blink)
                    .map(|event| event.position),
            });
        }
    }

    let values = SearchValues::new(0, generate_u64_values_from(seed, 0, 200_000));
    let (timeline, candidate_blinks, timeline_cursor) =
        build_timeline(&observed_events, target_model, None, &values);
    let first = &observed_events[0];
    let last = observed_events.last().unwrap();
    let intervals = observed_events
        .windows(2)
        .map(|pair| pair[1].wait_frame - pair[0].wait_frame)
        .collect::<Vec<_>>();
    let result = FieldResult {
        start_consumption: start,
        first_wait_frame: first.wait_frame,
        last_wait_frame: last.wait_frame,
        first_consumption: first.consumption,
        last_consumption: last.consumption,
        intervals,
        target_consumptions: observed_events
            .iter()
            .filter_map(|event| event.model_consumption)
            .collect(),
        next_blink_wait_frame: timeline
            .iter()
            .find(|row| row.timeline_frame > last.wait_frame)
            .map(|row| row.timeline_frame - last.wait_frame)
            .unwrap_or_default(),
        target_wait_frame: None,
        target_exact: None,
        target_frame_before: None,
        target_frame_after: None,
        observed_model: target_model,
        models,
        timeline,
        candidate_blinks,
        target_blink_count: None,
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: Some(timeline_cursor),
        target_cursor: Some(TimelineCursor {
            status: last.status.clone(),
            stream_cursor: last.stream_cursor,
            consumption: last.consumption,
            tick: last.wait_frame / 2,
            timeleap_runtime: None,
        }),
    };
    let cache = crate::domain::rng::generate_observation_cache(seed, 0..=200_000, 0, fps).unwrap();
    assert!(cache.available_end() < target);
    let direct_reference =
        apply_target_from_cursor(result.clone(), seed, target, Some(&cache), None).unwrap();
    let mut state = FieldState {
        seed: format!("{seed:08X}"),
        range_start: start.to_string(),
        range_end: (start + 10_000).to_string(),
        npc_count: (models - 1).to_string(),
        target_model: target_model.to_string(),
        target_consumption: target.to_string(),
        intervals: result.intervals.clone(),
        results: vec![result],
        observed: vec![Instant::now()],
        ..FieldState::default()
    };
    state.set_observation_cache(cache);
    state.prepare_shared_timeline_cache().unwrap();
    let target_started = Instant::now();
    let target_snapshot = state.clone();
    let target_worker = std::thread::spawn(move || {
        let mut snapshot = target_snapshot;
        snapshot.retarget_confirmed_result_from_cache(target, None)
    });
    let mut ui_snapshot = state.clone();
    let mut ui_updates_during_workers = 0_usize;
    let mut last_ui_update = Instant::now();
    let mut next_ui_update = last_ui_update + std::time::Duration::from_secs_f64(1.0 / 30.0);
    let mut max_ui_update_gap = std::time::Duration::ZERO;
    let mut max_ui_read_elapsed = std::time::Duration::ZERO;
    while !target_worker.is_finished() {
        let now = Instant::now();
        if now >= next_ui_update {
            let ui_read_started = Instant::now();
            let _ = ui_snapshot.current_sfmt_frame(fps);
            let ui_read_elapsed = ui_read_started.elapsed();
            max_ui_read_elapsed = max_ui_read_elapsed.max(ui_read_elapsed);
            max_ui_update_gap = max_ui_update_gap
                .max(now - last_ui_update)
                .max(ui_read_elapsed);
            last_ui_update = Instant::now();
            next_ui_update = last_ui_update + std::time::Duration::from_secs_f64(1.0 / 30.0);
            ui_updates_during_workers += 1;
        } else {
            std::thread::sleep((next_ui_update - now).min(std::time::Duration::from_millis(1)));
        }
    }
    let result = target_worker.join().unwrap().unwrap();
    let target_search_elapsed = target_started.elapsed();
    assert!(result.target_wait_frame.is_some());
    assert_eq!(result.target_wait_frame, direct_reference.target_wait_frame);
    assert_eq!(result.target_exact, direct_reference.target_exact);
    assert_eq!(
        result.target_frame_before,
        direct_reference.target_frame_before
    );
    assert_eq!(
        result.target_frame_after,
        direct_reference.target_frame_after
    );
    assert_eq!(
        result.target_blink_count,
        direct_reference.target_blink_count
    );
    let expected_target_blinks = result.target_blink_count.unwrap();
    let target_timeline_frame = result.last_wait_frame + result.target_wait_frame.unwrap();
    state.results[0] = result;

    let started = Instant::now();
    let mut request_durations = Vec::new();
    let mut request_count = 0_usize;
    loop {
        let projection_end = state.timeline_projection_through_tick;
        let cursor = state
            .resolved_timeline_cache
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .cursor
            .clone();
        if projection_end >= cursor.tick {
            break;
        }
        let current_frame = projection_end
            .saturating_mul(2)
            .saturating_sub(state.results[0].last_wait_frame)
            .saturating_add(2)
            .max(0);
        let elapsed = crate::domain::timing::game_frames_to_seconds(current_frame as f64, fps);
        state.observed = vec![Instant::now() - std::time::Duration::from_secs_f64(elapsed)];
        assert!(state.timeline_extension_needed(fps));

        // App::start_field_timeline_extensionと同様に、専用requestをworkerへ渡し、
        // 完了後はTimeline差分だけを現在状態へ反映する。
        let request = state
            .timeline_extension_request()
            .expect("unique timeline can be extended");
        let request_started = Instant::now();
        let worker = std::thread::spawn(move || request.extend(fps, None));
        while !worker.is_finished() {
            std::thread::yield_now();
        }
        let extension = worker.join().unwrap().unwrap();
        request_durations.push(request_started.elapsed());
        let can_continue = extension.can_continue;
        assert_eq!(
            state.apply_timeline_extension(extension),
            Some(can_continue)
        );
        request_count += 1;

        let result = &state.results[0];
        assert!(result.timeline.len() <= MAX_RETAINED_TIMELINE_ROWS);
        assert_eq!(result.target_blink_count, Some(expected_target_blinks));
        assert!(result
            .timeline
            .windows(2)
            .all(|pair| { pair[0].timeline_frame < pair[1].timeline_frame }));
        if !can_continue {
            let cursor = state.results[0].timeline_cursor.as_ref().unwrap();
            assert!(
                cursor.consumption >= MAX_SFMT_FRAME || cursor.tick >= MAX_FUTURE_TIMELINE_TICKS
            );
            break;
        }
    }

    let final_result = &state.results[0];
    let final_cursor = final_result.timeline_cursor.as_ref().unwrap();
    let mut sorted_durations = request_durations.clone();
    sorted_durations.sort_unstable();
    let first_ten_average = request_durations
        .iter()
        .take(10)
        .map(std::time::Duration::as_secs_f64)
        .sum::<f64>()
        / request_durations.len().min(10) as f64;
    let last_ten_average = request_durations
        .iter()
        .rev()
        .take(10)
        .map(std::time::Duration::as_secs_f64)
        .sum::<f64>()
        / request_durations.len().min(10) as f64;
    println!(
            "50M Frame field UI-worker simulation: target_calc={:.2}s, chunks={request_count}, extension_elapsed={:.2}s, first10_avg={:.2}ms, last10_avg={:.2}ms, p95={:.2}ms, ui_updates_during_target={ui_updates_during_workers}, max_ui_gap={:.2}ms, max_ui_read={:.2}ms, cache_end={}, final_consumption={}, display_rows={}",
            target_search_elapsed.as_secs_f64(),
            started.elapsed().as_secs_f64(),
            first_ten_average * 1_000.0,
            last_ten_average * 1_000.0,
            sorted_durations[(sorted_durations.len() * 95 / 100).min(sorted_durations.len() - 1)]
                .as_secs_f64()
                * 1_000.0,
            max_ui_update_gap.as_secs_f64() * 1_000.0,
            max_ui_read_elapsed.as_secs_f64() * 1_000.0,
            state.observation_cache.as_ref().unwrap().available_end(),
            final_cursor.consumption,
            final_result.timeline.len(),
        );
    assert!(final_cursor.consumption >= target);
    assert!(ui_updates_during_workers > 0);
    assert!(max_ui_update_gap <= std::time::Duration::from_millis(50));
    assert!(target_timeline_frame > final_result.last_wait_frame);
    assert_eq!(
        final_result.target_blink_count,
        Some(expected_target_blinks)
    );
    assert!(final_result.timeline.len() <= MAX_RETAINED_TIMELINE_ROWS);
}

#[test]
fn prepared_timeline_index_keeps_the_same_interval_candidates() {
    let config = FieldConfig {
        seed: 1,
        range_start: 0,
        range_end: 100,
        models: 2,
        target_model: 0,
        observed_intervals: vec![1],
        tolerance: 1_000,
        target_consumption: None,
    };
    let baseline = search(&config).unwrap();
    let pool = prepare_search_pool_with_configs(std::slice::from_ref(&config)).unwrap();
    let prepared = search_with_preferred_idx_with_pool(&config, &pool)
        .unwrap()
        .results;
    assert_eq!(
        baseline
            .iter()
            .map(|result| result.start_consumption)
            .collect::<Vec<_>>(),
        prepared
            .iter()
            .map(|result| result.start_consumption)
            .collect::<Vec<_>>()
    );
}

#[test]
fn unknown_target_search_keeps_idx_candidates_but_skips_target_check() {
    let config = FieldConfig {
        seed: 1,
        range_start: 0,
        range_end: 100,
        models: 2,
        target_model: 0,
        observed_intervals: vec![1],
        tolerance: 1_000,
        target_consumption: Some(50),
    };
    let results = search_without_target_model(&config).unwrap();
    assert!(!results.is_empty());
    assert!(results.iter().any(|result| result.observed_model == 0));
    assert!(results.iter().any(|result| result.observed_model == 1));
    assert!(results
        .iter()
        .all(|result| result.target_wait_frame.is_none()));
}
