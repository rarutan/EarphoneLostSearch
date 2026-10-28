use super::*;

fn sample_timeleap_candidate_group() -> crate::domain::timeleap::TimeleapCandidateGroup {
    let status = crate::domain::timeleap::TimeleapStatus {
        model: crate::domain::rng::ModelStatus::new(1),
        phase: 0,
        fidget_cd: -1,
        is_boy: false,
        raining: false,
    };
    crate::domain::timeleap::TimeleapCandidateGroup {
        frame1_start: 100,
        frame1_end: 100,
        frame1_tick_start: 50,
        frame1_tick_end: 50,
        representative_frame1: 100,
        frame2_ticks: 10,
        grace_frames: 0,
        target_frame_start: 120,
        target_frame_end: 120,
        target_tick_start: 60,
        target_tick_end: 60,
        representative_target_frame: 120,
        target_grace_frames: 0,
        candidates: smallvec::smallvec![crate::domain::timeleap::TimeleapCandidate {
            frame1: 100,
            frame1_tick: 50,
            post_tick: 60,
            grace_frames: 0,
            pre_frame: 100,
            pre_sfmt_position: 100,
            pre_status: status.clone(),
            post_frame: 120,
            post_sfmt_position: 120,
            post_status: status,
            target_inputs: vec![crate::domain::timeleap::TimeleapTargetInput {
                frame: 120,
                tick: 60,
            }]
            .into(),
        }],
    }
}

fn sample_timeleap_search_key() -> crate::domain::timeleap::TimeleapSearchKey {
    crate::domain::timeleap::TimeleapSearchKey {
        config: crate::domain::timeleap::TimeleapConfig {
            seed: 1,
            start_frame: 0,
            target_frame: 1_000,
            models: 1,
            min_frame1_elapsed_ticks: 0,
            max_frame1_elapsed_ticks: 1_000,
            mode: crate::domain::timeleap::TimeleapMode::Menu,
            is_boy: false,
            handoff_range_frames: 20,
            courier: crate::domain::timeleap::CourierConfig::default(),
        },
        consider_delay: None,
        grace_min: 0,
        target_grace_min: 0,
        frame1_filter: None,
        timeline_start: crate::domain::timeleap::TimeleapTimelineStart::initial(0, 1),
        current_tick: 0,
    }
}

#[test]
fn main_reset_cancels_running_frame1_search_and_keeps_candidate_rows() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let mut app = UiApp::default();
    let group = sample_timeleap_candidate_group();
    app.field.begin_timeleap_candidate_search();
    app.field.timeleap_candidate_groups.push(group.clone());

    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let key = sample_timeleap_search_key();
    assert!(app
        .jobs
        .field_timeleap_candidates
        .spawn_with_token(cancel.clone(), move || {
            while !worker_cancel.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            (
                key,
                Err(crate::domain::timeleap::TimeleapSearchFailure::Cancelled),
            )
        }));

    app.cancel_field_background_tasks();

    assert!(cancel.load(Ordering::Acquire));
    assert!(!app.jobs.field_timeleap_candidates.is_running());
    assert!(!app.field.timeleap_candidate_search_started);
    assert_eq!(app.field.timeleap_candidate_groups, vec![group]);
}

#[test]
fn main_reset_keeps_completed_frame1_search_results() {
    let mut app = UiApp::default();
    let group = sample_timeleap_candidate_group();
    app.field
        .apply_timeleap_candidate_groups(vec![group.clone()]);

    app.cancel_field_background_tasks();

    assert_eq!(app.field.timeleap_candidate_groups, vec![group]);
    assert!(!app.field.timeleap_search_completed_without_candidates());
}

#[test]
fn timeleap_runtime_state_is_not_saved_or_resumed() {
    let mut app = UiApp::default();
    app.field.timeleap_enabled = true;
    app.field.timeleap_stage = crate::application::field::TimeleapStage::Assumed;
    app.field.timeleap_shift_times.push(Instant::now());
    app.field.confirmed_target_search_form.status =
        crate::application::field::ConfirmedTargetSearchStatus::NoResults;

    let saved = app.persisted_state();
    let json = serde_json::to_value(&saved).expect("persisted state serializes");
    let field = json["field"].as_object().expect("field is an object");
    assert!(field.contains_key("timeleap"));
    assert!(!field.contains_key("timeleap_stage"));
    assert!(!field.contains_key("timeleap_shift_times"));
    assert!(!field.contains_key("confirmed_target_search_form"));

    let mut restored = UiApp::default();
    restored.apply_persisted_state(saved);
    assert_eq!(
        restored.field.timeleap_stage,
        crate::application::field::TimeleapStage::Off
    );
    assert!(restored.field.timeleap_shift_times.is_empty());
    assert!(matches!(
        restored.field.confirmed_target_search_form.status,
        crate::application::field::ConfirmedTargetSearchStatus::Idle
    ));
}

#[test]
fn hatch_production_recalculation_runs_through_the_background_worker() {
    let mut app = UiApp::default();
    app.state.seed = format!("{TEST_SEED:08X}");
    app.state.range_start = "0".into();
    app.state.range_end = "1000".into();
    app.state.target = "5000".into();
    app.state.npc = "5".into();
    app.state.candidates = vec![crate::application::state::TimelineCandidate {
        start_position: 0,
        sfmt_position: 1000,
        next_blink_ticks: Some(130),
        target_reachable: true,
    }];
    app.state.observation_cache = crate::domain::rng::generate_observation_cache(
        TEST_SEED,
        0..=100_000,
        100_000,
        app.state.fps_value(),
    )
    .ok();
    assert!(app.state.observation_cache.is_some());

    app.state.recalculate_production();
    assert!(app.state.production_preparation_pending);
    assert!(app.state.production.is_none());

    let context = egui::Context::default();
    app.maybe_start_production_preparation(&context);
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.jobs.production_preparation.is_running()
        && !app.jobs.production_preparation.is_finished()
        && Instant::now() < deadline
    {
        std::thread::yield_now();
    }
    assert!(app.jobs.production_preparation.is_finished());

    app.poll_production_preparation();

    assert!(!app.state.production_preparation_pending);
    assert_eq!(
        app.state
            .production
            .as_ref()
            .map(|timeline| timeline.target),
        Some(5000)
    );
}

#[test]
fn production_worker_keeps_pending_state_until_completion() {
    let mut app = UiApp::default();
    app.state.seed = format!("{TEST_SEED:08X}");
    app.state.range_start = "0".into();
    app.state.range_end = "1000".into();
    app.state.target = "5000".into();
    app.state.npc = "5".into();
    app.state.consider_rotom_talk = false;
    app.state.candidates = vec![crate::application::state::TimelineCandidate {
        start_position: 0,
        sfmt_position: 1000,
        next_blink_ticks: Some(130),
        target_reachable: true,
    }];
    app.state.observation_cache = crate::domain::rng::generate_observation_cache(
        TEST_SEED,
        0..=100_000,
        100_000,
        app.state.fps_value(),
    )
    .ok();
    app.state.mark_production_preparation_pending();
    let (request, _) = app
        .state
        .production_preparation_request()
        .expect("production request");
    let (release, wait_for_release) = std::sync::mpsc::channel();
    app.jobs.production_preparation.spawn(move || {
        wait_for_release.recv().expect("test releases worker");
        (request, Err(AppError::Cancelled))
    });

    app.poll_production_preparation();
    assert!(app.state.production_preparation_pending);

    release.send(()).expect("worker is still waiting");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !app.jobs.production_preparation.is_finished() {
        assert!(
            Instant::now() < deadline,
            "worker should finish after release"
        );
        std::thread::yield_now();
    }
    app.poll_production_preparation();

    assert!(!app.state.production_preparation_pending);
}

#[test]
fn field_extension_worker_error_is_reported_and_blocks_retry() {
    let mut app = app_with_empty_field_timeline();
    app.jobs.field_timeline_extension_key = Some(app.jobs.field_timeline_extension_generation);
    app.jobs
        .field_timeline_extension
        .spawn(|| Err(AppError::InvalidState(StateIssue::CalculationUnavailable)));
    while app.jobs.field_timeline_extension.is_running()
        && !app.jobs.field_timeline_extension.is_finished()
    {
        std::thread::yield_now();
    }

    app.poll_field_timeline_extension();

    assert!(app.jobs.field_timeline_extension_blocked);
    assert!(matches!(
        app.jobs.field_timeline_extension_error.as_deref(),
        Some(AppError::InvalidState(StateIssue::CalculationUnavailable))
    ));
    assert!(app.field.notice.is_none());
}

#[test]
fn stale_field_extension_generation_cannot_change_current_state() {
    let mut app = app_with_empty_field_timeline();
    let original_timeline = app.field.results[0].timeline.clone();
    let generation = app.jobs.field_timeline_extension_generation;
    app.jobs.field_timeline_extension_key = Some(generation);
    let stale = app.field.timeline_extension_result_for_test(Some(1234));
    app.jobs.field_timeline_extension.spawn(move || Ok(stale));
    while !app.jobs.field_timeline_extension.is_finished() {
        std::thread::yield_now();
    }
    app.jobs.field_timeline_extension_generation = generation.wrapping_add(1);

    app.poll_field_timeline_extension();

    assert_eq!(app.field.results[0].timeline, original_timeline);
    assert!(app.field.timeleap_safe_frame_display.is_none());
    assert!(!app.jobs.field_timeline_extension_blocked);
    assert!(app.jobs.field_timeline_extension_error.is_none());
}

#[test]
fn repeated_gui_updates_do_not_queue_parallel_field_timeline_workers() {
    let mut app = UiApp::default();
    let context = egui::Context::default();
    let (release_sender, release_receiver) = std::sync::mpsc::channel();
    app.jobs.field_timeline_extension.spawn(move || {
        let _ = release_receiver.recv();
        Err(AppError::Cancelled)
    });
    let worker_id = app.jobs.field_timeline_extension.thread_id().unwrap();

    for _ in 0..100 {
        app.start_field_timeline_extension(&context);
        assert_eq!(
            app.jobs.field_timeline_extension.thread_id().unwrap(),
            worker_id
        );
    }

    release_sender.send(()).unwrap();
    while !app.jobs.field_timeline_extension.is_finished() {
        std::thread::yield_now();
    }
    let (cancelled, Ok(Err(AppError::Cancelled))) =
        app.jobs.field_timeline_extension.take_finished().unwrap()
    else {
        panic!("field timeline worker should finish successfully");
    };
    assert!(!cancelled);
}

#[test]
fn space_start_cancel_repeated_five_times_does_not_leak_cache_files() {
    let mut app = UiApp::default();
    // This test covers cancellation and cache lifetime, not native audio-device teardown.
    app.beep_enabled = false;
    app.state.seed = format!("{TEST_SEED:08X}");
    app.state.range_start = HIGH_START.to_string();
    app.state.range_end = HIGH_END.to_string();
    app.state.target = HIGH_END.to_string();
    let ctx = egui::Context::default();
    let cache_path =
        crate::infrastructure::cache_store::CacheStore::default().path(TEST_SEED, HIGH_END + 4096);

    let mut started_sessions = 0;
    let mut blocked_restarts = 0;
    for attempt in 0..5 {
        app.start_observation_and_generation(&ctx);
        assert!(app.state.is_observing(), "session {attempt} should start");
        started_sessions += 1;
        std::thread::sleep(Duration::from_millis(10));
        app.state.cancel_observation();
        app.cancel_observation_generation();
        app.clear_unused_sfmt_cache_files();

        if attempt == 0 {
            // A Space press before the generator acknowledges cancellation cannot start
            // another session; verify it remains idle instead of duplicating the worker.
            app.start_observation_and_generation(&ctx);
            assert!(!app.state.is_observing());
            blocked_restarts += 1;
        }

        // Polling while idle discards the canceled result and waits for the cache lease
        // to be released before the next actual cycle.
        let wait_started = Instant::now();
        while app.jobs.generation.is_running() && !app.jobs.generation.is_finished() {
            assert!(wait_started.elapsed() < Duration::from_secs(30));
            std::thread::sleep(Duration::from_millis(10));
        }
        app.poll_generation();

        for _ in 0..3_000 {
            if !cache_path.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !cache_path.exists(),
            "session {attempt} cache should be removed"
        );
    }
    eprintln!(
            "five 10 ms start/cancel cycles: {started_sessions} started and each cache was removed; {blocked_restarts} immediate restart was blocked until generation completed"
        );
}

#[test]
#[ignore = "generates a roughly 400 MiB cache through Frame 50,000,000 to measure cancel-to-delete latency"]
fn measure_high_frame_cancel_to_cache_deletion() {
    const TARGET: i64 = 50_000_000;
    const SEED: u32 = 0x7A31_C5E2;

    let mut app = UiApp::default();
    app.beep_enabled = false;
    app.state.seed = format!("{SEED:08X}");
    app.state.range_start = (TARGET - 100).to_string();
    app.state.range_end = TARGET.to_string();
    app.state.target = TARGET.to_string();
    let ctx = egui::Context::default();
    let cache_path =
        crate::infrastructure::cache_store::CacheStore::default().path(SEED, TARGET + 4096);

    let started = Instant::now();
    app.start_observation_and_generation(&ctx);
    assert!(app.state.is_observing());
    let generation_started = Instant::now();
    std::thread::sleep(Duration::from_millis(10));
    app.state.cancel_observation();
    app.cancel_observation_generation();
    let cancelled = Instant::now();
    app.clear_unused_sfmt_cache_files();

    let worker_wait_started = Instant::now();
    while app.jobs.generation.is_running() && !app.jobs.generation.is_finished() {
        assert!(worker_wait_started.elapsed() < Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(10));
    }
    let worker_finished = Instant::now();
    app.poll_generation();

    let delete_wait_started = Instant::now();
    while cache_path.exists() {
        assert!(delete_wait_started.elapsed() < Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(1));
    }
    let deleted = Instant::now();

    eprintln!(
            "Frame 50,000,000: generation-before-cancel={:?}, cancel-to-worker-finish={:?}, worker-finish-to-delete={:?}, start-to-delete={:?}",
            cancelled.duration_since(generation_started),
            worker_finished.duration_since(cancelled),
            deleted.duration_since(worker_finished),
            deleted.duration_since(started)
        );
}
