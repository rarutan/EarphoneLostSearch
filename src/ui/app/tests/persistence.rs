use super::*;

#[test]
fn timeleap_methods_round_trip_through_app_state_for_valid_preset() {
    use crate::domain::timeleap::TimeleapMode;

    for mode in [
        TimeleapMode::Menu,
        TimeleapMode::Dialogue,
        TimeleapMode::Courier,
    ] {
        let mut saved = PersistedAppState::default();
        saved.field.timeleap.options.mode = mode;
        saved.field.timeleap.options.search_category = 1;
        saved.field.timeleap.options.search_preset = 1;

        let mut app = UiApp::default();
        app.apply_persisted_state(saved);
        assert_eq!(app.field.timeleap_mode, mode);
        assert_eq!(app.field.timeleap_search_category, 1);
        assert_eq!(app.field.timeleap_search_preset, 1);

        let round_tripped = app.persisted_state();
        assert_eq!(round_tripped.field.timeleap.options.mode, mode);
        assert_eq!(round_tripped.field.timeleap.options.search_category, 1);
        assert_eq!(round_tripped.field.timeleap.options.search_preset, 1);
    }
}

#[test]
fn game_edition_and_edition_specific_preset_round_trip_through_app_state() {
    use crate::domain::fixed_rng_catalog::{selected_preset_for, GameEdition, SM_CATEGORIES};

    let mut saved = PersistedAppState::default();
    saved.field.timeleap.options.game_edition = GameEdition::SunMoon;
    saved.field.timeleap.options.search_category = 1;
    saved.field.timeleap.options.search_preset = 4;

    let mut app = UiApp::default();
    app.apply_persisted_state(saved);
    assert_eq!(app.field.timeleap_game_edition, GameEdition::SunMoon);
    assert_eq!(app.field.timeleap_search_category, 1);
    assert_eq!(app.field.timeleap_search_preset, 4);
    assert_eq!(
        selected_preset_for(
            app.field.timeleap_game_edition,
            app.field.timeleap_search_category,
            app.field.timeleap_search_preset,
        )
        .unwrap()
        .species_id,
        791
    );
    assert_eq!(
        app.persisted_state().field.timeleap.options.game_edition,
        GameEdition::SunMoon
    );
    assert_eq!(SM_CATEGORIES[1].presets[4].species_id, 791);
}

#[test]
fn old_edition_selection_and_invalid_indices_restore_safely() {
    use crate::domain::fixed_rng_catalog::{categories, GameEdition};

    let old_json: PersistedAppState = serde_json::from_value(serde_json::json!({
        "field": {"timeleap": {"options": {"search_category": 1, "search_preset": 0}}}
    }))
    .expect("old nested settings without edition");
    let mut old_app = UiApp::default();
    old_app.apply_persisted_state(old_json);
    assert_eq!(
        old_app.field.timeleap_game_edition,
        GameEdition::UltraSunUltraMoon
    );
    assert_eq!(old_app.field.timeleap_search_category, 1);

    let mut invalid = PersistedAppState::default();
    invalid.field.timeleap.options.game_edition = GameEdition::SunMoon;
    invalid.field.timeleap.options.search_category = usize::MAX;
    invalid.field.timeleap.options.search_preset = usize::MAX;
    let mut invalid_app = UiApp::default();
    invalid_app.apply_persisted_state(invalid);
    assert_eq!(
        invalid_app.field.timeleap_game_edition,
        GameEdition::SunMoon
    );
    assert_eq!(
        (
            invalid_app.field.timeleap_search_category,
            invalid_app.field.timeleap_search_preset
        ),
        (0, 0)
    );
    assert_ne!(
        invalid_app.field.timeleap_search_category,
        categories(GameEdition::SunMoon).len()
    );
}

#[test]
fn hatch_npc_value_round_trips_through_persisted_app_state() {
    let mut app = UiApp::default();
    app.state.npc = "13".into();

    let json = serde_json::to_string(&app.persisted_state()).expect("settings serialize");
    let saved = serde_json::from_str::<PersistedAppState>(&json).expect("settings deserialize");
    let mut restored = UiApp::default();
    restored.apply_persisted_state(saved);

    assert_eq!(restored.state.npc, "13");
    assert_eq!(restored.state.search_config().unwrap().npc_models, 14);
}

#[test]
fn persisted_main_seed_is_copied_to_timeleap_source_at_startup() {
    let mut app = UiApp::default();
    let mut saved = PersistedAppState::default();
    saved.field.seed = "B248EA3B".into();
    saved.field.timeleap.source.inputs_initialized = true;
    saved.field.timeleap.source.starting_frame = "490".into();
    saved.field.timeleap.source.npc_count = "15".into();
    saved.field.timeleap.source.frame1 = "999".into();
    saved.field.timeleap.frame1_search.frame1_input = "1200".into();
    saved.field.timeleap.target_search.drift_search_enabled = true;
    saved
        .field
        .timeleap
        .target_search
        .drift_search_generation_start_frame = "70000".into();
    saved.field.timeleap.target_search.drift_range_ticks = "31".into();
    saved.field.timeleap.options.direct_encounter = true;

    app.apply_persisted_state(saved);

    assert_eq!(app.field.seed, "B248EA3B");
    assert_eq!(app.field.timeleap_source_seed, "B248EA3B");
    assert_eq!(app.field.timeleap_source_starting_frame, "490");
    assert_eq!(app.field.timeleap_source_npc_count, "15");
    assert!(app.field.timeleap_direct_encounter);
    assert_eq!(app.field.timeleap_source_frame1, "1200");
    assert!(app.field.confirmed_target_search_form.drift_search_enabled);
    assert!(
        app.persisted_state()
            .field
            .timeleap
            .options
            .direct_encounter
    );
    assert_eq!(
        app.field
            .confirmed_target_search_form
            .drift_search_generation_start_frame,
        "70000"
    );
    assert_eq!(
        app.field.confirmed_target_search_form.drift_range_ticks,
        "31"
    );
}

#[test]
fn confirmed_target_drift_search_conditions_round_trip_through_app_settings() {
    let mut app = UiApp::default();
    app.field.confirmed_target_search_form.initialized = true;
    app.field.confirmed_target_search_form.drift_search_enabled = true;
    app.field
        .confirmed_target_search_form
        .drift_search_generation_start_frame = "70000".into();
    app.field.confirmed_target_search_form.drift_range_ticks = "31".into();
    app.field.confirmed_target_search_form.grace_min_frames = "7".into();

    let saved = app.persisted_state();
    let mut restored = UiApp::default();
    restored.apply_persisted_state(saved);

    assert!(
        restored
            .field
            .confirmed_target_search_form
            .drift_search_enabled
    );
    assert_eq!(
        restored
            .field
            .confirmed_target_search_form
            .drift_search_generation_start_frame,
        "70000"
    );
    assert_eq!(
        restored
            .field
            .confirmed_target_search_form
            .drift_range_ticks,
        "31"
    );
    assert_eq!(
        restored.field.confirmed_target_search_form.grace_min_frames,
        "7"
    );
}

#[test]
fn legacy_shared_frame1_filter_values_migrate_without_losing_the_mode_value() {
    let mut app = UiApp::default();
    let mut saved = PersistedAppState::default();
    saved.field.timeleap.frame1_search.filter_mode = 1;
    saved.field.timeleap.frame1_search.seconds_min = "10000".into();
    saved.field.timeleap.frame1_search.seconds_max = "12000".into();
    saved.field.timeleap.frame1_search.frame1_min = None;
    saved.field.timeleap.frame1_search.frame1_max = None;

    app.apply_persisted_state(saved);

    assert_eq!(app.field.timeleap_frame1_min, "10000");
    assert_eq!(app.field.timeleap_frame1_max, "12000");
    assert_eq!(app.field.timeleap_seconds_min, "10000");
    assert_eq!(app.field.timeleap_seconds_max, "12000");
}

#[test]
fn frame1_offset_difference_converts_signed_timeline_ticks_to_frames() {
    assert_eq!(timeleap_frame1_offset_drift(5_002, 5_000), Some(4));
    assert_eq!(timeleap_frame1_offset_drift(5_000, 5_002), Some(-4));
    assert_eq!(timeleap_frame1_offset_drift(5_001, 5_000), Some(2));
    assert_eq!(timeleap_frame1_offset_drift(-1, 5_000), None);
}
