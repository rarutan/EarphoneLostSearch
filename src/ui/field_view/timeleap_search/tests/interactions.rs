use super::*;

#[test]
fn target_search_selection_controls_share_field_state_and_preview_path() {
    let mut state = FieldState::default();
    let context = egui::Context::default();
    let mut rendered = false;
    let _ = context.run(egui::RawInput::default(), |context| {
        egui::CentralPanel::default().show(context, |ui| {
            let width = ui.available_width();
            rendered = !timeleap_category_pokemon_method_controls(
                ui,
                &mut state,
                &Texts::new(crate::ui::i18n::Language::Japanese),
                "test",
                width,
                None,
            )
            .preview_requested;
        });
    });
    assert!(rendered);
    assert_eq!(state.timeleap_search_category, 0);
    assert_eq!(state.timeleap_search_preset, 0);

    state.timeleap_enabled = true;
    let courier_category = crate::domain::fixed_rng_catalog::CATEGORIES.len();
    state.timeleap_search_category = courier_category;
    assert!(apply_timeleap_category_selection(
        &mut state,
        courier_category
    ));
    assert_eq!(state.timeleap_mode, TimeleapMode::Courier);

    state.timeleap_search_category = 0;
    state.timeleap_search_preset = 1;
    assert!(apply_timeleap_category_selection(
        &mut state,
        courier_category
    ));
    assert_eq!(state.timeleap_search_preset, 0);
    assert!(apply_timeleap_preset_selection(&mut state));

    assert!(apply_timeleap_method_selection(
        &mut state,
        crate::application::field::TimeleapMethodSelection::MenuGirl,
    ));
    assert_eq!(
        (state.timeleap_mode, state.timeleap_is_boy),
        (TimeleapMode::Menu, false)
    );
}

#[test]
fn method_selector_remains_interactive_after_timeleap_is_locked() {
    let context = egui::Context::default();
    crate::ui::widgets::install_app_style(&context, crate::ui::widgets::ColorScheme::default());
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0));
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_locked = true;
    state.timeleap_stage = crate::application::field::TimeleapStage::Succeeded;
    let texts = Texts::new(crate::ui::i18n::Language::Japanese);
    let render = |context: &egui::Context, state: &mut FieldState, input: egui::RawInput| {
        context.run(input, |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let width = ui.available_width();
                let _ = timeleap_category_pokemon_method_controls(
                    ui,
                    state,
                    &texts,
                    "locked-method-selector-test",
                    width,
                    None,
                );
            });
        })
    };
    let initial = render(
        &context,
        &mut state,
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..egui::RawInput::default()
        },
    );
    let combo_rect = {
        let mut controls = None;
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(screen_rect),
                ..egui::RawInput::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    controls = Some(timeleap_category_pokemon_method_controls(
                        ui,
                        &mut state,
                        &texts,
                        "locked-method-selector-test",
                        ui.available_width(),
                        None,
                    ));
                });
            },
        );
        controls.expect("search controls render").combo_rects[2]
    };
    assert!(combo_rect.is_positive());
    let _ = initial;

    for pressed in [true, false] {
        let _ = render(
            &context,
            &mut state,
            egui::RawInput {
                screen_rect: Some(screen_rect),
                events: vec![
                    egui::Event::PointerMoved(combo_rect.center()),
                    egui::Event::PointerButton {
                        pos: combo_rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::default(),
                    },
                ],
                ..egui::RawInput::default()
            },
        );
    }
    let menu_output = render(
        &context,
        &mut state,
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..egui::RawInput::default()
        },
    );
    let option_rect = text_shape_rect(&menu_output, "メニュー閉じ（女の子）");
    assert!(
        option_rect.is_positive(),
        "method options should open while locked"
    );
    for pressed in [true, false] {
        let _ = render(
            &context,
            &mut state,
            egui::RawInput {
                screen_rect: Some(screen_rect),
                events: vec![
                    egui::Event::PointerMoved(option_rect.center()),
                    egui::Event::PointerButton {
                        pos: option_rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::default(),
                    },
                ],
                ..egui::RawInput::default()
            },
        );
    }

    assert_eq!(
        (state.timeleap_mode, state.timeleap_is_boy),
        (TimeleapMode::Menu, false)
    );
    assert!(
        state.timeleap_locked,
        "editing search conditions must not unlock execution"
    );
    assert_eq!(
        state.timeleap_stage,
        crate::application::field::TimeleapStage::Succeeded
    );
    assert!(
        !state.timeleap_frame1_preview_is_pending(),
        "editing search conditions must not invalidate the completed Timeleap preview"
    );
}

#[test]
fn delay_and_no_dex_search_options_remain_editable_without_resetting_locked_execution() {
    let context = egui::Context::default();
    crate::ui::widgets::install_app_style(&context, crate::ui::widgets::ColorScheme::default());
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 300.0));
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_locked = true;
    state.timeleap_stage = crate::application::field::TimeleapStage::Succeeded;
    state.timeleap_mode = TimeleapMode::Courier;
    state.timeleap_search_category = crate::domain::fixed_rng_catalog::CATEGORIES.len();
    state.timeleap_consider_delay_enabled = false;

    let mut initial_actions = None;
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                initial_actions = Some(timeleap_search_options_ui(
                    ui,
                    &mut state,
                    &Texts::new(crate::ui::i18n::Language::Japanese),
                ));
            });
        },
    );
    let initial_actions = initial_actions.expect("search options render");
    let delay_rect = initial_actions
        .consider_delay_rect
        .expect("Consider Delay checkbox has a response rectangle");
    let no_dex_rect = initial_actions
        .no_dex_rect
        .expect("No Dex checkbox has a response rectangle");
    assert!(delay_rect.is_positive() && no_dex_rect.is_positive());

    let mut preview_requested = false;
    for position in [delay_rect.center(), no_dex_rect.center()] {
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
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::default(),
                    },
                ],
                ..egui::RawInput::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    preview_requested = timeleap_search_options_ui(
                        ui,
                        &mut state,
                        &Texts::new(crate::ui::i18n::Language::Japanese),
                    )
                    .preview_requested;
                });
            },
        );
    }

    assert!(state.timeleap_consider_delay_enabled);
    assert!(state.timeleap_no_dex);
    assert!(state.timeleap_locked);
    assert_eq!(
        state.timeleap_stage,
        crate::application::field::TimeleapStage::Succeeded
    );
    assert!(!preview_requested);
    assert!(!state.timeleap_frame1_preview_is_pending());
}

#[test]
fn target_search_selection_controls_match_search_button_even_for_long_labels() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_locked = true;
    state.timeleap_stage = crate::application::field::TimeleapStage::Succeeded;
    let mut longest_preset = (0, 0, 0);
    let japanese = Texts::new(crate::ui::i18n::Language::Japanese);
    for (category_index, category) in crate::domain::fixed_rng_catalog::CATEGORIES
        .iter()
        .enumerate()
    {
        for (preset_index, preset) in category.presets.iter().enumerate() {
            let label_length = japanese.fixed_rng_preset_name(preset).chars().count();
            if label_length > longest_preset.2 {
                longest_preset = (category_index, preset_index, label_length);
            }
        }
    }
    state.timeleap_search_category = longest_preset.0;
    state.timeleap_search_preset = longest_preset.1;
    state.timeleap_mode = TimeleapMode::Dialogue;
    state.timeleap_is_boy = false;
    let context = egui::Context::default();
    crate::ui::widgets::install_app_style(&context, crate::ui::widgets::ColorScheme::default());
    let mut actions = None;
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(TIMELEAP_SEARCH_WINDOW_WIDTH, TIMELEAP_SEARCH_WINDOW_HEIGHT),
            )),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                actions = Some(confirmed_target_search_tab_ui(
                    ui,
                    &mut state,
                    &Texts::new(crate::ui::i18n::Language::Japanese),
                    false,
                    false,
                    false,
                ));
            });
        },
    );

    assert!(text_shape_rect(&output, "Consider Delay").is_positive());
    assert!(text_shape_rect(&output, "No Dex").is_positive());
    assert!(!output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        egui::Shape::Text(text) if text.galley.job.text == "tick"
    )));
    assert!(!output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        egui::Shape::Text(text) if text.galley.job.text == "検索条件"
    )));

    let actions = actions.expect("TargetFrame search tab should render");
    assert_eq!(actions.selection_combo_rects.len(), 3);
    let search_button = actions.search_button_rect;
    assert!(search_button.is_positive());
    for rect in &actions.selection_combo_rects {
        let painted = painted_rect_at_center(&output, rect.center());
        assert!(
                (rect.width() - search_button.width()).abs() < 1.0
                    && (rect.left() - search_button.left()).abs() < 1.0
                    && (rect.right() - search_button.right()).abs() < 1.0
                    && (painted.left() - search_button.left()).abs() < 1.0
                    && (painted.right() - search_button.right()).abs() < 1.0,
                "selector's painted bounds should match search button: response={rect:?}, painted={painted:?}, search={search_button:?}"
            );
    }
    let painted_search_button = painted_rect_at_center(&output, search_button.center());
    assert!(
            (painted_search_button.left() - search_button.left()).abs() < 1.0
                && (painted_search_button.right() - search_button.right()).abs() < 1.0,
            "search button paint should match its response: painted={painted_search_button:?}, response={search_button:?}"
        );
    assert!(
            (actions.selection_separator_rect.width() - search_button.width()).abs() < 1.0
                && (actions.selection_separator_rect.left() - search_button.left()).abs() < 1.0
                && (actions.selection_separator_rect.right() - search_button.right()).abs() < 1.0,
            "separator should match actual search button edges: separator={:?}, search={search_button:?}",
            actions.selection_separator_rect,
        );
    assert!(
        (actions.search_button_rect.left() - search_button.left()).abs() < 1.0,
        "search button keeps its historical left anchor: search={:?}, reference={search_button:?}",
        actions.search_button_rect,
    );
    let search_text = output
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.job.text == "検索" => Some(text),
            _ => None,
        })
        .expect("centered search button label should be rendered");
    let text_center_x = search_text.pos.x + search_text.galley.rect.center().x;
    assert!(
            (text_center_x - actions.search_button_rect.center().x).abs() < 1.0,
            "search label remains centered without moving/resizing the button: label_x={text_center_x}, button={:?}",
            actions.search_button_rect
        );
}

#[test]
fn frame1_search_selection_controls_match_search_button_width() {
    let context = egui::Context::default();
    crate::ui::widgets::install_app_style(&context, crate::ui::widgets::ColorScheme::default());
    let mut rectangles = Vec::new();
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    let texts = Texts::new(crate::ui::i18n::Language::Japanese);
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(TIMELEAP_SEARCH_WINDOW_WIDTH, TIMELEAP_SEARCH_WINDOW_HEIGHT),
            )),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let (column_width, _) = timeleap_search_column_widths(
                    ui.available_width(),
                    ui.spacing().item_spacing.x,
                );
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(column_width, 500.0),
                        egui::Layout::top_down_justified(egui::Align::Min),
                        |ui| {
                            ui.set_min_width(column_width);
                            ui.set_max_width(column_width);
                            let controls = timeleap_category_pokemon_method_controls(
                                ui,
                                &mut state,
                                &texts,
                                "frame1-search-test",
                                column_width,
                                Some((420.0, 460.0)),
                            );
                            rectangles.extend(controls.combo_rects);
                            ui.horizontal(|ui| {
                                ui.checkbox(
                                    &mut state.timeleap_consider_delay_enabled,
                                    "Consider Delay",
                                );
                                ui.add_enabled_ui(state.timeleap_consider_delay_enabled, |ui| {
                                    bounded_integer_text_edit(
                                        ui,
                                        &mut state.timeleap_consider_delay_frames,
                                        46.0,
                                        0,
                                        9998,
                                    );
                                    ui.label("F +4");
                                });
                            });
                            ui.add_enabled_ui(state.timeleap_uses_courier_method(), |ui| {
                                ui.checkbox(&mut state.timeleap_no_dex, "No Dex");
                            });
                            rectangles.push(ui.separator().rect);
                            ui.allocate_ui_with_layout(
                                egui::vec2(column_width, ui.spacing().interact_size.y),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.set_min_width(column_width);
                                    ui.set_max_width(column_width);
                                    let mut generation_widgets = egui::Rect::NOTHING;
                                    generation_widgets =
                                        generation_widgets.union(ui.label("生成開始Frame").rect);
                                    generation_widgets = generation_widgets.union(
                                        numeric_text_edit(
                                            ui,
                                            &mut "10000".to_owned(),
                                            103.0,
                                            false,
                                            false,
                                        )
                                        .rect,
                                    );
                                    generation_widgets =
                                        generation_widgets.union(ui.label("F").rect);
                                    generation_widgets = generation_widgets.union(
                                        help_button(ui, "frame1-generation-test", "説明").rect,
                                    );
                                    rectangles.insert(0, generation_widgets);
                                },
                            );
                            rectangles.push(centered_search_button(ui, true, column_width).rect);
                        },
                    );
                });
            });
        },
    );

    assert_eq!(rectangles.len(), 6);
    let search_button = rectangles[5];
    for (index, rect) in rectangles[1..4].iter().enumerate() {
        let painted = painted_rect_at_center(&output, rect.center());
        assert!(
                (rect.width() - search_button.width()).abs() < 1.0
                    && (rect.left() - search_button.left()).abs() < 1.0
                    && (rect.right() - search_button.right()).abs() < 1.0
                    && (painted.left() - search_button.left()).abs() < 1.0
                    && (painted.right() - search_button.right()).abs() < 1.0,
                "Frame1 selector/separator #{index} paint must match the actual search button bounds: response={rect:?}, painted={painted:?}, search={search_button:?}"
            );
    }
    assert!(
        search_button.width() > 0.0,
        "search button must keep its existing nonzero geometry: {search_button:?}"
    );
}

#[test]
fn target_search_geometry_matches_frame1_search() {
    fn painted_separator_y(output: &egui::FullOutput, response: egui::Rect) -> f32 {
        output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::LineSegment { points, .. }
                    if (points[0].y - points[1].y).abs() < 0.1
                        && (points[1].x - points[0].x).abs() >= response.width() - 2.0
                        && points[0].y >= response.top() - 2.0
                        && points[0].y <= response.bottom() + 2.0 =>
                {
                    Some(points[0].y)
                }
                _ => None,
            })
            .expect("separator should emit a horizontal painter line")
    }

    let screen_rect = egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(TIMELEAP_SEARCH_WINDOW_WIDTH, TIMELEAP_SEARCH_WINDOW_HEIGHT),
    );
    let texts = Texts::new(crate::ui::i18n::Language::Japanese);

    let frame1_context = egui::Context::default();
    crate::ui::widgets::install_app_style(
        &frame1_context,
        crate::ui::widgets::ColorScheme::default(),
    );
    let mut frame1_state = FieldState::default();
    frame1_state.timeleap_enabled = true;
    frame1_state.timeleap_locked = true;
    frame1_state.timeleap_stage = crate::application::field::TimeleapStage::Succeeded;
    frame1_state.timeleap_generation_start_frame = "70000".to_owned();
    let mut frame1_separator = egui::Rect::NOTHING;
    let mut frame1_generation_start_row_rects = [egui::Rect::NOTHING; 4];
    let frame1_output = frame1_context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let _ = timeleap_search_metadata_ui(ui, &mut frame1_state, false, false, false);
                ui.separator();
                let (left_width, _) = timeleap_search_column_widths(
                    ui.available_width(),
                    ui.spacing().item_spacing.x,
                );
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(left_width, ui.available_height()),
                        egui::Layout::top_down_justified(egui::Align::Min),
                        |ui| {
                            ui.set_min_width(left_width);
                            ui.set_max_width(left_width);
                            let _ = timeleap_category_pokemon_method_controls(
                                ui,
                                &mut frame1_state,
                                &texts,
                                "frame1-separator-position-test",
                                left_width,
                                Some((420.0, 460.0)),
                            );
                            ui.horizontal(|ui| {
                                ui.checkbox(
                                    &mut frame1_state.timeleap_consider_delay_enabled,
                                    "Consider Delay",
                                );
                                ui.add_enabled_ui(
                                    frame1_state.timeleap_consider_delay_enabled,
                                    |ui| {
                                        bounded_integer_text_edit(
                                            ui,
                                            &mut frame1_state.timeleap_consider_delay_frames,
                                            46.0,
                                            0,
                                            9998,
                                        );
                                        ui.label("F +4");
                                    },
                                );
                            });
                            ui.add_enabled_ui(frame1_state.timeleap_uses_courier_method(), |ui| {
                                ui.checkbox(&mut frame1_state.timeleap_no_dex, "No Dex");
                            });
                            frame1_separator = ui.separator().rect;
                            frame1_generation_start_row_rects = timeleap_generation_start_frame_row(
                                ui,
                                left_width,
                                "frame1-generation-start-alignment-test",
                                |ui| {
                                    numeric_text_edit(
                                        ui,
                                        &mut frame1_state.timeleap_generation_start_frame,
                                        103.0,
                                        false,
                                        false,
                                    )
                                },
                            );
                        },
                    );
                });
            });
        },
    );

    let target_context = egui::Context::default();
    crate::ui::widgets::install_app_style(
        &target_context,
        crate::ui::widgets::ColorScheme::default(),
    );
    let mut target_state = FieldState::default();
    target_state.timeleap_enabled = true;
    target_state.timeleap_locked = true;
    target_state.timeleap_stage = crate::application::field::TimeleapStage::Succeeded;
    target_state.confirmed_target_search_form.initialized = true;
    target_state.timeleap_generation_start_frame = "70000".to_owned();
    let mut target_separator = egui::Rect::NOTHING;
    let mut target_generation_start_row_rects = [egui::Rect::NOTHING; 4];
    let target_output = target_context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let actions = confirmed_target_search_tab_ui(
                    ui,
                    &mut target_state,
                    &texts,
                    false,
                    false,
                    false,
                );
                target_separator = actions.selection_separator_rect;
                target_generation_start_row_rects = actions.generation_start_row_rects;
            });
        },
    );

    let frame1_line_y = painted_separator_y(&frame1_output, frame1_separator);
    let target_line_y = painted_separator_y(&target_output, target_separator);
    assert!(
            (frame1_line_y - target_line_y).abs() < 0.5,
            "Frame1 and TargetFrame separator paint should share a y position: Frame1={frame1_line_y}, TargetFrame={target_line_y}"
        );
    for (index, (frame1, target)) in frame1_generation_start_row_rects
        .iter()
        .zip(target_generation_start_row_rects.iter())
        .enumerate()
    {
        assert!(
                (frame1.left() - target.left()).abs() < 0.5
                    && (frame1.top() - target.top()).abs() < 0.5
                    && (frame1.width() - target.width()).abs() < 0.5
                    && (frame1.height() - target.height()).abs() < 0.5,
                "generation-start widget #{index} should have matching geometry: Frame1={frame1:?}, TargetFrame={target:?}"
            );
    }
    let frame1_value = text_shape_rect(&frame1_output, "70000");
    let target_value = text_shape_rect(&target_output, "70000");
    assert!(
            (frame1_value.left() - target_value.left()).abs() < 0.5
                && (frame1_value.right() - target_value.right()).abs() < 0.5,
            "generation-start numeric text should share horizontal alignment: Frame1={frame1_value:?}, TargetFrame={target_value:?}"
        );
}
