use super::*;

#[test]
fn encounter_offset_rejects_values_above_the_supported_limit() {
    let mut state = AppState::default();
    state.use_offset = true;
    state.encounter_offset = (MAX_ENCOUNTER_OFFSET_FRAMES + 1).to_string();
    assert!(state.encounter_offset_error().is_some());

    state.encounter_offset = MAX_ENCOUNTER_OFFSET_FRAMES.to_string();
    assert!(state.encounter_offset_error().is_none());

    state.encounter_offset.clear();
    assert!(state.encounter_offset_error().is_none());
}

#[test]
fn offset_correction_converts_strict_timeline_ticks_to_display_frames() {
    let mut state = AppState::default();
    state.range_start = "0".into();
    state.range_end = "1000".into();
    state.npc = "1".into();
    state.use_offset = true;
    state.encounter_offset = "10".into();
    state.fastest_close = true;
    let config = state.search_config().unwrap();
    install_test_cache(&mut state);
    let plan = encounter_plan_with_rotom(config.seed, 0, 0, true).unwrap();
    // 設定した10FオフセットはTimelineの5tickに相当する。Targetと実測Frameは
    // B+ロトムから続く同じ経路から求め、オフセット境界でModelStatusを初期化し直さない。
    let target = advance_timeline(config.seed, plan.encounter_start, config.npc_models, 5).unwrap();
    let actual = advance_timeline(config.seed, plan.encounter_start, config.npc_models, 7).unwrap();
    state.target = target.to_string();
    state.actual_target_frame = actual.to_string();
    assert_eq!(state.suggested_offset_for_actual_from(0).unwrap(), 6);
}

#[test]
fn offset_correction_uses_the_selected_circle_rotom_prediction() {
    let mut state = AppState::default();
    state.range_end = "1000".into();
    state.npc = "1".into();
    state.use_offset = true;
    state.encounter_offset = "10".into();
    let config = state.search_config().unwrap();
    install_test_cache(&mut state);
    let predicted = state.rotom_talk_at(0).unwrap();
    let predicted_plan = encounter_plan_with_observed_rotom(
        config.seed,
        0,
        0,
        true,
        state.rotom_threshold_value(),
        Some(predicted),
    )
    .unwrap();
    let target = advance_timeline(
        config.seed,
        predicted_plan.encounter_start,
        config.npc_models,
        5,
    )
    .unwrap();
    let actual = advance_timeline(
        config.seed,
        predicted_plan.encounter_start,
        config.npc_models,
        7,
    )
    .unwrap();
    state.target = target.to_string();
    state.observed_rotom_talk = Some(!predicted);
    state.actual_target_frame = actual.to_string();

    assert_eq!(state.suggested_offset_for_actual_from(0).unwrap(), 6);
}

#[test]
fn correction_adjusts_existing_offset_by_timeline_difference() {
    let mut state = AppState::default();
    state.seed = "B63523CA".into();
    state.target = "3000".into();
    state.npc = "5".into();
    state.use_offset = true;
    state.encounter_offset = "216".into();
    state.correction_source_offset = Some(216);
    state.actual_target_frame = "3003".into();
    install_test_cache(&mut state);
    // このseedでは○Frame 692に対し、Target=3000はtick508、実測Frame=3003は
    // tick509となり、表示Frameで2Fのずれになる。
    assert_eq!(state.suggested_offset_for_actual_from(692).unwrap(), 214);
    state.encounter_offset = "214".into();
    assert_eq!(state.suggested_offset_for_actual_from(692).unwrap(), 214);
}

#[test]
fn correction_offset_uses_target_actual_display_frame_drift() {
    let mut state = AppState::default();
    state.use_offset = true;
    state.encounter_offset = "100".into();
    state.fastest_close = true;
    // プレビューは実際に開始した○を使う。Targetを仮の開始位置にせず、実在する
    // 開始位置からTimelineを生成する。
    state.npc = "5".into();
    let source_plan = encounter_plan_with_rotom(1, 0, 0, true).unwrap();
    let source_target = advance_timeline(1, source_plan.encounter_start, 6, 5).unwrap();
    let source_actual = advance_timeline(1, source_plan.encounter_start, 6, 7).unwrap();
    state.target = source_target.to_string();
    state.actual_target_frame = source_actual.to_string();
    state.candidates = vec![TimelineCandidate {
        start_position: 0,
        sfmt_position: 0,
        next_blink_ticks: None,
        target_reachable: false,
    }];
    install_test_cache(&mut state);
    let preview = state.correction_preview_from(0).unwrap();
    assert_eq!(preview.suggested_offset, 100 - 4);
    assert_eq!(preview.timeline_start, source_plan.encounter_start);
    assert!(preview.target_on_timeline);
    assert!(preview.actual_on_timeline);
}

#[test]
fn correction_preview_reports_a_missing_seed_cache_as_calculation_unavailable() {
    let mut state = AppState::default();
    state.actual_target_frame = "1000".into();
    state.candidates = vec![TimelineCandidate {
        start_position: 0,
        sfmt_position: 0,
        next_blink_ticks: None,
        target_reachable: false,
    }];

    let error = state.correction_preview_from(0).unwrap_err();
    assert!(matches!(error, CorrectionPreviewError::CacheSeedMismatch));
}

#[test]
fn hatch_correction_and_production_share_consumption_order() {
    let mut state = AppState::default();
    state.seed = "00000001".into();
    state.range_end = "1000".into();
    state.npc = "3".into();
    state.consider_pre_rotom_consumption = true;
    state.pre_rotom_consumption = "5".into();
    state.consider_npc_initial_load = true;
    state.npc_initial_load = "4".into();
    state.use_offset = true;
    state.encounter_offset = "200".into();
    state.fastest_close = false;
    state.candidates = vec![TimelineCandidate {
        start_position: 0,
        sfmt_position: 0,
        next_blink_ticks: None,
        target_reachable: true,
    }];

    let config = state.search_config().unwrap();
    install_test_cache(&mut state);
    let expected = encounter_plan_with_pre_rotom_consumption(
        config.seed,
        0,
        0,
        true,
        state.rotom_threshold_value(),
        None,
        5,
        4,
    )
    .unwrap();
    let correction = state.correction_plan(&config, 0).unwrap();
    let production = state.article_plan_for_selected().unwrap();

    assert_eq!(correction.b_position, expected.b_position);
    assert_eq!(correction.rotom_consumption, expected.rotom_consumption);
    assert_eq!(correction.encounter_start, expected.encounter_start);
    assert_eq!(production.b_position, expected.b_position);
    assert_eq!(production.rotom_consumption, expected.rotom_consumption);
    assert_eq!(production.encounter_start, expected.encounter_start);
}

#[test]
fn correction_keeps_offset_when_actual_is_target_step_end() {
    let mut state = AppState::default();
    state.use_offset = true;
    state.encounter_offset = "100".into();
    state.npc = "5".into();
    let source_plan = encounter_plan_with_rotom(1, 0, 0, true).unwrap();
    let target = source_plan.encounter_start + 1;
    let actual = advance_timeline(1, source_plan.encounter_start, 6, 1).unwrap();
    state.target = target.to_string();
    state.actual_target_frame = actual.to_string();
    install_test_cache(&mut state);

    assert_eq!(state.suggested_offset_for_actual_from(0).unwrap(), 100);
}

#[test]
fn fastest_correction_can_compare_frame_before_offset_landing() {
    let mut state = AppState::default();
    state.fastest_close = true;
    state.use_offset = true;
    state.encounter_offset = "10".into();
    state.npc = "1".into();
    let config = state.search_config().unwrap();
    let plan = encounter_plan_with_rotom(config.seed, 0, 0, true).unwrap();
    let target = advance_timeline(config.seed, plan.encounter_start, config.npc_models, 5).unwrap();
    let actual = advance_timeline(config.seed, plan.encounter_start, config.npc_models, 4).unwrap();
    assert!(actual < target);
    state.target = target.to_string();
    state.actual_target_frame = actual.to_string();
    install_test_cache(&mut state);

    // オフセット着地後も同じModelStatus状態を継続する。新しい初期化境界ではなく、
    // 正負どちらの補正方向も有効である。
    assert!(state.suggested_offset_for_actual_from(0).is_ok());
}

#[test]
fn correction_still_returns_preview_when_direction_would_go_below_zero() {
    let mut state = AppState::default();
    state.npc = "1".into();
    state.use_offset = false;
    let config = state.search_config().unwrap();
    let plan = encounter_plan_with_rotom(config.seed, 0, 0, true).unwrap();
    let earlier =
        advance_timeline(config.seed, plan.encounter_start, config.npc_models, 1).unwrap();
    let later = advance_timeline(config.seed, plan.encounter_start, config.npc_models, 2).unwrap();
    install_test_cache(&mut state);

    // 通常待機で実測がTargetより後ろでも、負のオフセットを示すだけで補正操作は
    // 0Fの適用可能なプレビューを返す。
    state.fastest_close = false;
    state.target = earlier.to_string();
    state.actual_target_frame = later.to_string();
    assert_eq!(state.suggested_offset_for_actual_from(0).unwrap(), 0);

    // 最速閉じでは実測がTargetより前なら、その差分だけ待機を足す。
    state.fastest_close = true;
    state.target = later.to_string();
    state.actual_target_frame = earlier.to_string();
    assert_eq!(state.suggested_offset_for_actual_from(0).unwrap(), 2);
}

#[test]
fn fastest_close_reachability_uses_offset_as_timeline_time() {
    let mut state = AppState::default();
    state.npc = "1".into();
    state.use_offset = true;
    state.encounter_offset = "10".into();
    state.fastest_close = true;
    state.range_start = "0".into();
    state.range_end = "100".into();
    let config = state.search_config().unwrap();
    let plan = encounter_plan_with_rotom(config.seed, 0, 0, true).unwrap();
    let target = advance_timeline(config.seed, plan.encounter_start, config.npc_models, 5).unwrap();
    state.target = target.to_string();
    install_test_cache(&mut state);
    assert_eq!(state.table_target_reachable(0), Some(true));
}

#[test]
fn fastest_close_drift_entry_stops_blink_timer_without_offset() {
    let mut state = AppState::default();
    state.mode = Mode::Ready;
    state.fastest_close = true;
    state.actual_target_frame = "1000".into();
    state.candidates = vec![TimelineCandidate {
        start_position: 0,
        sfmt_position: 0,
        next_blink_ticks: Some(130),
        target_reachable: true,
    }];
    state.fast_close_circle_frames = vec![0];
    state.next_blink_index = Some(0);
    state.next_blink = Some(Instant::now() + Duration::from_secs(1));
    install_test_cache(&mut state);

    assert!(state.finish_fastest_close_for_correction());
    assert!(matches!(state.mode, Mode::Finished));
    assert!(state.next_blink.is_none());
    assert_eq!(state.correction_start_candidates(), vec![0]);
    assert_eq!(state.correction_target_frame, Some(1000));
    assert!(state.correction_preview_from(0).is_ok());

    // Enter後に補正関連の値を変更しても、停止した瞬き／本番タイマーを再開しない。
    state.encounter_offset = "2".into();
    state.recalculate_production();
    assert!(matches!(state.mode, Mode::Finished));
    assert!(state.timer_start.is_none());
    assert!(state.next_blink.is_none());
}

#[test]
fn fastest_close_uses_recorded_circles_when_entered_after_target_window() {
    let mut state = AppState::default();
    state.mode = Mode::Ready;
    state.fastest_close = true;
    state.candidates = vec![TimelineCandidate {
        start_position: 0,
        sfmt_position: 480,
        next_blink_ticks: Some(130),
        target_reachable: false,
    }];
    state.next_blink_index = Some(480);

    assert!(!state.can_finish_fastest_close_for_correction());
    assert!(!state.finish_fastest_close_for_correction());
    assert!(matches!(state.mode, Mode::Ready));
    assert!(state.correction_start_candidates().is_empty());

    state.fast_close_circle_frames = vec![478, 482];
    assert!(state.can_finish_fastest_close_for_correction());
    assert!(state.finish_fastest_close_for_correction());
    assert_eq!(state.correction_start_candidates(), vec![478, 482]);
    assert_eq!(state.correction_source_frame, Some(478));
    assert_eq!(state.next_blink_index, Some(478));
}

#[test]
fn session_phase_enters_correction_without_restarting_timers() {
    let mut state = AppState::default();
    assert_eq!(state.session_phase(), SessionPhase::Input);
    assert!(state.can_edit_observation_settings());
    state.mode = Mode::Ready;
    assert_eq!(state.session_phase(), SessionPhase::TimelineReady);
    assert!(!state.can_edit_observation_settings());
    state.enter_correction_mode();
    assert_eq!(state.session_phase(), SessionPhase::Correction);
    assert!(!state.can_edit_general_settings());
    assert!(state.can_edit_correction_inputs());
    assert!(state.timer_start.is_none());
}

#[test]
fn fastest_close_guidance_bounds_use_maximum_per_step_consumption() {
    let mut state = AppState::default();
    state.target = "3000".into();
    state.range_start = "0".into();
    state.range_end = "10000".into();
    state.npc = "5".into();
    state.use_offset = true;
    state.encounter_offset = "200".into();
    state.fastest_close = true;
    state.consider_rotom_talk = true;

    let config = state.search_config().unwrap();
    assert_eq!(
        state.fastest_close_guidance_bounds(&config),
        Some((2297, 3098, true))
    );
}

#[test]
fn fastest_close_bounds_treat_zero_as_the_lower_frame_limit() {
    let mut state = AppState::default();
    state.target = "100".into();
    state.range_start = "0".into();
    state.range_end = "5000".into();
    state.npc = "0".into();
    state.use_offset = true;
    state.encounter_offset = "122".into();
    state.fastest_close = true;

    let config = state.search_config().unwrap();
    let (_, _, fully_covered) = state.fastest_close_guidance_bounds(&config).unwrap();
    assert!(fully_covered);
}

#[test]
fn correction_uses_current_circle_for_additional_wait_even_if_target_is_unreachable() {
    let mut state = AppState::default();
    state.mode = Mode::Ready;
    state.fastest_close = false;
    state.range_end = "100".into();
    state.candidates = vec![TimelineCandidate {
        start_position: 0,
        sfmt_position: 0,
        next_blink_ticks: Some(130),
        target_reachable: false,
    }];
    let config = state.search_config().unwrap();
    let plan = encounter_plan_with_rotom(config.seed, 0, 0, true).unwrap();
    state.target = advance_timeline(config.seed, plan.encounter_start, config.npc_models, 3)
        .unwrap()
        .to_string();
    assert_eq!(state.correction_start_candidates(), vec![0]);
}

#[test]
fn correction_uses_all_observation_candidates_when_current_position_is_ambiguous() {
    let mut state = AppState::default();
    state.fast_close_circle_frames.clear();
    state.candidates = vec![
        TimelineCandidate {
            start_position: 0,
            sfmt_position: 692,
            next_blink_ticks: Some(130),
            target_reachable: false,
        },
        TimelineCandidate {
            start_position: 1,
            sfmt_position: 694,
            next_blink_ticks: Some(130),
            target_reachable: false,
        },
    ];

    assert_eq!(state.correction_start_candidates(), vec![692, 694]);
}

#[test]
fn correction_source_selection_keeps_all_fastest_close_starts() {
    let mut state = AppState::default();
    state.fastest_close = true;
    state.fast_close_circle_frames = vec![10, 12, 14];

    assert_eq!(state.correction_start_candidates(), vec![10, 12, 14]);
    assert_eq!(state.selected_correction_start(), Some(10));
    assert!(state.set_correction_source_index(2));
    assert_eq!(state.correction_source_index(), 2);
    assert_eq!(state.selected_correction_start(), Some(14));
}

#[test]
fn correction_source_selection_updates_rotom_prediction_without_touching_guidance() {
    let mut state = AppState::default();
    state.fastest_close = true;
    state.fast_close_circle_frames = vec![0, 1];
    state.guidance.record_reachability(123, true);
    install_test_cache(&mut state);

    assert!(state.set_correction_source_index(1));
    let predicted = state.predicted_rotom_talk_for_correction_source();
    assert!(predicted.is_some());
    assert!(state.sync_observed_rotom_to_correction_source());
    assert_eq!(state.observed_rotom_talk, predicted);
    assert!(state.guidance.contains_reachable(123));
}

#[test]
fn reported_fastest_close_case_uses_each_start_position() {
    let expected = [
        (808, 10, 813, 1028),
        (809, 47, 814, 1028),
        (810, 2, 815, 1028),
    ];
    for (start, expected_roll, expected_encounter_start, expected_after_offset) in expected {
        assert_eq!(rotom_roll_at(1, start), Some(expected_roll));
        let plan = encounter_plan_with_pre_rotom_consumption(1, start, 0, true, 70, None, 0, 3)
            .expect("reported case must produce an encounter plan");
        assert_eq!(plan.rotom_consumption, 2);
        assert_eq!(plan.encounter_start, expected_encounter_start);
        assert_eq!(
            advance_timeline(1, plan.encounter_start, 4, 61),
            Some(expected_after_offset)
        );
    }

    let guidance = AppState::calculate_guidance_batch(GuidanceSearchRequest {
        session_generation: 0,
        observed_start: 808,
        observed_end: 810,
        range_start: 808,
        range_end: 810,
        seed: 1,
        target: 1028,
        models: 4,
        offset: 122,
        consider_rotom: true,
        rotom_threshold: 70,
        observed_rotom_talk: None,
        pre_rotom_consumption: 0,
        npc_initial_load: 3,
        fastest_ticks: Some(61),
    });
    assert_eq!(guidance, vec![(808, true), (809, true), (810, true)]);
}

#[test]
fn fastest_close_without_recorded_circle_skips_rotom_correction() {
    let mut state = AppState::default();
    state.fastest_close = true;
    state.fast_close_circle_frames.clear();
    assert!(state.correction_start_candidates().is_empty());
    assert!(state.rotom_correction_from(0, 0).is_none());
}

#[test]
fn rotom_correction_uses_only_start_roll_and_observed_talk() {
    let mut state = AppState::default();
    state.fastest_close = true;
    state.fast_close_circle_frames = vec![0];
    install_test_cache(&mut state);
    let predicted = state.rotom_talk_at(0).unwrap();
    state.observed_rotom_talk = Some(!predicted);

    let correction = state
        .rotom_correction_from(0, MAX_FRAME)
        .expect("opposite observed talk must produce a threshold correction");
    let roll = u64::from(correction.roll);
    let expected = if correction.observed_talk {
        roll + 1
    } else {
        roll
    }
    .min(100);
    assert_eq!(correction.suggested_threshold, expected);
}

#[test]
fn correction_requires_actual_frame_and_valid_conditions() {
    let mut state = AppState::default();
    assert!(!state.correction_inputs_complete());
    state.actual_target_frame = "100".into();
    assert!(state.correction_inputs_complete());
    state.npc = "not-a-number".into();
    assert!(!state.correction_inputs_complete());
}

#[test]
fn additional_wait_enter_starts_offset_then_target_timer() {
    let mut state = AppState::default();
    state.mode = Mode::Ready;
    state.fastest_close = false;
    state.use_offset = true;
    state.encounter_offset = "60".into();
    state.range_start = "0".into();
    state.range_end = "100".into();
    state.candidates = vec![TimelineCandidate {
        start_position: 0,
        sfmt_position: 0,
        next_blink_ticks: Some(130),
        target_reachable: true,
    }];
    let config = state.search_config().unwrap();
    let plan = encounter_plan_with_rotom(config.seed, 0, 0, true).unwrap();
    let target = advance_timeline(config.seed, plan.encounter_start, config.npc_models, 5).unwrap();
    state.target = target.to_string();
    install_test_cache(&mut state);
    state.mark_production_preparation_pending();
    let (request, cache) = state
        .production_preparation_request()
        .expect("production request");
    let prepared = AppState::calculate_production_preparation(request.clone(), cache)
        .expect("prepared production timeline");
    assert!(state.apply_production_preparation(&request, prepared));
    let expected_tick =
        production_timeline(config.seed, plan.encounter_start, target, config.npc_models)
            .target_tick
            .unwrap();

    state.begin_encounter();

    assert!(matches!(state.mode, Mode::Countdown));
    assert!(matches!(
        state.article_timer_stage,
        Some(ArticleTimerStage::Offset)
    ));
    let offset_seconds = game_frames_to_seconds(60.0, DEFAULT_FPS);
    let target_seconds = blink_ticks_to_seconds(expected_tick, DEFAULT_FPS);
    assert!((state.article_timer_seconds - offset_seconds).abs() < f64::EPSILON);
    assert_eq!(state.article_next_seconds, Some(target_seconds));

    state.article_timer_start = Some(Instant::now() - Duration::from_secs(2));
    assert_eq!(state.tick_article_timer(), Some(ArticleTimerStage::Offset));
    assert_eq!(state.article_timer_stage, Some(ArticleTimerStage::ToTarget));
    assert!((state.article_timer_seconds - target_seconds).abs() < f64::EPSILON);
    assert_eq!(state.article_next_seconds, None);
}

#[test]
fn observed_rotom_talk_does_not_change_configured_threshold() {
    let mut state = AppState::default();
    install_test_cache(&mut state);
    state.candidates = vec![TimelineCandidate {
        start_position: 0,
        sfmt_position: 0,
        next_blink_ticks: None,
        target_reachable: false,
    }];
    let roll = rotom_roll_at(1, 0).unwrap();
    let observed = u64::from(roll) >= 79;
    let configured_prediction = u64::from(roll) < 79;

    state.set_observed_rotom_talk(observed);

    assert_eq!(state.rotom_threshold_value(), 79);
    assert_eq!(state.predicted_rotom_talk(), Some(configured_prediction));
}

#[test]
fn observed_rotom_talk_keeps_guidance_results() {
    let mut state = AppState::default();
    state.guidance.record_reachability(123, true);

    state.set_observed_rotom_talk(true);

    assert!(state.guidance.contains_reachable(123));
}
