use super::*;

#[test]
fn frame1_reachability_indicator_is_preserved_across_ready_and_rechecks() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_frame1_input = "10000".into();

    assert_eq!(state.timeleap_frame1_indicator(), None);
    state.timeleap_frame1_reachable = Some(true);
    assert_eq!(
        state.timeleap_frame1_indicator(),
        Some(TimeleapFrame1Indicator::Reachable)
    );
    state.set_timeleap_frame1_unreachable();
    assert_eq!(
        state.timeleap_frame1_indicator(),
        Some(TimeleapFrame1Indicator::Unreachable)
    );

    state.timeleap_stage = TimeleapStage::Succeeded;
    assert_eq!(
        state.timeleap_frame1_indicator(),
        Some(TimeleapFrame1Indicator::Unreachable),
        "post-Timeleap Timeline confirmation must not replace ordinary Timeline reachability"
    );
    state.timeleap_frame1_reachable = Some(true);
    assert_eq!(
        state.timeleap_frame1_indicator(),
        Some(TimeleapFrame1Indicator::Reachable)
    );
    state.timeleap_stage = TimeleapStage::Recheck;
    assert_eq!(
        state.timeleap_frame1_indicator(),
        Some(TimeleapFrame1Indicator::Reachable)
    );
    state.timeleap_stage = TimeleapStage::Failed;
    state.timeleap_frame1_target = Some(10_005);
    assert_eq!(
        state.timeleap_frame1_indicator(),
        Some(TimeleapFrame1Indicator::Reachable)
    );

    state.timeleap_stage = TimeleapStage::Off;
    state.invalidate_timeleap_frame1_preview();
    assert_eq!(state.timeleap_frame1_indicator(), None);

    state.timeleap_frame1_reachable = Some(true);
    state.begin_timeleap_frame1_preparation();
    assert_eq!(
        state.timeleap_frame1_indicator(),
        Some(TimeleapFrame1Indicator::Reachable),
        "accepting Ready must preserve the last computed reachability result"
    );
}

#[test]
fn target_indicator_uses_post_frame1_reachability_only_before_timeleap_succeeds() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_stage = TimeleapStage::Off;
    state.timeleap_target_reachable = Some(false);
    assert_eq!(
        state.target_reachability_for_display(Some(true)),
        Some(false)
    );

    state.timeleap_stage = TimeleapStage::Frame1Waiting;
    assert_eq!(
        state.target_reachability_for_display(Some(true)),
        Some(false)
    );

    // During the post-Frame1 blink recheck, the old target result is hidden.
    state.timeleap_stage = TimeleapStage::Recheck;
    assert_eq!(state.target_reachability_for_display(Some(true)), None);

    // After successful adoption, the active post-leap Timeline owns the
    // normal Target indicator again.
    state.timeleap_stage = TimeleapStage::Succeeded;
    assert_eq!(
        state.target_reachability_for_display(Some(true)),
        Some(true)
    );

    state.timeleap_stage = TimeleapStage::Assumed;
    assert_eq!(
        state.target_reachability_for_display(Some(true)),
        Some(true)
    );

    state.timeleap_enabled = false;
    state.timeleap_stage = TimeleapStage::Off;
    assert_eq!(
        state.target_reachability_for_display(Some(true)),
        Some(true)
    );
}

#[test]
fn reachability_form_ignores_results_from_an_older_input_generation() {
    let mut state = FieldState::default();
    state.timeleap_reachability_form.generation = 8;
    state.timeleap_reachability_form.status = TimeleapReachabilityStatus::Running;

    state.apply_timeleap_reachability_result(
        7,
        Ok(crate::domain::timeleap::TimeleapReachability::Reachable {
            target_input_frames: vec![1_000],
        }),
    );
    assert!(matches!(
        state.timeleap_reachability_form.status,
        TimeleapReachabilityStatus::Running
    ));

    state.apply_timeleap_reachability_result(
        8,
        Ok(crate::domain::timeleap::TimeleapReachability::Reachable {
            target_input_frames: vec![1_005],
        }),
    );
    assert!(matches!(
        state.timeleap_reachability_form.status,
        TimeleapReachabilityStatus::Reachable(ref frames) if frames == &[1_005]
    ));
}

#[test]
fn reachability_form_starts_from_the_visible_starting_frame() {
    let mut state = FieldState::default();
    state.seed = "436F7E6C".into();
    state.npc_count = "1".into();
    state.timeleap_frame1_target = Some(500);
    state.timeleap_generation_start_frame = "1000".into();
    state.results.push(FieldResult {
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
        timeline: vec![FieldTimelineRow {
            frame: 480,
            timeline_frame: 0,
            duration_frames: 0,
        }],
        candidate_blinks: Vec::new(),
        target_blink_count: None,
        target_blink_count_offset_from_first_row: 0,
        timeline_cursor: None,
        target_cursor: None,
    });

    state.initialize_timeleap_reachability_form();

    assert_eq!(state.timeleap_reachability_form.starting_frame, "480");
    assert_eq!(state.timeleap_reachability_form.frame1, "500");
    assert_eq!(state.timeleap_reachability_form.models, 2);
}

#[test]
fn reachability_request_is_an_independent_snapshot_of_the_visible_form() {
    let seed = 0x436f_7e6c;
    let cache = crate::domain::rng::generate_observation_cache(
        seed,
        0..=4_000,
        4_000,
        crate::domain::timing::DEFAULT_FPS,
    )
    .expect("generate cache for reachability form snapshot");
    let mut state = FieldState::default();
    state.set_observation_cache(cache);
    let form = &mut state.timeleap_reachability_form;
    form.initialized = true;
    form.seed = format!("{seed:08X}");
    form.models = 4;
    form.category = 0;
    form.preset = 0;
    form.mode = TimeleapMode::Menu;
    form.is_boy = true;
    form.starting_frame = "500".into();
    form.frame1 = "500".into();
    form.consider_delay_enabled = false;
    form.consider_delay_frames = "0".into();
    form.generation_start_frame = "1000".into();

    let (generation, request, _) = state
        .begin_timeleap_reachability_check()
        .expect("valid visible inputs produce a request snapshot");
    state.timeleap_reachability_form.frame1 = "520".into();
    state.timeleap_reachability_form.generation_start_frame = "1100".into();

    assert_eq!(generation, 1);
    assert_eq!(request.frame1, 500);
    assert_eq!(request.config.start_frame, 500);
    assert_eq!(request.config.target_frame, 1_000);
    assert_eq!(request.config.models, 4);
    assert_eq!(request.config.mode, TimeleapMode::Menu);
    assert!(request.consider_delay.is_none());
}

#[test]
fn reachability_snapshot_uses_the_selected_editions_preset_profile() {
    use crate::domain::fixed_rng_catalog::{find_unique_preset, selected_preset_for, GameEdition};
    use crate::domain::timeleap::ConsiderDelayGenerator;

    for edition in GameEdition::ALL {
        let (category, preset_index) =
            find_unique_preset(edition, 791, "").expect("edition has a Solgaleo preset");
        let preset = selected_preset_for(edition, category, preset_index).unwrap();
        let form = TimeleapReachabilityForm {
            models: 4,
            game_edition: edition,
            category,
            preset: preset_index,
            consider_delay_enabled: true,
            ..Default::default()
        };
        let delay = super::forms::timeleap_consider_delay_config(
            form.game_edition,
            form.category,
            form.preset,
            form.consider_delay_enabled,
            i64::from(preset.delay_frames),
            &form.courier,
        )
        .expect("Consider Delay is enabled in the form");
        assert_eq!(
            delay.generator,
            ConsiderDelayGenerator::FixedSymbol(preset.delay_type),
            "edition {edition:?}"
        );
        assert_eq!(delay.delay_frames, i64::from(preset.delay_frames));
    }
}
