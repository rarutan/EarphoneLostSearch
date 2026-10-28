use super::*;

#[test]
fn main_candidate_rows_draw_group_boundary_as_a_bottom_border() {
    let context = egui::Context::default();
    let mut rows = Vec::new();
    for (frame1, group_start, group_end) in [(0, 0, 1), (2, 0, 1), (10, 5, 6), (12, 5, 6)] {
        rows.push(TimeleapMainCandidateRow {
            values: [
                frame1.to_string(),
                "4".to_owned(),
                "100".to_owned(),
                "4".to_owned(),
                "1".to_owned(),
                "2".to_owned(),
            ],
            frame1,
            target_frame: 100,
            frame1_group_tick_start: group_start,
            frame1_group_tick_end: group_end,
        });
    }

    let mut cell_rects = Vec::new();
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(720.0, 320.0),
            )),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                cell_rects = timeleap_main_candidate_rows_ui(ui, &rows, true, &[Some(20); 3]).0;
            });
        },
    );

    assert_eq!(
        rows.iter()
            .map(|row| row.values[0].as_str())
            .collect::<Vec<_>>(),
        ["0", "2", "10", "12"]
    );
    assert!(rows.iter().all(|row| row.values[1] == "4"));
    assert_eq!(cell_rects.len(), rows.len() * 6);

    let separator_lines = output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::LineSegment { points, .. }
                if (points[0].y - points[1].y).abs() < 0.1
                    && (points[1].x - points[0].x - timeleap_main_candidate_table_width())
                        .abs()
                        < 1.0 =>
            {
                Some((points[0].y, points[0].x, points[1].x))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(separator_lines.len(), 1);
    let (line_y, line_left, line_right) = separator_lines[0];
    assert!(
        (line_y - cell_rects[6].bottom()).abs() < 0.5,
        "line {line_y} must align with previous row bottom {}",
        cell_rects[6].bottom()
    );
    assert!(
        cell_rects[12].top() - line_y <= context.style().spacing.item_spacing.y + 0.5,
        "bottom border should stay within normal row spacing: line={line_y}, next row={}",
        cell_rects[12].top()
    );
    assert!((line_left - cell_rects[0].left()).abs() < 1.0);
    assert!((line_right - cell_rects[5].right()).abs() < 1.0);
}

#[test]
fn candidate_row_offset_is_not_calculated_until_a_context_menu_opens() {
    let context = egui::Context::default();
    let row = TimeleapMainCandidateRow {
        values: [
            "100".into(),
            "2".into(),
            "200".into(),
            "2".into(),
            "00:00:01".into(),
            "00:00:02".into(),
        ],
        frame1: 100,
        target_frame: 200,
        frame1_group_tick_start: 50,
        frame1_group_tick_end: 50,
    };
    let offset_calculations = std::cell::Cell::new(0);
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(720.0, 160.0),
            )),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                timeleap_main_candidate_rows_ui_at(
                    ui,
                    std::slice::from_ref(&row),
                    0,
                    None,
                    true,
                    |_, _| {
                        offset_calculations.set(offset_calculations.get() + 1);
                        Some(0)
                    },
                );
            });
        },
    );

    assert_eq!(offset_calculations.get(), 0);
}

#[test]
fn large_candidate_table_layouts_only_rows_in_the_visible_range() {
    let context = egui::Context::default();
    let total_rows = 10_000;
    let mut rendered_rows = 0;
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(720.0, 220.0),
            )),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                timeleap_candidate_rows_virtualized_ui(ui, total_rows, |ui, visible| {
                    rendered_rows += visible.len();
                    for _ in visible {
                        ui.allocate_exact_size(
                            egui::vec2(500.0, TIMELEAP_SEARCH_TABLE_ROW_HEIGHT),
                            egui::Sense::hover(),
                        );
                    }
                });
            });
        },
    );

    assert!(rendered_rows > 0);
    assert!(
        rendered_rows < total_rows / 10,
        "rendered {rendered_rows} of {total_rows} rows"
    );
}

#[test]
fn main_timeleap_controls_use_full_row_with_actions_right_aligned() {
    let context = egui::Context::default();
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_frame1_reachable = Some(true);
    for width in [1_052.0, 1_250.0, 1_600.0] {
        let mut layout = TimeleapMainControlsLayout::default();
        let mut outer = egui::Rect::NOTHING;
        let mut inner = egui::Rect::NOTHING;
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 700.0),
                )),
                ..egui::RawInput::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    let parent_rect = ui.max_rect();
                    let (_, right_bottom_rect) =
                        split_panel_rects(parent_rect, LEFT_PANEL_WIDTH, PANEL_GAP);
                    outer = right_bottom_rect;
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(outer), |ui| {
                        fixed_group_panel(ui, |ui| {
                            inner = ui.max_rect();
                            let panel_width = ui.max_rect().width();
                            layout = timeleap_main_controls_ui(
                                ui,
                                &mut state,
                                30.0,
                                panel_width,
                                ColorScheme::Standard,
                            )
                            .layout;
                        });
                    });
                });
            },
        );

        assert!(layout.row.left() >= inner.left() - 0.5);
        assert!(layout.row.right() <= inner.right() + 0.5);
        assert!(inner.right() < outer.right());
        for rect in [layout.frame1, layout.frame1_indicator_row, layout.search] {
            assert!(!rect.is_negative());
            assert!(rect.left() >= layout.row.left() - 0.5);
            assert!(rect.right() <= layout.row.right() + 0.5);
            assert!((rect.center().y - layout.row.center().y).abs() < 1.0);
        }
        assert!(
            (layout.timeleap_checkbox.left() - layout.row.left() - 4.0).abs() <= 0.5,
            "Timeleap checkbox {:?} should keep a 4px left inset from row {:?}",
            layout.timeleap_checkbox,
            layout.row
        );
        assert!(
            (layout.method_label.left() - layout.row.left() - 4.0).abs() <= 0.5,
            "Method label {:?} should align with the checkbox's 4px left inset from row {:?}",
            layout.method_label,
            layout.row
        );
        for rect in [layout.method, layout.no_dex] {
            assert!(!rect.is_negative());
            assert!(rect.left() >= inner.left() - 0.5);
            assert!(rect.right() <= inner.right() + 0.5);
            assert!(rect.top() > layout.row.bottom());
        }
        assert!(layout.method.width() <= layout.row.width() * 0.55);
        assert!(layout.no_dex.left() >= layout.method.right());
        assert!(
            layout.search.width() >= 30.0,
            "Search rect: {:?}",
            layout.search
        );
        assert!(layout.row.right() - layout.search.right() >= 3.5);
    }
}

#[test]
fn enabling_timeleap_immediately_selects_frame1_timer_before_ready() {
    let context = egui::Context::default();
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1_052.0, 220.0));
    let mut state = FieldState::default();
    let mut layout = TimeleapMainControlsLayout::default();
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                layout = timeleap_main_controls_ui(
                    ui,
                    &mut state,
                    30.0,
                    ui.available_width(),
                    ColorScheme::Standard,
                )
                .layout;
            });
        },
    );
    assert_eq!(state.timer_kind(), FieldTimerKind::NextBlink);
    let click = layout.timeleap_checkbox.center();
    let mut actions = TimeleapMainControlsActions::default();
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            events: vec![
                egui::Event::PointerMoved(click),
                egui::Event::PointerButton {
                    pos: click,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
                egui::Event::PointerButton {
                    pos: click,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                },
            ],
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                actions = timeleap_main_controls_ui(
                    ui,
                    &mut state,
                    30.0,
                    ui.available_width(),
                    ColorScheme::Standard,
                );
            });
        },
    );

    assert!(actions.enabled_changed);
    assert!(actions.frame1_preview_requested);
    assert!(state.timeleap_enabled);
    assert_eq!(state.timer_kind(), FieldTimerKind::Target);
}

#[test]
fn frame1_table_renders_countdown_before_ready() {
    let context = egui::Context::default();
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1_052.0, 600.0));
    let texts = Texts::new(crate::ui::i18n::Language::Japanese);
    let mut state = FieldState::empty_timeline_for_test();
    state.observed = vec![std::time::Instant::now()];
    state.timeleap_enabled = true;
    assert_eq!(state.timer_kind(), FieldTimerKind::NextBlink);
    state.timeleap_frame1_input = "20".into();
    state.results[0].timeline = vec![
        crate::application::field::FieldTimelineRow {
            frame: 10,
            timeline_frame: 0,
            duration_frames: 10,
        },
        crate::application::field::FieldTimelineRow {
            frame: 20,
            timeline_frame: 10,
            duration_frames: 10,
        },
        crate::application::field::FieldTimelineRow {
            frame: 30,
            timeline_frame: 20,
            duration_frames: 0,
        },
    ];

    assert_eq!(
        state.timeleap_lifecycle_policy().table_mode,
        crate::application::field::TimeleapTableMode::Frame1
    );
    assert_eq!(
        state.results[0]
            .blinks_through_frame_after_row(0, 20)
            .expect("Frame1 is within the visible table"),
        1
    );

    let output = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                field_timeline_ui(
                    ui,
                    &mut state,
                    false,
                    30.0,
                    1.0,
                    ColorScheme::Standard,
                    &texts,
                );
            });
        },
    );

    assert!(text_shape_rect(&output, texts.timeline_frame1_blink_count()).is_positive());
    assert!(
        text_shape_rect(&output, "1").is_positive(),
        "the first row should show the one remaining blink to Frame1 before Ready"
    );
}

#[test]
fn frame1_reachability_label_is_hidden_without_a_visible_result() {
    for (enabled, reachable) in [(false, None), (true, None), (false, Some(true))] {
        let context = egui::Context::default();
        let mut state = FieldState::default();
        state.timeleap_enabled = enabled;
        state.timeleap_frame1_reachable = reachable;
        let mut frame1_input = egui::Rect::NOTHING;
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1_052.0, 220.0),
                )),
                ..egui::RawInput::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    frame1_input = timeleap_main_controls_ui(
                        ui,
                        &mut state,
                        30.0,
                        ui.available_width(),
                        ColorScheme::Standard,
                    )
                    .layout
                    .frame1;
                });
            },
        );
        assert!(
            frame1_input.is_positive(),
            "Frame1 input must remain visible"
        );
        for unexpected in ["Frame1", "○", "×"] {
            assert!(
                !output.shapes.iter().any(|clipped| matches!(
                    &clipped.shape,
                    egui::Shape::Text(text) if text.galley.job.text == unexpected
                )),
                "{unexpected} must be hidden when no status is visible"
            );
        }
    }
}

#[test]
fn frame1_reachability_colors_label_and_symbol_together() {
    let context = egui::Context::default();
    let mut rendered = Vec::new();
    for reachable in [true, false] {
        let mut state = FieldState::default();
        state.timeleap_enabled = true;
        state.timeleap_frame1_reachable = Some(reachable);
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1_052.0, 220.0),
                )),
                ..egui::RawInput::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    timeleap_main_controls_ui(
                        ui,
                        &mut state,
                        30.0,
                        ui.available_width(),
                        ColorScheme::Standard,
                    );
                });
            },
        );
        let symbol = if reachable { "○" } else { "×" };
        assert!(text_shape_rect(&output, symbol).is_positive());
        assert_eq!(
            text_shape_color(&output, "Frame1"),
            text_shape_color(&output, symbol),
            "Frame1 label and reachability symbol must share the status color"
        );
        assert!(
            !output.shapes.iter().any(|clipped| matches!(
                &clipped.shape,
                egui::Shape::Text(text)
                    if text.galley.job.text == "Frame1 ○" || text.galley.job.text == "Frame1 ×"
            )),
            "Frame1 reachability must not duplicate the Frame1 label"
        );
        rendered.push(text_shape_color(&output, symbol));
    }
    assert_ne!(
        rendered[0], rendered[1],
        "○ and × must use distinct status colors"
    );
}

#[test]
fn adopted_frame1_appears_next_to_starting_frame_and_can_be_copied_before_recheck() {
    let context = egui::Context::default();
    let texts = Texts::new(crate::ui::i18n::Language::Japanese);
    let mut state = FieldState::empty_timeline_for_test();
    state.timeleap_enabled = true;
    state.timeleap_selected_frame = Some(106040);
    state.observed = vec![std::time::Instant::now()];
    state.results[0].start_consumption = 497;
    state.results[0]
        .timeline
        .push(crate::application::field::FieldTimelineRow {
            frame: 497,
            timeline_frame: 2,
            duration_frames: 0,
        });

    state.timeleap_stage = crate::application::field::TimeleapStage::Assumed;
    let before_confirmation = context.run(egui::RawInput::default(), |context| {
        egui::CentralPanel::default().show(context, |ui| {
            field_timeline_ui(
                ui,
                &mut state,
                false,
                30.0,
                1.0,
                ColorScheme::Standard,
                &texts,
            );
        });
    });
    assert!(text_shape_rect(&before_confirmation, &texts.starting_frame(497)).is_positive());
    assert!(!before_confirmation.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        egui::Shape::Text(text) if text.galley.job.text.starts_with("Frame1:")
    )));

    state.set_timeleap_timeline_anchor_for_test(Some(std::time::Instant::now()), Some(497));
    let provisionally_adopted = context.run(egui::RawInput::default(), |context| {
        egui::CentralPanel::default().show(context, |ui| {
            field_timeline_ui(
                ui,
                &mut state,
                false,
                30.0,
                1.0,
                ColorScheme::Standard,
                &texts,
            );
        });
    });
    let starting = text_shape_rect(&provisionally_adopted, &texts.starting_frame(497));
    let provisional = text_shape_rect(&provisionally_adopted, "Frame1: 106040");
    assert!(provisional.is_positive());
    assert!(
            (starting.top() - provisional.top()).abs() <= 1.5,
            "StartingFrame and Frame1 should be on the same summary row: {starting:?} vs {provisional:?}"
        );
    let copy_buttons = text_shape_rects(&provisionally_adopted, texts.copy());
    assert_eq!(
        copy_buttons.len(),
        2,
        "both summary values have copy buttons"
    );
    let frame1_copy = copy_buttons[1].center();
    let copied = context.run(
        egui::RawInput {
            events: vec![
                egui::Event::PointerMoved(frame1_copy),
                egui::Event::PointerButton {
                    pos: frame1_copy,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
                egui::Event::PointerButton {
                    pos: frame1_copy,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                },
            ],
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                field_timeline_ui(
                    ui,
                    &mut state,
                    false,
                    30.0,
                    1.0,
                    ColorScheme::Standard,
                    &texts,
                );
            });
        },
    );
    assert_eq!(copied.platform_output.copied_text, "106040");

    state.timeleap_stage = crate::application::field::TimeleapStage::Succeeded;
    state.timeleap_confirmed_frame1 = Some(106041);
    let after_confirmation = context.run(egui::RawInput::default(), |context| {
        egui::CentralPanel::default().show(context, |ui| {
            field_timeline_ui(
                ui,
                &mut state,
                false,
                30.0,
                1.0,
                ColorScheme::Standard,
                &texts,
            );
        });
    });
    let starting = text_shape_rect(&after_confirmation, &texts.starting_frame(497));
    let actual = text_shape_rect(&after_confirmation, "Frame1: 106041");
    assert!(actual.is_positive());
    assert!(
        (starting.top() - actual.top()).abs() <= 1.5,
        "actual Frame1 {:?} should share the StartingFrame summary row {:?}",
        actual,
        starting
    );
}

#[test]
fn main_timeleap_search_button_stays_available_after_frame1_passes() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_locked = true;
    state.timeleap_stage = crate::application::field::TimeleapStage::Assumed;
    assert!(main_timeleap_search_enabled(&state));

    state.timeleap_stage = crate::application::field::TimeleapStage::Frame1Waiting;
    assert!(!main_timeleap_search_enabled(&state));

    state.timeleap_locked = false;
    state.timeleap_stage = crate::application::field::TimeleapStage::Off;
    assert!(main_timeleap_search_enabled(&state));

    state.timeleap_frame1_preparing = true;
    assert!(!main_timeleap_search_enabled(&state));
}

#[test]
fn adopted_timeleap_keeps_the_frame1_input_visible_and_disabled() {
    let context = egui::Context::default();
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1_052.0, 240.0));
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_locked = true;
    state.timeleap_stage = crate::application::field::TimeleapStage::Assumed;
    state.timeleap_frame1_input = "74241".into();
    let mut layout = TimeleapMainControlsLayout::default();
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                layout = timeleap_main_controls_ui(
                    ui,
                    &mut state,
                    30.0,
                    ui.available_width(),
                    ColorScheme::Standard,
                )
                .layout;
            });
        },
    );

    assert!(layout.frame1.is_positive());
    assert_eq!(state.timeleap_frame1_input, "74241");
    assert!(!state.can_edit_timeleap_frame1_input());
    assert!(text_shape_rect(&output, "74241").is_positive());
}

#[test]
fn main_timeleap_no_dex_is_enabled_only_for_active_unlocked_courier_setup() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_mode = TimeleapMode::Courier;
    assert!(main_timeleap_no_dex_enabled(&state));

    state.timeleap_mode = TimeleapMode::Dialogue;
    assert!(!main_timeleap_no_dex_enabled(&state));
    state.timeleap_mode = TimeleapMode::Courier;
    state.timeleap_enabled = false;
    assert!(!main_timeleap_no_dex_enabled(&state));
    state.timeleap_enabled = true;
    state.timeleap_locked = true;
    assert!(!main_timeleap_no_dex_enabled(&state));
    state.timeleap_locked = false;
    state.timeleap_stage = crate::application::field::TimeleapStage::Assumed;
    assert!(!main_timeleap_no_dex_enabled(&state));
}

#[test]
fn direct_encounter_is_editable_for_every_unlocked_timeleap_method() {
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    for method in [
        TimeleapMode::Dialogue,
        TimeleapMode::Menu,
        TimeleapMode::Courier,
    ] {
        state.timeleap_mode = method;
        assert!(main_timeleap_direct_encounter_enabled(&state));
    }
    state.timeleap_locked = true;
    assert!(!main_timeleap_direct_encounter_enabled(&state));
    state.timeleap_locked = false;
    state.timeleap_stage = crate::application::field::TimeleapStage::Assumed;
    assert!(!main_timeleap_direct_encounter_enabled(&state));
}

#[test]
fn direct_encounter_checkbox_is_next_to_no_dex_and_updates_shared_state() {
    let context = egui::Context::default();
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1_052.0, 700.0));
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_mode = TimeleapMode::Courier;
    let mut layout = TimeleapMainControlsLayout::default();
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                layout = timeleap_main_controls_ui(
                    ui,
                    &mut state,
                    30.0,
                    ui.available_width(),
                    ColorScheme::Standard,
                )
                .layout;
            });
        },
    );
    assert!(layout.direct_encounter.is_positive());
    assert!(layout.direct_encounter.left() >= layout.no_dex.right());
    assert!(layout.direct_encounter.right() <= layout.row.right());

    let click = layout.direct_encounter.center();
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            events: vec![
                egui::Event::PointerMoved(click),
                egui::Event::PointerButton {
                    pos: click,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
                egui::Event::PointerButton {
                    pos: click,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                },
            ],
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let _ = timeleap_main_controls_ui(
                    ui,
                    &mut state,
                    30.0,
                    ui.available_width(),
                    ColorScheme::Standard,
                );
            });
        },
    );
    assert!(state.timeleap_direct_encounter);
}

#[test]
fn main_timeleap_no_dex_checkbox_updates_shared_state_only_when_enabled() {
    let context = egui::Context::default();
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1_052.0, 700.0));
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    state.timeleap_mode = TimeleapMode::Courier;
    let mut layout = TimeleapMainControlsLayout::default();
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                layout = timeleap_main_controls_ui(
                    ui,
                    &mut state,
                    30.0,
                    ui.available_width(),
                    ColorScheme::Standard,
                )
                .layout;
            });
        },
    );
    assert!(layout.no_dex.left() >= layout.method.right());
    let click = layout.no_dex.center();
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            events: vec![
                egui::Event::PointerMoved(click),
                egui::Event::PointerButton {
                    pos: click,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
                egui::Event::PointerButton {
                    pos: click,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                },
            ],
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let _ = timeleap_main_controls_ui(
                    ui,
                    &mut state,
                    30.0,
                    ui.available_width(),
                    ColorScheme::Standard,
                );
            });
        },
    );
    assert!(state.timeleap_no_dex);

    state.timeleap_mode = TimeleapMode::Dialogue;
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                layout = timeleap_main_controls_ui(
                    ui,
                    &mut state,
                    30.0,
                    ui.available_width(),
                    ColorScheme::Standard,
                )
                .layout;
            });
        },
    );
    let click = layout.no_dex.center();
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            events: vec![
                egui::Event::PointerMoved(click),
                egui::Event::PointerButton {
                    pos: click,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
                egui::Event::PointerButton {
                    pos: click,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                },
            ],
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let _ = timeleap_main_controls_ui(
                    ui,
                    &mut state,
                    30.0,
                    ui.available_width(),
                    ColorScheme::Standard,
                );
            });
        },
    );
    assert!(
        state.timeleap_no_dex,
        "disabled checkbox preserves its value"
    );
}

#[test]
fn field_settings_keep_timeleap_actions_visible_inside_fixed_right_panel() {
    let context = egui::Context::default();
    for height in [637.6] {
        let mut state = FieldState::default();
        state.timeleap_enabled = true;
        let mut outer = egui::Rect::NOTHING;
        let mut inner = egui::Rect::NOTHING;
        let mut controls = TimeleapMainControlsLayout::default();

        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1_052.0, height),
                )),
                ..egui::RawInput::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    ui.allocate_space(egui::vec2(ui.available_width(), TOP_PANEL_HEIGHT));
                    ui.separator();
                    let (_, bottom_rect) =
                        ui.allocate_space(egui::vec2(ui.available_width(), ui.available_height()));
                    let (_, right_rect) =
                        split_panel_rects(bottom_rect, LEFT_PANEL_WIDTH, PANEL_GAP);
                    outer = right_rect;
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(outer), |ui| {
                        fixed_group_panel(ui, |ui| {
                            inner = ui.max_rect();
                            let result = field_settings_ui(
                                ui,
                                &mut state,
                                false,
                                false,
                                false,
                                false,
                                30.0,
                                ColorScheme::Standard,
                                true,
                                &Texts::new(crate::ui::i18n::Language::Japanese),
                                false,
                                false,
                            );
                            controls = result.timeleap_controls_layout;
                        });
                    });
                });
            },
        );

        assert!(outer.height() > controls.row.height());
        assert!(controls.row.top() >= inner.top() - 0.5);
        assert!(
            controls.row.bottom() <= inner.bottom() + 0.5,
            "row {:?} exceeds inner panel {:?}",
            controls.row,
            inner
        );
        assert!(
                inner.bottom() - controls.row.bottom() >= 24.0,
                "Timeleap row {:?} should follow settings naturally instead of being pinned to the panel bottom {:?}",
                controls.row,
                inner
            );
        assert!(controls.search.left() >= inner.left() - 0.5);
        assert!(controls.search.right() <= inner.right() + 0.5);
        assert!(controls.search.width() >= 30.0);
        assert!(controls.row.right() - controls.search.right() >= 3.5);
        for rect in [controls.method, controls.no_dex] {
            assert!(rect.left() >= inner.left() - 0.5);
            assert!(rect.right() <= inner.right() + 0.5);
            assert!(rect.bottom() <= inner.bottom() + 0.5);
        }
        assert!(controls.no_dex.left() >= controls.method.right());
        assert!(controls.method.top() > controls.row.bottom());
    }
}

#[test]
fn field_screen_timeleap_controls_fit_after_the_real_tab_and_timer_layout() {
    let context = egui::Context::default();
    for width in [900.0, 1_052.0, 1_280.0] {
        let mut state = FieldState::default();
        state.timeleap_enabled = true;
        state.timeleap_frame1_reachable = Some(true);
        state.timeleap_stage = crate::application::field::TimeleapStage::Assumed;
        let mut controls = TimeleapMainControlsLayout::default();
        let mut panel_inner = egui::Rect::NOTHING;
        let mut observation_panel = egui::Rect::NOTHING;
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 637.6),
                )),
                ..egui::RawInput::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    ui.horizontal(|ui| {
                        let _ = ui.selectable_label(false, "孵化");
                        let _ = ui.selectable_label(true, "フィールド");
                        let _ = ui.selectable_label(false, "タイマー");
                        let _ = ui.selectable_label(false, "設定");
                    });
                    ui.separator();
                    let actions = field_ui(
                        ui,
                        &mut state,
                        FieldUiWorkerState::default(),
                        context,
                        30.0,
                        false,
                        true,
                        ColorScheme::Standard,
                        &Texts::new(crate::ui::i18n::Language::Japanese),
                        false,
                    );
                    controls = actions.timeleap_controls_layout;
                    panel_inner = actions.timeleap_controls_panel;
                    observation_panel = actions.observation_panel;
                });
            },
        );

        assert!(
            controls.row.left() >= panel_inner.left() - 0.5,
            "row {:?} outside estimated inner panel {:?} at width {width}",
            controls.row,
            panel_inner
        );
        assert!(
            controls.row.right() <= panel_inner.right() + 0.5,
            "row {:?} outside inner panel {:?} at width {width}",
            controls.row,
            panel_inner
        );
        assert!(controls.search.width() >= 30.0);
        assert!(controls.search.right() <= panel_inner.right() + 0.5);
        assert!(
            controls.search.right() <= controls.row.right() - 3.5,
            "Search button {:?} should keep a 4px right inset from row {:?}",
            controls.search,
            controls.row
        );
        let method_label = text_shape_rect(&output, "方式");
        let range_label = text_shape_rect(&output, "再検索範囲（Frame1） ±");
        assert!(method_label.top() > controls.row.bottom());
        assert!(range_label.right() < panel_inner.left());
        let observation_heading = text_shape_rect(&output, "瞬き観測");
        assert!(range_label.top() > observation_heading.bottom());
        assert!(range_label.left() >= observation_panel.left());
        assert!(range_label.right() <= observation_panel.right());
        assert!(range_label.bottom() < observation_panel.bottom() - 40.0);
        let search_text = text_shape_rect(&output, "検索");
        assert!(
            (search_text.center().x - controls.search.center().x).abs() <= 0.5,
            "Main search label {:?} is not centered in button {:?}",
            search_text,
            controls.search
        );
        assert!(controls.frame1_indicator_row.right() <= controls.search.left());
        assert!(
            controls.npc_row.bottom() <= controls.separator_before_timeleap.center().y,
            "NPC row {:?} should precede separator {:?} at width {width}",
            controls.npc_row,
            controls.separator_before_timeleap
        );
        assert!(
            controls.separator_before_timeleap.center().y < controls.row.top(),
            "separator {:?} should precede Timeleap row {:?} at width {width}",
            controls.separator_before_timeleap,
            controls.row
        );
        assert!(controls.frame1_indicator_row.top() >= controls.row.top() - 0.5);
        assert!(controls.frame1_indicator_row.bottom() <= controls.row.bottom() + 0.5);
        assert!(
            controls.separator_before_correction.center().y < controls.correction_row.top(),
            "separator {:?} should precede correction row {:?} at width {width}",
            controls.separator_before_correction,
            controls.correction_row
        );
    }
}
