use super::*;

#[test]
fn confirmed_target_search_highlights_only_exact_generation_start_frames() {
    let row = crate::domain::timeleap::ConfirmedTargetSearchRow {
        frame1: 500,
        target_frame: 1_000,
        drift_frames: 0,
        grace_frames: 2,
        generation_start_frame: Some(1_020),
    };
    assert!(is_exact_generation_start(&row, Some(1_020)));
    assert!(!is_exact_generation_start(&row, Some(1_019)));
    assert!(!is_exact_generation_start(&row, None));
}

#[test]
fn drift_search_marker_selects_exact_missing_value_boundary() {
    let rows = [
        crate::domain::timeleap::ConfirmedTargetSearchRow {
            frame1: 500,
            target_frame: 998,
            drift_frames: -2,
            grace_frames: 2,
            generation_start_frame: Some(998),
        },
        crate::domain::timeleap::ConfirmedTargetSearchRow {
            frame1: 500,
            target_frame: 1_002,
            drift_frames: 2,
            grace_frames: 2,
            generation_start_frame: Some(1_004),
        },
    ];

    assert_eq!(drift_search_marker_boundary(&rows, 1_001), Some(1));
    assert_eq!(drift_search_marker_boundary(&rows, 997), Some(0));
    assert_eq!(drift_search_marker_boundary(&rows, 1_010), Some(2));
    assert_eq!(drift_search_marker_boundary(&rows, 1_004), None);
    assert_eq!(drift_search_marker_boundary(&[], 1_001), None);

    let uncomputed = [crate::domain::timeleap::ConfirmedTargetSearchRow {
        generation_start_frame: None,
        ..rows[0].clone()
    }];
    assert_eq!(drift_search_marker_boundary(&uncomputed, 1_001), None);
}

#[test]
fn confirmed_target_table_draws_only_rows_inside_the_viewport() {
    let context = egui::Context::default();
    let rows = (0..2_000)
        .map(|index| crate::domain::timeleap::ConfirmedTargetSearchRow {
            frame1: 35_624,
            target_frame: 69_846 + index,
            drift_frames: index,
            grace_frames: 2,
            generation_start_frame: Some(70_000 + index),
        })
        .collect::<Vec<_>>();
    let state = FieldState::default();
    let widths = confirmed_target_search_column_widths();
    let mut visible_row_count = 0;
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(720.0, 180.0),
            )),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let actions = confirmed_target_search_rows_virtualized_ui(
                    ui,
                    &rows,
                    Some(70_000),
                    None,
                    None,
                    &widths,
                    ui.cursor().left() + CONFIRMED_TARGET_SEARCH_OUTER_INSET,
                    &state,
                    false,
                    &Texts::new(crate::ui::i18n::Language::Japanese),
                );
                visible_row_count = actions.visible_row_count;
            });
        },
    );

    assert!(visible_row_count > 0);
    assert!(visible_row_count < rows.len());
}

#[test]
fn confirmed_target_search_header_and_row_share_fixed_column_rectangles() {
    let context = egui::Context::default();
    let widths = confirmed_target_search_column_widths();
    let row = crate::domain::timeleap::ConfirmedTargetSearchRow {
        frame1: 35_624,
        target_frame: 69_846,
        drift_frames: 0,
        grace_frames: 2,
        generation_start_frame: Some(70_000),
    };
    let mut header_cells = [egui::Rect::NOTHING; 5];
    let mut row_cells = [egui::Rect::NOTHING; 5];
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(720.0, 180.0),
            )),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let table_left = ui.cursor().left() + CONFIRMED_TARGET_SEARCH_OUTER_INSET;
                        ui.label("Frame1 font baseline");
                        header_cells = confirmed_target_search_header(ui, &widths, table_left);
                        row_cells = confirmed_target_search_row(
                            ui,
                            &row,
                            &widths,
                            table_left,
                            0,
                            true,
                            true,
                            Some(20),
                        )
                        .0;
                    });
            });
        },
    );

    for (index, (header, cell)) in header_cells.iter().zip(row_cells.iter()).enumerate() {
        assert!(
            (header.left() - cell.left()).abs() < 0.5,
            "column {index} left edge differs: header={header:?}, row={cell:?}"
        );
        assert!(
            (header.right() - cell.right()).abs() < 0.5,
            "column {index} right edge differs: header={header:?}, row={cell:?}"
        );
    }
    for (cell, value) in row_cells
        .iter()
        .zip(["35624", "69846", "+0F", "2F", "70000"])
    {
        let text_rect = text_shape_rect(&output, value);
        assert!(
            (text_rect.center().x - cell.center().x).abs() <= 0.5,
            "TargetFrame value {value} should be centered in {cell:?}, got {text_rect:?}"
        );
    }
    for cell in header_cells.iter().take(5) {
        let help_rect = confirmed_target_search_help_rect(*cell);
        assert!((help_rect.width() - help_rect.height()).abs() < f32::EPSILON);
        assert!(
            help_rect.width() < 22.0,
            "help control should not use the old wide slot"
        );
        assert!(cell.contains_rect(help_rect));
    }
    let table_right = header_cells[4].right();
    assert!(table_right <= 720.0);
    let frame1_font = text_shape_font_id(&output, "Frame1 font baseline");
    assert_eq!(
        text_shape_font_id(&output, "Frame1"),
        frame1_font,
        "TargetFrame search header should use the Frame1 table body font"
    );
    assert_eq!(
        text_shape_font_id(&output, "35624"),
        frame1_font,
        "TargetFrame search values should use the Frame1 table body font"
    );
    let frame1_text_rect = text_shape_rect(&output, "35624");
    assert!(
        (frame1_text_rect.center().x - row_cells[0].center().x).abs() < 0.5,
        "row text should be centered in its cell: text={frame1_text_rect:?}, cell={:?}",
        row_cells[0]
    );
    let frame1_header_text = text_shape_rect(&output, "Frame1");
    let frame1_help_rect = confirmed_target_search_help_rect(header_cells[0]);
    assert!(
        frame1_header_text.left() >= header_cells[0].left() + CONFIRMED_TARGET_SEARCH_CELL_INSET
    );
    assert!(
        frame1_header_text.right() + CONFIRMED_TARGET_SEARCH_CELL_INSET <= frame1_help_rect.left()
    );
    for (label, cell) in [
        ("TargetFrame", header_cells[1]),
        ("ずれ", header_cells[2]),
        ("猶予", header_cells[3]),
        ("生成開始Frame", header_cells[4]),
    ] {
        let text = text_shape_rect(&output, label);
        let help = confirmed_target_search_help_rect(cell);
        assert!(text.left() >= cell.left() + CONFIRMED_TARGET_SEARCH_CELL_INSET);
        assert!(
            text.right() + CONFIRMED_TARGET_SEARCH_CELL_INSET <= help.left(),
            "{label} overlaps its help button: text={text:?}, help={help:?}, cell={cell:?}"
        );
    }
    for (value, cell) in [
        ("69846", row_cells[1]),
        ("+0F", row_cells[2]),
        ("2F", row_cells[3]),
        ("70000", row_cells[4]),
    ] {
        let text = text_shape_rect(&output, value);
        assert!(text.left() >= cell.left() + CONFIRMED_TARGET_SEARCH_CELL_INSET);
        assert!(text.right() + CONFIRMED_TARGET_SEARCH_CELL_INSET <= cell.right());
    }

    let highlight_color = context.style().visuals.selection.bg_fill;
    assert!(
        output.shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) => {
                rect.fill == highlight_color
                    && (rect.rect.left() - row_cells[0].left()).abs() < 0.5
                    && (rect.rect.right() - row_cells[4].right()).abs() < 0.5
                    && (rect.rect.top() - row_cells[0].top()).abs() < 0.5
            }
            _ => false,
        }),
        "the matching row should have one blue background spanning every column"
    );
    assert!(
        output.shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) => {
                rect.stroke.color == egui::Color32::BLACK
                    && (rect.rect.left() - row_cells[0].left() - 1.0).abs() < 0.5
                    && (rect.rect.right() - row_cells[4].right() + 1.0).abs() < 0.5
                    && (rect.rect.top() - row_cells[0].top() - 1.0).abs() < 0.5
                    && (rect.rect.bottom() - row_cells[0].bottom() + 1.0).abs() < 0.5
            }
            _ => false,
        }),
        "the drift match outline should surround the entire row"
    );
    assert!(!output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        egui::Shape::Text(text) if text.galley.job.text == "Set"
    )));
}

#[test]
fn confirmed_target_rows_use_alternating_frame1_backgrounds() {
    let context = egui::Context::default();
    let widths = confirmed_target_search_column_widths();
    let rows = [
        crate::domain::timeleap::ConfirmedTargetSearchRow {
            frame1: 10,
            target_frame: 20,
            drift_frames: 0,
            grace_frames: 0,
            generation_start_frame: Some(30),
        },
        crate::domain::timeleap::ConfirmedTargetSearchRow {
            frame1: 12,
            target_frame: 22,
            drift_frames: 2,
            grace_frames: 0,
            generation_start_frame: Some(32),
        },
    ];
    let mut row_rects = [egui::Rect::NOTHING; 2];
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(720.0, 180.0),
            )),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                for (index, row) in rows.iter().enumerate() {
                    row_rects[index] = confirmed_target_search_row(
                        ui,
                        row,
                        &widths,
                        ui.cursor().left() + CONFIRMED_TARGET_SEARCH_OUTER_INSET,
                        index,
                        false,
                        false,
                        Some(20),
                    )
                    .0[0];
                }
            });
        },
    );
    let stripe = context.style().visuals.faint_bg_color;
    assert!(output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        egui::Shape::Rect(rect)
            if rect.fill == stripe
                && (rect.rect.top() - row_rects[0].top()).abs() < 0.5
    )));
    assert!(!output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        egui::Shape::Rect(rect)
            if rect.fill == stripe
                && (rect.rect.top() - row_rects[1].top()).abs() < 0.5
    )));
}

#[test]
fn centered_search_button_centers_its_label() {
    let context = egui::Context::default();
    let mut button_rect = egui::Rect::NOTHING;
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(360.0, 120.0),
            )),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                button_rect = centered_button(ui, true, 280.0, "検索").rect;
            });
        },
    );
    let label_rect = text_shape_rect(&output, "検索");
    assert!((label_rect.center().x - button_rect.center().x).abs() < 1.0);
}

#[test]
fn frame1_candidate_search_button_always_says_search() {
    let context = egui::Context::default();
    for _previous_search_completed in [false, true] {
        let mut button_rect = egui::Rect::NOTHING;
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 100.0),
                )),
                ..egui::RawInput::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    button_rect = timeleap_candidate_search_button(ui, true, 240.0).rect;
                });
            },
        );

        let label = text_shape_rect(&output, "検索");
        assert!((label.center().x - button_rect.center().x).abs() < 1.0);
        assert!(!output.shapes.iter().any(|clipped| matches!(
            &clipped.shape,
            egui::Shape::Text(text) if text.galley.job.text == "再検索"
        )));
    }
}

#[test]
fn confirmed_target_table_starts_at_left_inset_when_it_fits_the_viewport() {
    let context = egui::Context::default();
    let widths = confirmed_target_search_column_widths();
    let table_width = confirmed_target_search_content_width(&widths);
    let mut actual_left = 0.0;
    let mut expected_left = 0.0;
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 120.0),
            )),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let viewport_width = ui.available_width();
                expected_left = ui.cursor().left() + CONFIRMED_TARGET_SEARCH_OUTER_INSET;
                actual_left = confirmed_target_search_table_left(ui, table_width, viewport_width);
            });
        },
    );
    assert!((actual_left - expected_left).abs() < 0.5);
}

#[test]
fn confirmed_target_search_table_scrolls_horizontally_at_narrow_width() {
    let context = egui::Context::default();
    let widths = confirmed_target_search_column_widths();
    let row = crate::domain::timeleap::ConfirmedTargetSearchRow {
        frame1: 35_624,
        target_frame: 69_846,
        drift_frames: 0,
        grace_frames: 2,
        generation_start_frame: Some(70_000),
    };
    let mut scroll_metrics = None;
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(480.0, 180.0),
            )),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let output = egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let table_left = ui.cursor().left() + CONFIRMED_TARGET_SEARCH_OUTER_INSET;
                        confirmed_target_search_header(ui, &widths, table_left);
                        confirmed_target_search_row(
                            ui,
                            &row,
                            &widths,
                            table_left,
                            0,
                            false,
                            false,
                            Some(20),
                        );
                    });
                scroll_metrics = Some((output.inner_rect, output.content_size));
            });
        },
    );

    let (viewport, content_size) = scroll_metrics.expect("scroll area should be rendered");
    let table_width =
        confirmed_target_search_content_width(&widths) + CONFIRMED_TARGET_SEARCH_OUTER_INSET * 2.0;
    assert!(content_size.x >= table_width);
    assert!(
            content_size.x > viewport.width(),
            "narrow layouts should scroll horizontally instead of clipping columns: viewport={viewport:?}, content={content_size:?}"
        );
}

#[test]
fn nonmatching_generation_frame_is_not_highlighted() {
    let unreachable_row = crate::domain::timeleap::ConfirmedTargetSearchRow {
        frame1: 500,
        target_frame: 1_002,
        drift_frames: 2,
        grace_frames: 0,
        generation_start_frame: None,
    };
    assert!(!is_exact_generation_start(&unreachable_row, Some(1_020)));
    assert!(!is_exact_drift_search_generation_start(
        &unreachable_row,
        Some(1_020)
    ));
}

#[test]
fn drift_search_outlines_measured_row_and_keeps_unreachable_ticks_visible() {
    let mut state = FieldState::default();
    state.confirmed_target_search_form.initialized = true;
    state.timeleap_generation_start_frame = "1004".into();
    state.confirmed_target_search_form.drift_search_enabled = true;
    state
        .confirmed_target_search_form
        .drift_search_generation_start_frame = "1004".into();
    state
        .confirmed_target_search_form
        .result_generation_start_frame = Some(1_004);
    state
        .confirmed_target_search_form
        .result_drift_search_generation_start_frame = Some(1_004);
    state.confirmed_target_search_form.status =
        crate::application::field::ConfirmedTargetSearchStatus::Results(vec![
            crate::domain::timeleap::ConfirmedTargetSearchRow {
                frame1: 500,
                target_frame: 998,
                drift_frames: -2,
                grace_frames: 2,
                generation_start_frame: None,
            },
            crate::domain::timeleap::ConfirmedTargetSearchRow {
                frame1: 500,
                target_frame: 1_002,
                drift_frames: 2,
                grace_frames: 2,
                generation_start_frame: Some(1_004),
            },
        ]);
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        egui::CentralPanel::default().show(context, |ui| {
            confirmed_target_search_tab_ui(
                ui,
                &mut state,
                &Texts::new(crate::ui::i18n::Language::Japanese),
                false,
                false,
                false,
            );
        });
    });

    assert!(output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        egui::Shape::Rect(rect)
            if rect.stroke.width == 1.5 && rect.stroke.color == egui::Color32::BLACK
    )));
    assert!(output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        egui::Shape::Text(text) if text.galley.job.text == "—"
    )));
    assert!(text_shape_rect(&output, "Frame1").is_positive());
    assert!(text_shape_rect(&output, "500").is_positive());
    assert!(text_shape_rect(&output, "TargetFrame").is_positive());
    assert!(text_shape_rect(&output, "998").is_positive());
    assert!(!output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        egui::Shape::Text(text) if text.galley.job.text == "候補: 2件"
    )));
}

#[test]
fn target_search_keeps_nonmatching_tick_rows_and_does_not_draw_a_marker_line() {
    let mut state = FieldState::default();
    state.confirmed_target_search_form.initialized = true;
    state.timeleap_generation_start_frame = "1001".into();
    state
        .confirmed_target_search_form
        .result_generation_start_frame = Some(1_001);
    state.confirmed_target_search_form.status =
        crate::application::field::ConfirmedTargetSearchStatus::Results(vec![
            crate::domain::timeleap::ConfirmedTargetSearchRow {
                frame1: 500,
                target_frame: 998,
                drift_frames: -2,
                grace_frames: 2,
                generation_start_frame: Some(998),
            },
            crate::domain::timeleap::ConfirmedTargetSearchRow {
                frame1: 500,
                target_frame: 1_002,
                drift_frames: 2,
                grace_frames: 2,
                generation_start_frame: Some(1_004),
            },
        ]);
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        egui::CentralPanel::default().show(context, |ui| {
            confirmed_target_search_tab_ui(
                ui,
                &mut state,
                &Texts::new(crate::ui::i18n::Language::Japanese),
                false,
                false,
                false,
            );
        });
    });

    assert!(!output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        egui::Shape::LineSegment { stroke, .. }
            if stroke.width == 3.0
    )));
    assert!(output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        egui::Shape::Text(text) if text.galley.job.text == "998"
    )));
    assert!(output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        egui::Shape::Text(text) if text.galley.job.text == "1004"
    )));
}

#[test]
fn target_search_shows_explicit_unreachable_state_without_table_rows() {
    let mut state = FieldState::default();
    state.confirmed_target_search_form.initialized = true;
    state.confirmed_target_search_form.status =
        crate::application::field::ConfirmedTargetSearchStatus::GenerationFrameUnreachable;
    let context = egui::Context::default();
    let mut actions = None;
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1_000.0, 640.0),
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
    let actions = actions.expect("TargetFrame search renders its result pane");
    assert!(output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        egui::Shape::Text(text)
            if text.galley.job.text == "指定した生成開始Frameには到達できません。"
    )));
    assert!(actions
        .result_header_cells
        .iter()
        .all(|rect| rect.is_positive()));
    assert!(actions.result_body_rect.is_positive());
    assert!(actions.result_status_rect.is_positive());
    assert!(
        (actions.result_status_rect.center().x - actions.result_body_rect.center().x).abs() < 1.0,
        "status should be centered horizontally below the pinned header: body={:?}, status={:?}",
        actions.result_body_rect,
        actions.result_status_rect
    );
    assert!(
        (actions.result_status_rect.center().y - actions.result_body_rect.center().y).abs() < 1.0,
        "status should be centered vertically below the pinned header: body={:?}, status={:?}",
        actions.result_body_rect,
        actions.result_status_rect
    );
    assert!(!output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        egui::Shape::Text(text) if text.galley.job.text == "998"
    )));
}

#[test]
fn target_search_shows_frame1_unreachable_message_below_persistent_headers() {
    let mut state = FieldState::default();
    state.confirmed_target_search_form.initialized = true;
    state.confirmed_target_search_form.status =
        crate::application::field::ConfirmedTargetSearchStatus::Frame1Unreachable;
    let context = egui::Context::default();
    let mut actions = None;
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1_000.0, 640.0),
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
    let actions = actions.expect("TargetFrame search renders its result pane");
    let message = text_shape_rect(&output, "指定したFrame1には到達できません。");
    assert!(message.is_positive());
    assert!(actions
        .result_header_cells
        .iter()
        .all(|rect| rect.is_positive()));
    assert!(actions.result_body_rect.is_positive());
    assert!(
        (message.center().x - actions.result_body_rect.center().x).abs() < 1.0,
        "Frame1-unreachable message should be centered below the headings"
    );
    assert!(
        (message.center().y - actions.result_body_rect.center().y).abs() < 1.0,
        "Frame1-unreachable message should be centered below the headings"
    );
    assert!(!output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        egui::Shape::Text(text)
            if text.galley.job.text == "指定した生成開始Frameには到達できません。"
    )));
}

#[test]
fn shared_search_progress_uses_centered_spinner_and_matching_label_geometry() {
    let context = egui::Context::default();
    let body_rect = egui::Rect::from_min_size(egui::pos2(100.0, 80.0), egui::vec2(600.0, 320.0));
    let texts = Texts::new(crate::ui::i18n::Language::Japanese);
    let mut progress_rect = egui::Rect::NOTHING;
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 600.0),
            )),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                progress_rect = search_progress_ui(ui, body_rect, &texts);
            });
        },
    );
    let labels = text_shape_rects(&output, "検索中…");
    assert_eq!(labels.len(), 1, "the shared progress label appears once");
    assert!(
        (progress_rect.center().x - body_rect.center().x).abs() < 0.1
            && (progress_rect.center().y - body_rect.center().y).abs() < 0.1,
        "spinner and label are centered as one group"
    );
    assert!(
        (progress_rect.height() - 18.0).abs() < 0.1,
        "the shared spinner retains its 18-point footprint"
    );
    let message = labels[0];
    assert!((message.center().y - body_rect.center().y).abs() < 1.0);
    assert!(progress_rect.left() < message.left());
}

#[test]
fn target_search_running_spinner_and_message_are_centered_once_below_headings() {
    let mut state = FieldState::default();
    state.confirmed_target_search_form.initialized = true;
    state.confirmed_target_search_form.status =
        crate::application::field::ConfirmedTargetSearchStatus::Running;
    let context = egui::Context::default();
    let mut actions = None;
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1_000.0, 640.0),
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
    let actions = actions.expect("TargetFrame search renders its result pane");
    let message = text_shape_rect(&output, "検索中…");
    assert_eq!(
        text_shape_rects(&output, "検索中…").len(),
        1,
        "TargetFrame search uses the same single progress label as Frame1 search"
    );
    assert!(message.is_positive());
    assert!(actions
        .result_header_cells
        .iter()
        .all(|rect| rect.is_positive()));
    assert!(actions.result_body_rect.is_positive());
    assert!(actions.result_status_rect.is_positive());
    assert!(
        (actions.result_status_rect.center().x - actions.result_body_rect.center().x).abs() < 1.0,
        "the spinner and its message should be centered as one group"
    );
    assert!(
        (actions.result_status_rect.center().y - actions.result_body_rect.center().y).abs() < 1.0,
        "the spinner and its message should be centered below the headings"
    );
    assert!(actions.result_status_rect.left() < message.left());
    assert!((actions.result_status_rect.height() - 18.0).abs() < 0.1);
}

#[test]
fn target_search_no_results_and_failure_states_keep_and_center_under_the_headings() {
    let statuses = [
        (
            crate::application::field::ConfirmedTargetSearchStatus::NoResults,
            "条件に一致する候補はありません。",
        ),
        (
            crate::application::field::ConfirmedTargetSearchStatus::Failed(
                "検索に失敗しました".into(),
            ),
            "検索に失敗しました",
        ),
    ];

    for (status, expected_text) in statuses {
        let mut state = FieldState::default();
        state.confirmed_target_search_form.initialized = true;
        state.confirmed_target_search_form.status = status;
        let context = egui::Context::default();
        let mut actions = None;
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1_000.0, 640.0),
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
        let actions = actions.expect("TargetFrame search renders its result pane");
        assert!(text_shape_rect(&output, expected_text).is_positive());
        assert!(actions
            .result_header_cells
            .iter()
            .all(|rect| rect.is_positive()));
        assert!(
            (actions.result_status_rect.center().x - actions.result_body_rect.center().x).abs()
                < 1.0
        );
        assert!(
            (actions.result_status_rect.center().y - actions.result_body_rect.center().y).abs()
                < 1.0
        );
    }
}

#[test]
fn target_search_pins_headings_vertically_and_scrolls_them_with_rows_horizontally() {
    let context = egui::Context::default();
    let rows = (0..100)
        .map(|index| crate::domain::timeleap::ConfirmedTargetSearchRow {
            frame1: 35_624,
            target_frame: 69_846 + index,
            drift_frames: index,
            grace_frames: 2,
            generation_start_frame: Some(70_000 + index),
        })
        .collect::<Vec<_>>();
    let widths = confirmed_target_search_column_widths();
    let state = FieldState::default();
    let status = crate::application::field::ConfirmedTargetSearchStatus::Results(rows.clone());
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(480.0, 220.0));
    let mut initial_actions = None;
    let initial = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let actions = confirmed_target_search_table_ui(
                    ui,
                    &status,
                    Some(70_000),
                    None,
                    None,
                    &widths,
                    &state,
                    false,
                );
                initial_actions = Some(actions);
            });
        },
    );
    let initial_actions = initial_actions.expect("initial result table is rendered");
    let initial_header_text = text_shape_rect(&initial, "Frame1");
    assert!(initial_actions.header_cells[0].is_positive());
    assert!(initial_actions.body_rect.is_positive());
    assert!(initial_actions.first_row_cells[0].is_positive());
    assert!(initial_header_text.is_positive());

    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            events: vec![
                egui::Event::PointerMoved(initial_actions.body_rect.center()),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(0.0, -8.0),
                    modifiers: egui::Modifiers::default(),
                },
            ],
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                confirmed_target_search_table_ui(
                    ui,
                    &status,
                    Some(70_000),
                    None,
                    None,
                    &widths,
                    &state,
                    false,
                );
            });
        },
    );
    let mut vertical_actions = None;
    let scrolled_vertically = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                vertical_actions = Some(confirmed_target_search_table_ui(
                    ui,
                    &status,
                    Some(70_000),
                    None,
                    None,
                    &widths,
                    &state,
                    false,
                ));
            });
        },
    );
    let vertical_header_text = text_shape_rect(&scrolled_vertically, "Frame1");
    let vertical_actions = vertical_actions.expect("row viewport is rendered");
    assert!(
        (vertical_header_text.top() - initial_header_text.top()).abs() < 0.5,
        "headings must remain pinned during vertical row scrolling"
    );
    assert!(
        (vertical_actions.first_row_cells[0].top()
            - initial_actions.first_row_cells[0].top())
            .abs()
            > 1.0
            && vertical_actions.first_row_cells[0].intersects(vertical_actions.body_rect),
        "rows should move within the body viewport below the fixed heading: initial {:?}, scrolled {:?}, offset {:?}, content {:?}, viewport {:?}",
        initial_actions.first_row_cells[0],
        vertical_actions.first_row_cells[0],
        vertical_actions.scroll_offset,
        vertical_actions.content_size,
        vertical_actions.body_rect
    );

    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            events: vec![
                egui::Event::PointerMoved(initial_actions.body_rect.center()),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(-80.0, 0.0),
                    modifiers: egui::Modifiers::default(),
                },
            ],
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                confirmed_target_search_table_ui(
                    ui,
                    &status,
                    Some(70_000),
                    None,
                    None,
                    &widths,
                    &state,
                    false,
                );
            });
        },
    );
    let mut horizontal_actions = None;
    let scrolled_horizontally = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                horizontal_actions = Some(confirmed_target_search_table_ui(
                    ui,
                    &status,
                    Some(70_000),
                    None,
                    None,
                    &widths,
                    &state,
                    false,
                ));
            });
        },
    );
    let horizontal_actions = horizontal_actions.expect("horizontally scrolled table is rendered");
    let _ = scrolled_horizontally;
    let header_delta =
        horizontal_actions.header_cells[0].left() - vertical_actions.header_cells[0].left();
    let row_delta =
        horizontal_actions.first_row_cells[0].left() - vertical_actions.first_row_cells[0].left();
    assert!(
        header_delta.abs() > 1.0,
        "the narrow table viewport should scroll horizontally"
    );
    assert!(
        (header_delta - row_delta).abs() < 0.5,
        "headings and table rows must share the same horizontal scroll offset"
    );
}

#[test]
fn exact_drift_search_frame_gets_a_black_outline_while_target_match_stays_blue() {
    let row = crate::domain::timeleap::ConfirmedTargetSearchRow {
        frame1: 500,
        target_frame: 1_000,
        drift_frames: 0,
        grace_frames: 2,
        generation_start_frame: Some(1_004),
    };
    assert!(is_exact_generation_start(&row, Some(1_004)));
    assert!(is_exact_drift_search_generation_start(&row, Some(1_004)));
    assert!(!is_exact_generation_start(&row, Some(1_000)));

    let mut state = FieldState::default();
    state.confirmed_target_search_form.initialized = true;
    state.timeleap_generation_start_frame = "1004".into();
    state.confirmed_target_search_form.drift_search_enabled = true;
    state
        .confirmed_target_search_form
        .drift_search_generation_start_frame = "1004".into();
    state
        .confirmed_target_search_form
        .result_generation_start_frame = Some(1_004);
    state
        .confirmed_target_search_form
        .result_drift_search_generation_start_frame = Some(1_004);
    state.confirmed_target_search_form.status =
        crate::application::field::ConfirmedTargetSearchStatus::Results(vec![row]);
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        egui::CentralPanel::default().show(context, |ui| {
            confirmed_target_search_tab_ui(
                ui,
                &mut state,
                &Texts::new(crate::ui::i18n::Language::Japanese),
                false,
                false,
                false,
            );
        });
    });

    assert!(output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        egui::Shape::Rect(rect)
            if rect.stroke.width == 1.5
                && rect.stroke.color == egui::Color32::BLACK
    )));
}

#[test]
fn drift_search_marks_missing_measured_frame_between_candidate_rows() {
    let mut state = FieldState::default();
    state.confirmed_target_search_form.initialized = true;
    state.timeleap_generation_start_frame = "1004".into();
    state.confirmed_target_search_form.drift_search_enabled = true;
    state
        .confirmed_target_search_form
        .drift_search_generation_start_frame = "1001".into();
    state
        .confirmed_target_search_form
        .result_generation_start_frame = Some(1_004);
    state
        .confirmed_target_search_form
        .result_drift_search_generation_start_frame = Some(1_001);
    state
        .confirmed_target_search_form
        .result_drift_marker_boundary = Some(1);
    state.confirmed_target_search_form.status =
        crate::application::field::ConfirmedTargetSearchStatus::Results(vec![
            crate::domain::timeleap::ConfirmedTargetSearchRow {
                frame1: 500,
                target_frame: 998,
                drift_frames: -2,
                grace_frames: 2,
                generation_start_frame: Some(998),
            },
            crate::domain::timeleap::ConfirmedTargetSearchRow {
                frame1: 500,
                target_frame: 1_002,
                drift_frames: 2,
                grace_frames: 2,
                generation_start_frame: Some(1_004),
            },
        ]);
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        egui::CentralPanel::default().show(context, |ui| {
            confirmed_target_search_tab_ui(
                ui,
                &mut state,
                &Texts::new(crate::ui::i18n::Language::Japanese),
                false,
                false,
                false,
            );
        });
    });

    let table_width = confirmed_target_search_column_widths().iter().sum::<f32>()
        + CONFIRMED_TARGET_SEARCH_COLUMN_GAP * 4.0;
    assert!(
        output.shapes.iter().any(|clipped| matches!(
            &clipped.shape,
            egui::Shape::LineSegment { points, stroke }
                if stroke.width == 3.0
                    && stroke.color == eframe::epaint::ColorMode::Solid(egui::Color32::BLACK)
                    && (points[1].x - points[0].x - table_width).abs() < 0.5
        )),
        "the measured-frame marker should underline the full table width"
    );
    assert!(!output.shapes.iter().any(|clipped| matches!(
        &clipped.shape,
        egui::Shape::Rect(rect)
            if rect.stroke.width == 1.5 && rect.stroke.color == egui::Color32::BLACK
    )));
}
