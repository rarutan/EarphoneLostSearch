use super::*;

fn ready_direct_encounter_app(seed: u32) -> (UiApp, egui::Context) {
    use crate::domain::timeleap::{TimeleapCandidate, TimeleapFrame1Preparation, TimeleapStatus};
    let mut app = app_with_empty_field_timeline();
    app.beep_enabled = false;
    app.field_timer_retry = false;
    app.field.timeleap_enabled = true;
    app.field.timeleap_direct_encounter = true;
    app.field.seed = format!("{seed:08X}");
    app.field.target_consumption = "1200".into();
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=2_000,
        2_000,
        app.state.fps_value(),
    )
    .unwrap();
    app.field.set_observation_cache(cache);
    let status = TimeleapStatus {
        model: crate::domain::rng::ModelStatus::new(1),
        phase: 0,
        fidget_cd: -1,
        is_boy: true,
        raining: false,
    };
    let candidate = TimeleapCandidate {
        frame1: 480,
        frame1_tick: 240,
        post_tick: 250,
        grace_frames: 0,
        pre_frame: 480,
        pre_sfmt_position: 480,
        pre_status: status.clone(),
        post_frame: 500,
        post_sfmt_position: 500,
        post_status: status,
        target_inputs: Vec::new().into(),
    };
    app.field.select_timeleap_frame1_timer();
    app.field.apply_timeleap_frame1_candidates(
        TimeleapFrame1Preparation {
            nominal_frame1: 480,
            nominal_tick: 240,
            candidates: vec![candidate],
        },
        app.state.fps_value(),
    );
    (app, egui::Context::default())
}

#[test]
fn ready_precomputes_target_and_starts_it_at_frame1_transition() {
    let (mut app, context) = ready_direct_encounter_app(0x53A0_0001);
    let (_, mut reference) = app.field.direct_encounter_target_snapshot().unwrap();
    let expected_result = reference
        .retarget_confirmed_result_from_cache(1200, None)
        .unwrap();
    assert!(!app.field.timeleap_timeline_adopted());
    app.start_prepared_timeleap_target(&context);
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.jobs.field_timeleap_prepared_target.is_running() && Instant::now() < deadline {
        app.poll_prepared_timeleap_target(&context);
        std::thread::yield_now();
    }
    app.poll_prepared_timeleap_target(&context);
    assert!(
        app.jobs.prepared_timeleap_target.is_some(),
        "Ready must prepare a Target result"
    );
    let actual_result = &app.jobs.prepared_timeleap_target.as_ref().unwrap().result;
    assert_eq!(
        actual_result.target_wait_frame,
        expected_result.target_wait_frame
    );
    assert_eq!(actual_result.target_exact, expected_result.target_exact);
    assert_eq!(
        actual_result.target_frame_before,
        expected_result.target_frame_before
    );
    assert_eq!(
        actual_result.target_frame_after,
        expected_result.target_frame_after
    );
    assert_eq!(
        actual_result.target_blink_count,
        expected_result.target_blink_count
    );
    let prepared_wait = app
        .jobs
        .prepared_timeleap_target
        .as_ref()
        .unwrap()
        .result
        .target_wait_frame
        .unwrap();
    let transition_at = Instant::now() + Duration::from_secs(60);
    assert!(app.field.tick_timeleap_frame1(transition_at));
    app.handle_timeleap_frame1_transition(&context, transition_at);
    assert_eq!(app.field.timer_kind(), FieldTimerKind::Target);
    assert!(app.field.timer_is_running());
    let expected = crate::domain::timing::game_frames_to_seconds(
        (prepared_wait + 20) as f64,
        app.state.fps_value(),
    );
    assert!((app.field.timer_seconds_value() - expected).abs() < 0.01);
    assert!(!app.field.target_research_pending);
    assert!(!app.field_timer_retry);
    assert!(app.jobs.prepared_timeleap_target.is_none());
}

#[test]
fn cancelling_prepared_target_during_cache_initialization_returns_promptly() {
    let (app, _) = ready_direct_encounter_app(0x53A0_0005);
    let (_, mut snapshot) = app.field.direct_encounter_target_snapshot().unwrap();
    let cancel = std::sync::atomic::AtomicBool::new(true);

    assert!(matches!(
        snapshot.retarget_confirmed_result_from_cache(1_200, Some(&cancel)),
        Err(crate::errors::AppError::Cancelled)
    ));
}

#[test]
fn unfinished_target_preparation_falls_back_without_switching_to_blink() {
    let (mut app, context) = ready_direct_encounter_app(0x53A0_0003);
    let transition_at = Instant::now() + Duration::from_secs(60);
    assert!(app.field.tick_timeleap_frame1(transition_at));
    app.handle_timeleap_frame1_transition(&context, transition_at);
    assert_eq!(app.field.timer_kind(), FieldTimerKind::Target);
    assert!(app.field.target_research_pending);
    app.field.normalize_timer_kind(app.state.fps_value());
    assert_eq!(app.field.timer_kind(), FieldTimerKind::Target);
    app.cancel_field_background_tasks();
}

#[test]
fn changed_candidate_rejects_prepared_target() {
    let (mut app, context) = ready_direct_encounter_app(0x53A0_0004);
    app.start_prepared_timeleap_target(&context);
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.jobs.field_timeleap_prepared_target.is_running() && Instant::now() < deadline {
        app.poll_prepared_timeleap_target(&context);
        std::thread::yield_now();
    }
    app.poll_prepared_timeleap_target(&context);
    app.jobs
        .prepared_timeleap_target
        .as_mut()
        .unwrap()
        .candidate
        .post_status
        .phase += 1;
    let transition_at = Instant::now() + Duration::from_secs(60);
    assert!(app.field.tick_timeleap_frame1(transition_at));
    app.handle_timeleap_frame1_transition(&context, transition_at);
    assert_eq!(app.field.timer_kind(), FieldTimerKind::Target);
    assert!(app.field.target_research_pending);
    app.cancel_field_background_tasks();
}

#[test]
fn changed_target_rejects_prepared_result_and_uses_fallback() {
    let (mut app, context) = ready_direct_encounter_app(0x53A0_0002);
    app.start_prepared_timeleap_target(&context);
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.jobs.field_timeleap_prepared_target.is_running() && Instant::now() < deadline {
        app.poll_prepared_timeleap_target(&context);
        std::thread::yield_now();
    }
    app.poll_prepared_timeleap_target(&context);
    assert!(app.jobs.prepared_timeleap_target.is_some());
    app.field.target_consumption = "1300".into();
    let transition_at = Instant::now() + Duration::from_secs(60);
    assert!(app.field.tick_timeleap_frame1(transition_at));
    app.handle_timeleap_frame1_transition(&context, transition_at);
    assert_eq!(app.field.timer_kind(), FieldTimerKind::Target);
    assert!(app.field.target_research_pending);
    assert!(app.field_timer_retry);
    app.cancel_field_background_tasks();
}

#[test]
fn direct_encounter_survives_field_render_and_starts_target_after_research() {
    let context = egui::Context::default();
    let mut app = app_with_empty_field_timeline();
    let fps = app.state.fps_value();
    app.field.timeleap_enabled = true;
    app.field.timeleap_direct_encounter = true;
    app.field.timeleap_stage = crate::application::field::TimeleapStage::Assumed;
    app.field
        .set_timeleap_timeline_anchor_for_test(Some(Instant::now()), Some(0));
    app.field.select_timeleap_frame1_timer();
    app.field.prepare_target_research(fps);

    let _ = context.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = field_ui(
                ui,
                &mut app.field,
                FieldUiWorkerState::default(),
                ctx,
                fps,
                false,
                false,
                ColorScheme::Standard,
                &Texts::new(Language::Japanese),
                false,
            );
        });
    });
    assert_eq!(app.field.timer_kind(), FieldTimerKind::Target);
    app.retry_field_timer_if_needed(&context);
    assert!(app.field_timer_retry);
    assert!(!app.field.timer_is_running());

    app.field.results[0].target_wait_frame = Some(600);
    app.field.results[0].target_exact = Some(true);
    app.field.target_research_pending = false;
    app.retry_field_timer_if_needed(&context);
    assert_eq!(app.field.timer_kind(), FieldTimerKind::Target);
    assert!(app.field.timer_is_running());
    assert!(!app.field_timer_retry);
    assert!(app.field.timer_remaining().unwrap() > 0.0);
}

#[test]
fn unavailable_direct_target_stays_selected_but_off_keeps_normal_fallback() {
    let context = egui::Context::default();
    for direct in [true, false] {
        let mut app = app_with_empty_field_timeline();
        app.field.timeleap_enabled = true;
        app.field.timeleap_direct_encounter = direct;
        app.field.timeleap_stage = crate::application::field::TimeleapStage::Assumed;
        app.field
            .set_timeleap_timeline_anchor_for_test(Some(Instant::now()), Some(0));
        app.field.select_timeleap_frame1_timer();
        app.field.normalize_timer_kind(app.state.fps_value());
        assert_eq!(
            app.field.timer_kind(),
            if direct {
                FieldTimerKind::Target
            } else {
                FieldTimerKind::NextBlink
            }
        );
        if direct {
            app.retry_field_timer_if_needed(&context);
            app.field.normalize_timer_kind(app.state.fps_value());
            assert_eq!(app.field.timer_kind(), FieldTimerKind::Target);
            assert!(!app.field.timer_is_running());
            assert!(!app.field_timer_retry);
        }
    }
}

#[test]
fn frame1_deadline_drains_last_due_beep_before_discarding_old_schedule() {
    let now = Instant::now();
    let mut app = UiApp {
        beep_enabled: false,
        field_timer_retry: true,
        ..UiApp::default()
    };
    app.beeps.field = vec![now, now + Duration::from_secs(1)];
    app.beeps.field_start = Some(now - Duration::from_secs(1));

    let played_due_count = app.finish_field_beep_segment_before_timeleap_transition(now);

    assert_eq!(played_due_count, 1);
    assert!(app.beeps.field.is_empty());
    assert_eq!(app.beeps.field_start, None);
    assert!(!app.field_timer_retry);
}

#[test]
fn blocked_field_extension_stops_timer_retry_and_reports_the_reason() {
    let context = egui::Context::default();
    let mut app = app_with_empty_field_timeline();
    app.jobs.field_timeline_extension_blocked = true;

    app.retry_field_timer_if_needed(&context);

    assert!(!app.field_timer_retry);
    assert!(app.field.next_blink_out_of_range);
    assert!(app.field.notice.is_none());

    let mut app = app_with_empty_field_timeline();
    app.jobs.field_timeline_extension_blocked = true;
    app.jobs.field_timeline_extension_error = Some(Arc::new(AppError::InvalidState(
        StateIssue::CalculationUnavailable,
    )));

    app.retry_field_timer_if_needed(&context);

    assert!(!app.field_timer_retry);
    assert!(app.field.notice.is_none());
}

#[test]
fn normal_timer_ui_tick_catches_up_after_fifty_million_frame_virtual_elapsed() {
    let mut app = UiApp::default();
    app.beep_enabled = false;
    let context = egui::Context::default();
    let start = Instant::now();
    let fps = app.state.fps_value();
    let first_segment_frames = 120;
    let total_frames = crate::application::state::MAX_FRAME + first_segment_frames;
    let virtual_now = start
        + Duration::from_secs_f64(
            crate::domain::timing::game_frames_to_seconds(total_frames as f64, fps) + 1.0,
        );
    let timer = &mut app.state.normal_timer;
    timer.timer_start = Some(start);
    timer.timer_seconds =
        crate::domain::timing::game_frames_to_seconds(first_segment_frames as f64, fps);
    timer.timer_segments = vec![
        (
            crate::application::timer_state::NormalTimerSegment::Offset(0),
            first_segment_frames,
        ),
        (
            crate::application::timer_state::NormalTimerSegment::Wait,
            crate::application::state::MAX_FRAME,
        ),
    ];
    timer.timer_segment_index = 0;
    timer.finished = false;

    app.tick_normal_timer_at(&context, virtual_now);

    assert!(!app.state.normal_timer.is_running());
    assert!(app.state.normal_timer.finished);
    assert_eq!(app.state.normal_timer.remaining_at(virtual_now), 0.0);
}
