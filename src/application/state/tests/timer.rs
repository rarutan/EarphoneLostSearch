use super::*;

#[test]
fn default_beep_settings_are_half_second_and_six_times() {
    let state = AppState::default();

    assert_eq!(state.beep_interval, "0.5");
    assert_eq!(state.beep_count, "6");
}

#[test]
fn hatch_timer_catches_up_after_fifty_million_frame_virtual_elapsed() {
    let mut state = AppState::default();
    let start = Instant::now();
    let duration_seconds = game_frames_to_seconds(MAX_FRAME as f64, state.fps_value());
    let virtual_now = start + Duration::from_secs_f64(duration_seconds);
    state.mode = Mode::Production;
    state.timer_start = Some(start);
    state.timer_seconds = duration_seconds;

    assert_eq!(state.remaining_at(virtual_now), 0.0);
    assert!(state.finish_production_if_elapsed(virtual_now));
    assert!(state.is_finished());
    assert_eq!(state.remaining_at(virtual_now), 0.0);
}

#[test]
fn hatch_article_timer_catches_up_both_stages_after_long_virtual_elapsed() {
    let mut state = AppState::default();
    let start = Instant::now();
    let long_wait = game_frames_to_seconds(MAX_FRAME as f64, state.fps_value());
    let virtual_now = start + Duration::from_secs_f64(long_wait);
    state.mode = Mode::Countdown;
    state.article_timer_stage = Some(ArticleTimerStage::Offset);
    state.article_timer_start = Some(start);
    state.article_timer_seconds = game_frames_to_seconds(120.0, state.fps_value());
    state.article_next_seconds = Some(30.0);

    assert_eq!(
        state.tick_article_timer_at(virtual_now),
        Some(ArticleTimerStage::Offset)
    );
    assert_eq!(state.article_timer_stage, Some(ArticleTimerStage::ToTarget));
    assert_eq!(
        state.tick_article_timer_at(virtual_now),
        Some(ArticleTimerStage::ToTarget)
    );
    assert!(state.is_finished());
    assert_eq!(state.article_timer_start_time(), None);
}

#[test]
fn normal_timer_sums_zero_or_more_offsets() {
    let mut timer = NormalTimerState::default();
    timer.wait_frames = "120".into();
    assert_eq!(timer.total_frames().unwrap(), 120);

    timer.offsets = vec!["5".into(), "2".into(), "10".into()];
    assert_eq!(timer.total_frames().unwrap(), 137);
}

#[test]
fn normal_timer_runs_offsets_top_to_bottom_then_actual_wait() {
    let mut timer = NormalTimerState::default();
    timer.wait_frames = "800".into();
    timer.offsets = vec!["200".into(), "100".into()];

    timer.start(100.0).unwrap();
    assert_eq!(timer.current_segment(), Some(NormalTimerSegment::Offset(0)));
    assert!((timer.timer_seconds - 2.0).abs() < f64::EPSILON);
    assert_eq!(timer.next_segment_seconds(100.0), Some(1.0));

    timer.timer_start = Some(Instant::now() - Duration::from_secs(3));
    assert!(timer.advance_segment(100.0));
    assert_eq!(timer.current_segment(), Some(NormalTimerSegment::Offset(1)));
    assert!((timer.timer_seconds - 1.0).abs() < f64::EPSILON);

    assert!(timer.advance_segment(100.0));
    assert_eq!(timer.current_segment(), Some(NormalTimerSegment::Wait));
    assert!((timer.timer_seconds - 8.0).abs() < f64::EPSILON);
    assert_eq!(timer.next_segment_seconds(100.0), None);
}

#[test]
fn normal_timer_ignores_disabled_offsets_but_keeps_their_values() {
    let mut timer = NormalTimerState::default();
    timer.wait_frames = "800".into();
    timer.offsets = vec!["200".into(), "100".into()];
    timer.offset_enabled = vec![true, false];

    assert_eq!(timer.total_frames().unwrap(), 1_000);
    assert_eq!(
        timer.segment_plan().unwrap(),
        vec![
            (NormalTimerSegment::Offset(0), 200),
            (NormalTimerSegment::Wait, 800)
        ]
    );
}

#[test]
fn normal_timer_supports_zero_offsets_and_rejects_negative_segments() {
    let mut timer = NormalTimerState::default();
    timer.wait_frames = "800".into();
    assert_eq!(
        timer.segment_plan().unwrap(),
        vec![(NormalTimerSegment::Wait, 800)]
    );

    timer.offsets = vec!["-2".into()];
    assert!(timer.start(DEFAULT_FPS).is_err());
}

#[test]
fn timeline_display_position_follows_predicted_blinks() {
    let mut state = AppState::default();
    state.candidates = vec![TimelineCandidate {
        start_position: 0,
        sfmt_position: 100,
        next_blink_ticks: Some(300),
        target_reachable: true,
    }];
    state.adjust = 2;
    state.next_blink_index = Some(100);
    assert_eq!(state.timeline_display_position(), Some(100));

    state.next_blink_index = Some(101);
    assert_eq!(state.timeline_display_position(), Some(101));
}

#[test]
fn multiple_candidates_do_not_start_or_display_a_blink_timer() {
    let mut state = AppState::default();
    state.observed.push(Instant::now());
    state.candidates = vec![
        TimelineCandidate {
            start_position: 0,
            sfmt_position: 100,
            next_blink_ticks: Some(300),
            target_reachable: true,
        },
        TimelineCandidate {
            start_position: 10,
            sfmt_position: 110,
            next_blink_ticks: Some(200),
            target_reachable: true,
        },
    ];

    state.schedule_blink();

    assert!(state.next_blink.is_none());
    assert!(state.next_blink_index.is_none());
    assert_eq!(state.timeline_display_position(), None);
}

#[test]
fn blink_timer_uses_observation_anchor_when_cache_is_unavailable() {
    let mut state = AppState::default();
    state.candidates = vec![TimelineCandidate {
        start_position: 0,
        sfmt_position: 100,
        next_blink_ticks: Some(300),
        target_reachable: true,
    }];
    let observed_at = Instant::now() - Duration::from_secs(1);
    state.observed.push(observed_at);

    state.schedule_blink();

    let deadline = state.next_blink.expect("blink deadline");
    let expected =
        300.0 * crate::domain::rng::GAME_FRAMES_PER_BLINK_TICK as f64 / state.fps_value();
    assert!((deadline.duration_since(observed_at).as_secs_f64() - expected).abs() < 0.001);
    assert!(deadline < Instant::now() + Duration::from_secs_f64(expected));
}

#[test]
fn blink_timer_connects_prediction_after_actual_observed_elapsed_time() {
    let mut state = AppState::default();
    let fps = state.fps_value();
    let cache = crate::domain::rng::generate_observation_cache(1, 0..=4, 4, fps).unwrap();
    let expected_seconds = cache.interval_seconds_at(2).unwrap().unwrap();
    let first_observed_at = Instant::now() - Duration::from_secs(3);
    let last_observed_at = first_observed_at + Duration::from_secs_f64(1.234);
    state.observation_cache = Some(cache);
    state.candidates = vec![TimelineCandidate {
        start_position: 0,
        sfmt_position: 2,
        next_blink_ticks: Some(130),
        target_reachable: true,
    }];
    state.observed.push(first_observed_at);
    state.observed.push(last_observed_at);
    state.schedule_blink();

    let deadline = state.next_blink.expect("blink deadline");
    assert!(
        (deadline.duration_since(last_observed_at).as_secs_f64() - expected_seconds).abs() < 0.001
    );
}

#[test]
fn predicted_blink_uses_the_next_random_value_after_each_beep() {
    let mut state = AppState::default();
    let fps = state.fps_value();
    let cache = crate::domain::rng::generate_observation_cache(1, 0..=4, 4, fps).unwrap();
    let expected_seconds = cache.interval_seconds_at(1).unwrap().unwrap();
    state.observation_cache = Some(cache);
    state.candidates = vec![TimelineCandidate {
        start_position: 0,
        sfmt_position: 0,
        next_blink_ticks: Some(130),
        target_reachable: true,
    }];
    let previous_deadline = Instant::now() + Duration::from_secs(10);
    state.next_blink = Some(previous_deadline);
    state.next_blink_index = Some(0);

    state.advance_predicted_blink();

    let next_deadline = state.next_blink.expect("next blink deadline");
    assert_eq!(state.next_blink_index, Some(1));
    assert!(
        (next_deadline
            .duration_since(previous_deadline)
            .as_secs_f64()
            - expected_seconds)
            .abs()
            < 0.001
    );
}

#[test]
fn predicted_blink_keeps_known_guidance_while_production_prepares() {
    let mut state = AppState::default();
    state.seed = "00000001".into();
    state.range_start = "0".into();
    state.range_end = "100".into();
    state.target = "80".into();
    state.npc = "1".into();
    state.observation_cache =
        crate::domain::rng::generate_observation_cache(1, 0..=200, 200, state.fps_value()).ok();
    state.candidates = vec![TimelineCandidate {
        start_position: 0,
        sfmt_position: 0,
        next_blink_ticks: Some(130),
        target_reachable: true,
    }];
    state.next_blink = Some(Instant::now() + Duration::from_secs(10));
    state.next_blink_index = Some(0);
    state.record_guidance_reachability(1, false);
    state.record_guidance_reachability(2, false);
    state.record_guidance_reachability(3, true);
    state.guidance.mark_computed_until(3);

    state.advance_predicted_blink();

    assert!(state.production_preparation_pending);
    assert_eq!(state.next_blink_index, Some(1));
    assert_eq!(state.guidance.reachability(1), Some(false));
    assert_eq!(state.guidance.reachability(3), Some(true));
    assert_eq!(
        state
            .hatch_guidance()
            .expect("guidance calculation")
            .and_then(|guidance| guidance.next_circle_frame),
        Some(3)
    );
}

#[test]
fn positive_timing_adjustment_makes_the_beep_earlier_without_changing_sfmt() {
    let mut state = AppState::default();
    state.fps = "50".into();
    state.candidates = vec![TimelineCandidate {
        start_position: 0,
        sfmt_position: 100,
        next_blink_ticks: Some(300),
        target_reachable: true,
    }];
    state.next_blink_index = Some(100);
    let before = Instant::now() + Duration::from_secs(10);
    state.next_blink = Some(before);

    state.adjust_by(2);

    assert_eq!(state.timeline_display_position(), Some(100));
    assert_eq!(state.selected_candidate_position(), Some(100));
    let shift = before
        .duration_since(state.next_blink.expect("adjusted deadline"))
        .as_secs_f64();
    assert!((shift - 2.0 / 50.0).abs() < 0.001);
}

#[test]
fn negative_timing_adjustment_makes_the_beep_later() {
    let mut state = AppState::default();
    state.fps = "50".into();
    let before = Instant::now() + Duration::from_secs(10);
    state.next_blink = Some(before);

    state.adjust_by(-2);

    let shift = state
        .next_blink
        .expect("adjusted deadline")
        .duration_since(before)
        .as_secs_f64();
    assert!((shift - 2.0 / 50.0).abs() < 0.001);
}

#[test]
fn cancelling_observation_resets_ready_state_for_retry() {
    let mut state = AppState::default();
    state.mode = Mode::Ready;
    state.match_tolerance = 15;
    state.candidates.push(TimelineCandidate {
        start_position: 0,
        sfmt_position: 100,
        next_blink_ticks: Some(300),
        target_reachable: true,
    });
    state.cancel_observation();
    assert!(matches!(state.mode, Mode::Idle));
    assert!(state.candidates.is_empty());
    assert_eq!(state.match_tolerance, state.tolerance_value());
}

#[test]
fn production_time_steps_use_configured_timeline_fps() {
    let mut state = AppState::default();
    state.fps = "59.8702".into();
    assert!((state.game_ticks_to_seconds(30) - 60.0 / 59.8702).abs() < f64::EPSILON);
}

#[test]
fn normal_timer_rejects_negative_total() {
    let mut timer = NormalTimerState::default();
    timer.wait_frames = "10".into();
    timer.offsets = vec!["-11".into()];
    assert!(timer.total_frames().is_err());
}
