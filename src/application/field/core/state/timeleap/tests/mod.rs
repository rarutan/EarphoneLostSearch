use super::*;

fn confirmed_target_search_state() -> FieldState {
    let seed = 0xA02D_5C73;
    let status = crate::domain::timeleap::TimeleapStatus {
        model: crate::domain::rng::ModelStatus::new(2),
        phase: 1,
        fidget_cd: 2,
        is_boy: true,
        raining: false,
    };
    let candidate = crate::domain::timeleap::TimeleapCandidate {
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
        target_inputs: vec![crate::domain::timeleap::TimeleapTargetInput {
            frame: 500,
            tick: 250,
        }]
        .into(),
    };
    let mut state = FieldState {
        seed: format!("{seed:08X}"),
        timeleap_source_inputs_initialized: true,
        timeleap_source_seed: format!("{seed:08X}"),
        timeleap_source_starting_frame: "480".into(),
        timeleap_source_npc_count: "1".into(),
        timeleap_source_frame1: "480".into(),
        timeleap_enabled: true,
        timeleap_stage: TimeleapStage::Succeeded,
        timeleap_confirmed_frame1: Some(candidate.frame1),
        timeleap_confirmed_candidate: Some(candidate),
        timeleap_shift_times: vec![Instant::now()],
        timeleap_generation_start_frame: "900".into(),
        target_consumption: "1000".into(),
        results: vec![FieldResult {
            start_consumption: 480,
            first_wait_frame: 0,
            last_wait_frame: 0,
            first_consumption: 480,
            last_consumption: 480,
            intervals: Vec::new(),
            target_consumptions: Vec::new(),
            next_blink_wait_frame: 0,
            target_wait_frame: None,
            target_exact: None,
            target_frame_before: None,
            target_frame_after: None,
            observed_model: 0,
            models: 2,
            timeline: Vec::new(),
            candidate_blinks: Vec::new(),
            target_blink_count: None,
            target_blink_count_offset_from_first_row: 0,
            timeline_cursor: None,
            target_cursor: None,
        }],
        ..FieldState::default()
    };
    state
        .commit_target_input()
        .expect("valid baseline TargetFrame");
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=2_000,
        2_000,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate cache for confirmed Target search state");
    state.set_observation_cache(cache);
    state.target_input_locked = true;
    assert!(state.adopt_timeleap_timeline());
    state
}

mod candidate_search;
mod lifecycle;
mod observation;
mod reachability;
mod rng_search;
mod shift_resolution;
mod target_search;
mod timeline;
mod timer;
