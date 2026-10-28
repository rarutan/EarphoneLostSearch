use super::*;

#[test]
fn timeleap_help_copy_matches_the_requested_frame_semantics() {
    assert_eq!(
        super::super::TIMELEAP_MAIN_FRAME1_GRACE_HELP,
        "同じ生成開始Frameにたどり着けるTimelineへ移動できるFrame1が、前後にどれくらい連続しているかを2F単位で表示します。"
    );
    assert_eq!(
        super::super::TIMELEAP_MAIN_TARGET_GRACE_HELP,
        "同じ生成開始Frameにたどり着けるTargetFrameが、前後にどれくらい連続しているかを2F単位で表示します。"
    );
    assert_eq!(
        super::super::TIMELEAP_MAIN_FRAME1_GRACE_FILTER_HELP,
        "同じ生成開始Frameにたどり着けるタイムラインへ移動できるFrame1が、どれくらい連続しているか、その下限を設定します。"
    );
    assert_eq!(
        super::super::TIMELEAP_MAIN_TARGET_GRACE_FILTER_HELP,
        "同じ生成開始Frameへ到達するA入力TargetFrameが、どれくらい連続しているか、その下限を設定します。"
    );
    assert_eq!(
        super::super::TIMELEAP_MAIN_SECONDS_ONE_HELP,
        "現在時刻からFrame1までの残り秒数です。内部で使われているタイムラインと、入力されている情報が違う場合、StartingFrameからFrame1までの秒数になります。"
    );
    assert_eq!(
        super::super::TIMELEAP_MAIN_SECONDS_TWO_HELP,
        "Frame1からTargetFrameまでの時間です。"
    );
    assert_eq!(
        super::super::TIMELEAP_TARGET_DRIFT_HELP,
        "生成開始Frameからどれくらいフレームずれしているかを2F単位で表示します。"
    );
    assert_eq!(
        super::super::TIMELEAP_TARGET_GRACE_HELP,
        "同じ生成開始Frameにたどり着けるTargetFrameが、前後にどれくらい連続しているかを2F単位で表示します。"
    );
}

#[test]
fn timeleap_search_tabs_render_in_the_selected_language() {
    let context = egui::Context::default();
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 500.0));

    for (language, expected, absent) in [
        (
            crate::ui::i18n::Language::Japanese,
            ["Frame1検索", "TargetFrame検索", "配達員", "設定"],
            "Frame1 Search",
        ),
        (
            crate::ui::i18n::Language::English,
            [
                "Frame1 Search",
                "TargetFrame Search",
                "Deliveryman",
                "Settings",
            ],
            "Frame1検索",
        ),
    ] {
        let mut selected_tab = TIMELEAP_TAB_FRAME1_SEARCH;
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(screen_rect),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    timeleap_search_tabs_ui_with_texts(
                        ui,
                        &mut selected_tab,
                        &Texts::new(language),
                    );
                });
            },
        );
        let rendered_labels = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.to_string()),
                _ => None,
            })
            .collect::<Vec<_>>();
        for label in expected {
            assert!(
                rendered_labels.iter().any(|rendered| rendered == label),
                "missing rendered tab label {label:?} for {language:?}; got {rendered_labels:?}"
            );
        }
        assert!(
            !rendered_labels.iter().any(|rendered| rendered == absent),
            "unexpected label {absent:?} for {language:?}"
        );
    }
}

#[test]
fn unlabeled_edition_selector_stays_after_four_tabs_in_both_languages() {
    for width in [920.0, 1_080.0] {
        for language in [
            crate::ui::i18n::Language::Japanese,
            crate::ui::i18n::Language::English,
        ] {
            let context = egui::Context::default();
            let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 500.0));
            let mut state = FieldState::default();
            let mut header = None;
            let output = context.run(
                egui::RawInput {
                    screen_rect: Some(screen_rect),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        header = Some(timeleap_search_header_ui(
                            ui,
                            &mut state,
                            &Texts::new(language),
                        ));
                    });
                },
            );
            let header = header.expect("header layout");
            assert!(header
                .tab_rects
                .windows(2)
                .all(|pair| pair[0].right() <= pair[1].left()));
            assert!(header.tab_rects[3].right() < header.edition_rect.left());
            assert!(header.edition_rect.right() <= screen_rect.right());
            let rendered_labels = output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => Some(text.galley.job.text.to_string()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(!rendered_labels
                .iter()
                .any(|label| label == "Game" || label == "ゲーム版"));
            assert!(rendered_labels.iter().any(|label| label == "USUM"));
        }
    }
}

#[test]
fn category_and_pokemon_controls_follow_the_selected_edition_catalog() {
    use crate::domain::fixed_rng_catalog::GameEdition;

    for (edition, category_label, preset_index) in [
        (GameEdition::SunMoon, "Legendary", 4),
        (GameEdition::UltraSunUltraMoon, "Alola Legendary", 5),
    ] {
        let context = egui::Context::default();
        let mut state = FieldState::default();
        state.timeleap_game_edition = edition;
        state.timeleap_search_category = 1;
        state.timeleap_search_preset = preset_index;
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(700.0, 500.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    let _ = timeleap_category_pokemon_method_controls(
                        ui,
                        &mut state,
                        &Texts::new(crate::ui::i18n::Language::English),
                        "edition-catalog-test",
                        400.0,
                        None,
                    );
                });
            },
        );
        let rendered_labels = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.to_string()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(rendered_labels.iter().any(|label| label == category_label));
        assert!(rendered_labels.iter().any(|label| label == "Solgaleo"));
    }
}

#[test]
fn english_target_search_form_renders_inside_compact_and_standard_windows() {
    for width in [460.0, 1_000.0] {
        let context = egui::Context::default();
        let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 720.0));
        let mut state = FieldState::default();
        state.initialize_timeleap_source_inputs();
        let mut actions = None;
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(screen_rect),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    actions = Some(confirmed_target_search_tab_ui(
                        ui,
                        &mut state,
                        &Texts::new(crate::ui::i18n::Language::English),
                        false,
                        false,
                        false,
                    ));
                });
            },
        );

        for label in [
            "Category",
            "Pokémon",
            "Timeleap method",
            "Generation start frame",
            "Frame1",
            "TargetFrame",
            "Drift",
            "Streak",
            "Search",
        ] {
            assert!(
                output.shapes.iter().any(|clipped| matches!(
                    &clipped.shape,
                    egui::Shape::Text(text) if text.galley.job.text == label
                )),
                "missing English label {label:?} at window width {width}"
            );
        }

        let actions = actions.expect("TargetFrame search form should render");
        assert!(actions.search_button_rect.left() >= screen_rect.left());
        assert!(actions.search_button_rect.right() <= screen_rect.right());
        for rect in actions.selection_combo_rects {
            assert!(rect.left() >= screen_rect.left());
            assert!(rect.right() <= screen_rect.right());
        }
    }
}

#[test]
fn wondercard_import_replaces_existing_your_id_and_egg_flags() {
    let mut data = vec![0; 264];
    data[0x82..0x84].copy_from_slice(&803u16.to_le_bytes());
    data[0xb5] = 0;
    data[0xd1] = 0;
    let settings = crate::domain::wondercard::parse(&data).unwrap();

    let mut state = FieldState::default();
    state.timeleap_search_category = state.timeleap_courier_category();
    state.timeleap_courier_your_id = true;
    state.timeleap_courier_is_egg = true;
    state.timeleap_consider_delay_frames = "999".into();
    apply_wondercard_settings(&mut state, settings);

    assert!(!state.timeleap_courier_your_id);
    assert!(!state.timeleap_courier_is_egg);
    assert_eq!(state.timeleap_consider_delay_frames, "0");
}

#[test]
fn wondercard_identity_flag_selects_edition_event_delay_only_for_non_eggs() {
    let mut data = vec![0; 264];
    data[0x82..0x84].copy_from_slice(&803u16.to_le_bytes());
    data[0xb5] = 3;
    data[0xd1] = 0;
    let settings = crate::domain::wondercard::parse(&data).unwrap();

    let mut state = FieldState::default();
    state.timeleap_search_category = state.timeleap_courier_category();
    state.timeleap_consider_delay_enabled = false;
    apply_wondercard_settings(&mut state, settings);
    assert!(state.timeleap_courier_your_id);
    assert_eq!(state.timeleap_consider_delay_frames, "42");
    assert!(!state.timeleap_consider_delay_enabled);

    state.set_timeleap_game_edition(crate::domain::fixed_rng_catalog::GameEdition::SunMoon);
    assert_eq!(state.timeleap_consider_delay_frames, "62");
}

#[test]
fn timeleap_keepalive_skips_field_settings_and_preserves_timer_state() {
    let context = egui::Context::default();
    let mut state = FieldState::default();
    state.seed = "12345678".to_owned();
    let seed_before = state.seed.clone();
    let timer_kind_before = state.timer_kind();
    let mut actions = FieldUiActions::default();
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(500.0, 300.0),
            )),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                actions = timeleap_window_keepalive_ui(
                    ui,
                    &mut state,
                    FieldUiWorkerState::default(),
                    context,
                    30.0,
                    true,
                    ColorScheme::Standard,
                    &Texts::new(crate::ui::i18n::Language::Japanese),
                    false,
                );
            });
        },
    );

    assert_eq!(state.seed, seed_before);
    assert_eq!(state.timer_kind(), timer_kind_before);
    assert!(
        output
            .shapes
            .iter()
            .all(|shape| !matches!(&shape.shape, egui::epaint::Shape::Text(_))),
        "the hidden-tab keepalive must not render the Field settings panel"
    );
    assert!(!actions.search_requested);
    assert!(!actions.observe_requested);
    assert!(!actions.timeleap_enabled_changed);
}

#[test]
fn confirmed_target_search_tab_stays_visible_until_timeline_adoption() {
    let mut state = FieldState::default();
    state.seed = "12345678".into();
    state.timeleap_frame1_input = "10000".into();
    let context = egui::Context::default();
    let mut response = None;
    let output = context.run(egui::RawInput::default(), |context| {
        egui::CentralPanel::default().show(context, |ui| {
            response = Some(confirmed_target_search_tab_ui(
                ui,
                &mut state,
                &Texts::new(crate::ui::i18n::Language::Japanese),
                false,
                false,
                false,
            ));
        });
    });
    let response = response.expect("TargetFrame search tab should render");
    assert!(!response.search_requested);
    assert!(!response.preview_requested);
    assert!(response.selected_row.is_none());
    assert!(response.search_button_rect.width() > 100.0);
    assert!(text_shape_rect(&output, "猶予").width() > 0.0);
    let search_text = text_shape_rect(&output, "検索");
    assert!(
        (search_text.center().x - response.search_button_rect.center().x).abs() <= 0.5,
        "TargetFrame search label {:?} is not centered in button {:?}",
        search_text,
        response.search_button_rect
    );
    assert!(state.confirmed_target_search_form.initialized);
    assert!(!confirmed_target_search_can_search(false, false));
    assert!(confirmed_target_search_can_search(true, false));
    assert!(!confirmed_target_search_can_search(true, true));
    state.initialize_timeleap_source_inputs();
    assert_eq!(state.timeleap_source_seed, "12345678");
    assert!(state.timeleap_source_starting_frame.is_empty());
    assert!(state.timeleap_source_npc_count.is_empty());
    assert_eq!(state.timeleap_source_frame1, state.timeleap_frame1_input);
    state.timeleap_enabled = true;
    assert_eq!(actual_timeleap_frame1_for_display(&state), None);
    state.timeleap_confirmed_frame1 = Some(10004);
    assert_eq!(actual_timeleap_frame1_for_display(&state), None);
    state.timeleap_selected_frame = Some(10003);
    state.timeleap_stage = crate::application::field::TimeleapStage::Assumed;
    assert_eq!(actual_timeleap_frame1_for_display(&state), None);
    state.set_timeleap_timeline_anchor_for_test(Some(std::time::Instant::now()), Some(497));
    assert_eq!(actual_timeleap_frame1_for_display(&state), Some(10003));
    state.timeleap_stage = crate::application::field::TimeleapStage::Succeeded;
    assert_eq!(actual_timeleap_frame1_for_display(&state), Some(10004));
    state.timeleap_stage = crate::application::field::TimeleapStage::Recheck;
    state.timeleap_confirmed_frame1 = None;
    assert_eq!(actual_timeleap_frame1_for_display(&state), Some(10003));
    let (left, right) = timeleap_search_column_widths(1_000.0, 8.0);
    assert!((left / 992.0 - 0.28).abs() < f32::EPSILON);
    assert!((left + right + 8.0 - 1_000.0).abs() < f32::EPSILON);
    let (frame1_left, frame1_right) = timeleap_search_column_widths(1_000.0, 8.0);
    assert!((frame1_left / 992.0 - 0.28).abs() < f32::EPSILON);
    assert!((frame1_left + frame1_right + 8.0 - 1_000.0).abs() < f32::EPSILON);
    assert_eq!(TIMELEAP_SEARCH_WINDOW_HEIGHT, 610.0);
    assert!(matches!(
        state.confirmed_target_search_form.status,
        crate::application::field::ConfirmedTargetSearchStatus::Idle
    ));
}

#[test]
fn confirmed_target_search_button_emits_request_when_clicked_with_valid_inputs() {
    let context = egui::Context::default();
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 640.0));
    let mut state = FieldState::default();
    state.timeleap_source_inputs_initialized = true;
    state.timeleap_source_seed = "b248ea3b".into();
    state.timeleap_source_starting_frame = "490".into();
    state.timeleap_source_npc_count = "15".into();
    state.timeleap_source_frame1 = "10000".into();
    state.target_consumption = "1000".into();
    state
        .commit_target_input()
        .expect("valid TargetFrame input");
    let mut button_rect = egui::Rect::NOTHING;
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                button_rect = confirmed_target_search_tab_ui(
                    ui,
                    &mut state,
                    &Texts::new(crate::ui::i18n::Language::Japanese),
                    false,
                    false,
                    false,
                )
                .search_button_rect;
            });
        },
    );

    let pointer = button_rect.center();
    let mut requested = false;
    for pressed in [true, false] {
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(screen_rect),
                events: vec![
                    egui::Event::PointerMoved(pointer),
                    egui::Event::PointerButton {
                        pos: pointer,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::default(),
                    },
                ],
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    requested |= confirmed_target_search_tab_ui(
                        ui,
                        &mut state,
                        &Texts::new(crate::ui::i18n::Language::Japanese),
                        false,
                        false,
                        false,
                    )
                    .search_requested;
                });
            },
        );
        let _ = output;
    }
    assert!(button_rect.is_positive());
    assert!(requested, "clicking Search should return a worker request");
}

#[test]
fn frame1_search_button_changes_to_cancel_and_emits_the_matching_action() {
    let context = egui::Context::default();
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 180.0));
    for (running, expected_label, expected_action) in [
        (false, "検索", SearchButtonAction::Search),
        (true, "キャンセル", SearchButtonAction::Cancel),
    ] {
        let mut button_rect = egui::Rect::NOTHING;
        let initial = context.run(
            egui::RawInput {
                screen_rect: Some(screen_rect),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    button_rect = timeleap_candidate_search_button_with_texts(
                        ui,
                        true,
                        240.0,
                        &Texts::new(crate::ui::i18n::Language::Japanese),
                        running,
                    )
                    .rect;
                });
            },
        );
        assert!(button_rect.is_positive());
        assert!(text_shape_rect(&initial, expected_label).is_positive());

        let mut emitted_action = None;
        for pressed in [true, false] {
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(screen_rect),
                    events: vec![
                        egui::Event::PointerMoved(button_rect.center()),
                        egui::Event::PointerButton {
                            pos: button_rect.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::default(),
                        },
                    ],
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        let response = timeleap_candidate_search_button_with_texts(
                            ui,
                            true,
                            240.0,
                            &Texts::new(crate::ui::i18n::Language::Japanese),
                            running,
                        );
                        if response.clicked() {
                            emitted_action = Some(search_button_action(running));
                        }
                    });
                },
            );
        }
        assert_eq!(emitted_action, Some(expected_action));
    }
}

#[test]
fn confirmed_target_search_button_cancels_running_worker() {
    let context = egui::Context::default();
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 640.0));
    let mut state = FieldState::default();
    state.timeleap_source_inputs_initialized = true;
    state.timeleap_source_seed = "12345678".into();
    state.timeleap_source_starting_frame = "490".into();
    state.timeleap_source_npc_count = "15".into();
    state.timeleap_source_frame1 = "10000".into();
    state.target_consumption = "1000".into();
    state.commit_target_input().expect("valid target input");
    state
        .begin_confirmed_target_search()
        .expect("valid source starts a search and establishes Running state");

    let mut button_rect = egui::Rect::NOTHING;
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                button_rect = confirmed_target_search_tab_ui(
                    ui,
                    &mut state,
                    &Texts::new(crate::ui::i18n::Language::Japanese),
                    false,
                    false,
                    false,
                )
                .search_button_rect;
            });
        },
    );
    assert!(text_shape_rect(&output, "キャンセル").is_positive());

    let mut search_requested = false;
    let mut cancel_requested = false;
    for pressed in [true, false] {
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(screen_rect),
                events: vec![
                    egui::Event::PointerMoved(button_rect.center()),
                    egui::Event::PointerButton {
                        pos: button_rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::default(),
                    },
                ],
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    let actions = confirmed_target_search_tab_ui(
                        ui,
                        &mut state,
                        &Texts::new(crate::ui::i18n::Language::Japanese),
                        false,
                        false,
                        false,
                    );
                    search_requested |= actions.search_requested;
                    cancel_requested |= actions.cancel_requested;
                });
            },
        );
    }
    assert!(cancel_requested);
    assert!(!search_requested);

    state.cancel_confirmed_target_search();
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let _ = confirmed_target_search_tab_ui(
                    ui,
                    &mut state,
                    &Texts::new(crate::ui::i18n::Language::Japanese),
                    false,
                    false,
                    false,
                );
            });
        },
    );
    assert!(text_shape_rect(&output, "検索").is_positive());
}

#[test]
fn confirmed_target_cancel_button_tracks_worker_after_request_invalidation() {
    let context = egui::Context::default();
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 640.0));
    let mut state = FieldState::default();
    let mut button_rect = egui::Rect::NOTHING;
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                button_rect = confirmed_target_search_tab_ui_with_worker_running(
                    ui,
                    &mut state,
                    &Texts::new(crate::ui::i18n::Language::Japanese),
                    false,
                    false,
                    false,
                    true,
                )
                .search_button_rect;
            });
        },
    );
    assert!(text_shape_rect(&output, "キャンセル").is_positive());

    let mut cancel_requested = false;
    for pressed in [true, false] {
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(screen_rect),
                events: vec![
                    egui::Event::PointerMoved(button_rect.center()),
                    egui::Event::PointerButton {
                        pos: button_rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::default(),
                    },
                ],
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    cancel_requested |= confirmed_target_search_tab_ui_with_worker_running(
                        ui,
                        &mut state,
                        &Texts::new(crate::ui::i18n::Language::Japanese),
                        false,
                        false,
                        false,
                        true,
                    )
                    .cancel_requested;
                });
            },
        );
    }
    assert!(cancel_requested);
}

#[test]
fn frame1_range_filter_uses_two_rows_inside_the_restored_left_column() {
    let context = egui::Context::default();
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 180.0));
    for filter_mode in [0, 1] {
        let mut state = FieldState::default();
        state.timeleap_filter_mode = filter_mode;
        let mut rows = [egui::Rect::NOTHING; 2];
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(screen_rect),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(278.0, 120.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| rows = timeleap_filter_range_ui(ui, &mut state),
                    );
                });
            },
        );
        if filter_mode == 0 {
            assert!(output.shapes.iter().any(|clipped| matches!(
                &clipped.shape,
                egui::Shape::Text(text)
                    if text.galley.job.text == "時間①秒数"
            )));
            assert!(!output.shapes.iter().any(|clipped| matches!(
                &clipped.shape,
                egui::Shape::Text(text) if text.galley.job.text == "秒数"
            )));
        }
        assert_eq!(state.timeleap_filter_mode, filter_mode);
        assert!(rows[0].is_positive() && rows[1].is_positive());
        assert!(rows[1].top() >= rows[0].bottom());
        assert!(rows
            .iter()
            .all(|row| row.left() >= 8.0 && row.right() <= 286.0));
        assert!(rows[1].bottom() < 120.0, "range rows must fit in panel");
    }
}

#[test]
fn shared_source_row_omits_frame1_on_main_but_shows_it_for_target_search() {
    let mut state = FieldState::default();
    state.seed = "12345678".into();
    let context = egui::Context::default();
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 100.0));
    let main_output = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let _ = timeleap_search_metadata_ui(ui, &mut state, false, false, false);
            });
        },
    );
    assert!(main_output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        egui::Shape::Text(text) if text.galley.job.text == "初期SEED:"
    )));
    assert!(!main_output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        egui::Shape::Text(text) if text.galley.job.text == "Frame1:"
    )));

    let target_output = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let _ = timeleap_search_metadata_ui(ui, &mut state, true, false, false);
            });
        },
    );
    assert!(target_output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        egui::Shape::Text(text) if text.galley.job.text == "Frame1:"
    )));
}

#[test]
fn timeleap_source_frames_keep_body_font_without_resizing_inputs() {
    const TEXT_SCALE: f32 = 1.2;
    const STARTING_FRAME: i64 = 50_000_000;
    const ACTUAL_FRAME1: i64 = 49_999_999;
    const INPUT_STARTING_FRAME: &str = "12345678";
    const INPUT_FRAME1: &str = "87654321";

    let context = egui::Context::default();
    let texts = Texts::new(crate::ui::i18n::Language::Japanese);
    let mut main_state = FieldState::empty_timeline_for_test();
    main_state.timeleap_enabled = true;
    main_state.timeleap_stage = crate::application::field::TimeleapStage::Assumed;
    main_state.timeleap_selected_frame = Some(ACTUAL_FRAME1);
    main_state.results[0].start_consumption = STARTING_FRAME;
    main_state.results[0]
        .timeline
        .push(crate::application::field::FieldTimelineRow {
            frame: STARTING_FRAME,
            timeline_frame: 0,
            duration_frames: 0,
        });
    main_state.set_timeleap_timeline_anchor_for_test(
        Some(std::time::Instant::now()),
        Some(STARTING_FRAME),
    );

    let mut metadata_state = FieldState::default();
    metadata_state.initialize_timeleap_source_inputs();
    metadata_state.timeleap_source_starting_frame = INPUT_STARTING_FRAME.into();
    metadata_state.timeleap_source_frame1 = INPUT_FRAME1.into();
    let mut actions = None;
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(TIMELEAP_SEARCH_WINDOW_WIDTH, 180.0),
            )),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let _ = field_timeline_header_ui(ui, &mut main_state, true, TEXT_SCALE, &texts);
                ui.separator();
                actions = Some(timeleap_search_metadata_ui_with_texts(
                    ui,
                    &mut metadata_state,
                    true,
                    false,
                    false,
                    &texts,
                ));
            });
        },
    );

    let main_header_font_size = 14.0 * TEXT_SCALE;
    for rendered_text in [
        texts.starting_frame(STARTING_FRAME),
        format!("Frame1: {ACTUAL_FRAME1}"),
    ] {
        let actual = text_shape_font_size(&output, &rendered_text);
        assert!(
            (actual - main_header_font_size).abs() < f32::EPSILON,
            "Main {rendered_text:?} should use {main_header_font_size:.2}pt font, got {actual:.2}pt"
        );
    }

    let metadata_font_size = text_shape_font_size(&output, &texts.ui_label("初期SEED:"));
    assert!(metadata_font_size > 0.0);
    for rendered_text in [
        texts.ui_label("初期SEED:").to_owned(),
        "StartingFrame:".to_owned(),
        INPUT_STARTING_FRAME.to_owned(),
        texts.ui_label("NPC数:").to_owned(),
        "Frame1:".to_owned(),
        INPUT_FRAME1.to_owned(),
        "F".to_owned(),
    ] {
        let actual = text_shape_font_size(&output, &rendered_text);
        assert!(
            (actual - metadata_font_size).abs() < f32::EPSILON,
            "Timeleap metadata {rendered_text:?} should use the surrounding body font ({metadata_font_size:.2}pt), got {actual:.2}pt"
        );
    }

    let actions = actions.expect("metadata row should be laid out");
    let starting_frame_rect = actions
        .starting_frame_rect
        .expect("StartingFrame input should be rendered");
    let frame1_rect = actions
        .frame1_rect
        .expect("Frame1 input should be rendered");
    let reset_rect = actions.reset_button_rect.expect("Reset should be rendered");
    assert!((starting_frame_rect.width() - 90.0).abs() < 0.5);
    assert!((frame1_rect.width() - 72.0).abs() < 0.5);
    assert!(
        frame1_rect.right() <= reset_rect.left(),
        "metadata inputs must not overlap Reset: Frame1={frame1_rect:?}, Reset={reset_rect:?}"
    );
}

#[test]
fn timeleap_search_tabs_have_the_requested_order_and_labels() {
    let context = egui::Context::default();
    let mut selected_tab = TIMELEAP_TAB_FRAME1_SEARCH;
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 80.0));
    let mut tab_rects = None;
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                tab_rects = Some(timeleap_search_tabs_ui(ui, &mut selected_tab));
            });
        },
    );
    let tab_rects = tab_rects.expect("all four tab hit targets should be rendered");
    let mut labels = output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => Some((
                text.pos.x + text.galley.rect.center().x,
                text.galley.job.text.to_string(),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    labels.sort_by(|left, right| left.0.total_cmp(&right.0));
    assert_eq!(
        labels
            .into_iter()
            .map(|(_, label)| label)
            .collect::<Vec<_>>(),
        ["Frame1検索", "TargetFrame検索", "配達員", "設定"]
    );
    assert_eq!(selected_tab, TIMELEAP_TAB_FRAME1_SEARCH);

    for (tab, rect) in [
        (TIMELEAP_TAB_FRAME1_SEARCH, tab_rects[0]),
        (TIMELEAP_TAB_TARGET_SEARCH, tab_rects[1]),
        (TIMELEAP_TAB_COURIER, tab_rects[2]),
        (TIMELEAP_TAB_SETTINGS, tab_rects[3]),
    ] {
        let position = rect.center();
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(screen_rect),
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::default(),
                    },
                ],
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    timeleap_search_tabs_ui(ui, &mut selected_tab);
                });
            },
        );
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(screen_rect),
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::default(),
                    },
                ],
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    timeleap_search_tabs_ui(ui, &mut selected_tab);
                });
            },
        );
        assert_eq!(selected_tab, tab, "clicking tab {tab} should select it");
    }
}

#[test]
fn timeleap_search_tabs_and_selected_content_stay_at_the_top_of_a_tall_viewport() {
    let context = egui::Context::default();
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 570.0));

    for tab in [
        TIMELEAP_TAB_FRAME1_SEARCH,
        TIMELEAP_TAB_TARGET_SEARCH,
        TIMELEAP_TAB_COURIER,
        TIMELEAP_TAB_SETTINGS,
    ] {
        let mut selected_tab = tab;
        let mut tab_rects = None;
        let mut content_rect = egui::Rect::NOTHING;
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(screen_rect),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    tab_rects = Some(timeleap_search_tabs_ui(ui, &mut selected_tab));
                    ui.separator();
                    content_rect = ui.label(format!("tab body {tab}")).rect;
                });
            },
        );

        let tab_rects = tab_rects.expect("tab row should be rendered");
        assert!(
            tab_rects.iter().all(|rect| rect.top() < 50.0),
            "tab row should stay at the top for selected tab {tab}: {tab_rects:?}"
        );
        assert!(
                content_rect.top() < 90.0,
                "selected tab content should follow the tab row near the top for tab {tab}: {content_rect:?}"
            );
    }
}

#[test]
fn main_candidate_columns_fit_and_align_inside_the_table() {
    let context = egui::Context::default();
    let mut header_rects = Vec::new();
    let mut cell_rects = Vec::new();
    let mut table_right = 0.0;
    let mut table_left = 0.0;
    let mut max_time_text_width = 0.0_f32;
    let mut render_output = None;
    for _ in 0..3 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(720.0, 320.0),
            )),
            ..egui::RawInput::default()
        };
        render_output = Some(context.run(input, |context| {
            egui::CentralPanel::default().show(context, |ui| {
                table_right = ui.max_rect().right();
                table_left = ui.cursor().left();
                header_rects = timeleap_main_candidate_header_ui(ui);
                let row = TimeleapMainCandidateRow {
                    values: [
                        "100000".to_owned(),
                        "24".to_owned(),
                        "110369".to_owned(),
                        "18".to_owned(),
                        "23:59:59".to_owned(),
                        "01:23:45".to_owned(),
                    ],
                    frame1: 100_000,
                    target_frame: 110_369,
                    frame1_group_tick_start: 10,
                    frame1_group_tick_end: 10,
                };
                cell_rects = timeleap_main_candidate_rows_ui(ui, &[row], true, &[Some(20)]).0;
                max_time_text_width = ui.fonts(|fonts| {
                    fonts
                        .layout_no_wrap(
                            "23:59:59".to_owned(),
                            egui::FontId::proportional(14.0),
                            egui::Color32::BLACK,
                        )
                        .size()
                        .x
                });
            });
        }));
    }

    assert_eq!(header_rects.len(), 18);
    assert_eq!(cell_rects.len(), 6);
    assert!((cell_rects[0].left() - table_left).abs() <= 0.5);
    assert!(header_rects
        .iter()
        .all(|rect| rect.right() <= table_right + 0.5));
    assert!(cell_rects
        .iter()
        .all(|rect| rect.right() <= table_right + 0.5));
    assert!(TIMELEAP_FRAME1_CANDIDATE_COLUMN_WIDTHS[4] >= max_time_text_width + 8.0);
    assert!(TIMELEAP_FRAME1_CANDIDATE_COLUMN_WIDTHS[5] >= max_time_text_width + 8.0);
    for (header_cell, cell) in header_rects.iter().step_by(3).zip(&cell_rects) {
        assert!((header_cell.left() - cell.left()).abs() <= 0.5);
    }
    let output = render_output.expect("candidate table should render");
    for (cell, value) in cell_rects
        .iter()
        .zip(["100000", "24", "110369", "18", "23:59:59", "01:23:45"])
    {
        let text_rect = text_shape_rect(&output, value);
        assert!(
            (text_rect.center().x - cell.center().x).abs() <= 0.5,
            "candidate value {value} should be centered in {cell:?}, got {text_rect:?}"
        );
    }
}

#[test]
fn timeleap_duration_display_is_zero_padded_and_saturates_without_wrapping() {
    for (seconds, expected) in [
        (0, "00:00:00"),
        (9, "00:00:09"),
        (59, "00:00:59"),
        (60, "00:01:00"),
        (3599, "00:59:59"),
        (3600, "01:00:00"),
        (86_398, "23:59:58"),
        (86_399, "23:59:59"),
        (86_400, "23:59:59"),
        (i64::MAX, "23:59:59"),
        (-1, "00:00:00"),
    ] {
        assert_eq!(format_timeleap_duration(seconds), expected);
    }
}

#[test]
fn compact_timeleap_window_keeps_both_search_tables_inside_the_right_column() {
    let context = egui::Context::default();
    let widths = confirmed_target_search_column_widths();
    let confirmed_table_width =
        confirmed_target_search_content_width(&widths) + CONFIRMED_TARGET_SEARCH_OUTER_INSET * 2.0;
    let mut available_right_width = 0.0;
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(TIMELEAP_SEARCH_WINDOW_WIDTH, TIMELEAP_SEARCH_WINDOW_HEIGHT),
            )),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                available_right_width = timeleap_search_column_widths(
                    ui.available_width(),
                    ui.spacing().item_spacing.x,
                )
                .1;
            });
        },
    );

    assert_eq!(TIMELEAP_SEARCH_WINDOW_WIDTH, 884.0);
    assert_eq!(
        TIMELEAP_SEARCH_WINDOW_WIDTH,
        920.0 - TIMELEAP_SEARCH_WINDOW_RESET_BUTTON_WIDTH_REDUCTION
    );
    assert!(available_right_width >= timeleap_main_candidate_table_width());
    assert!(available_right_width >= confirmed_table_width);
}

#[test]
fn timeleap_window_width_reduction_is_about_ninety_percent_of_reset_button() {
    let context = egui::Context::default();
    let mut state = FieldState::default();
    state.seed = "12345678".into();
    let mut reset_button_rect = None;
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(TIMELEAP_SEARCH_WINDOW_WIDTH, 100.0),
            )),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                reset_button_rect =
                    timeleap_search_metadata_ui(ui, &mut state, false, false, false)
                        .reset_button_rect;
            });
        },
    );

    let reset_button_rect = reset_button_rect.expect("Reset button should render");
    let actual_reduction = 920.0 - TIMELEAP_SEARCH_WINDOW_WIDTH;
    let reset_based_reduction = reset_button_rect.width() * 0.9;
    assert!(reset_button_rect.is_positive());
    assert!(
            (actual_reduction - reset_based_reduction).abs() <= 3.0,
            "window shrinks by {actual_reduction:.1}pt, but 90% of Reset width is {reset_based_reduction:.1}pt (Reset rect: {reset_button_rect:?})"
        );
    assert!(reset_button_rect.right() <= TIMELEAP_SEARCH_WINDOW_WIDTH);
}
