use super::*;

#[test]
fn frame1_and_target_search_list_values_share_font_without_resizing_columns() {
    let context = egui::Context::default();
    let frame1_row = TimeleapMainCandidateRow {
        values: [
            "13579".into(),
            "2".into(),
            "24680".into(),
            "0".into(),
            "00:00:10".into(),
            "00:00:20".into(),
        ],
        frame1: 13_579,
        target_frame: 24_680,
        frame1_group_tick_start: 1,
        frame1_group_tick_end: 1,
    };
    let target_row = crate::domain::timeleap::ConfirmedTargetSearchRow {
        frame1: 24_680,
        target_frame: 35_791,
        drift_frames: 0,
        grace_frames: 0,
        generation_start_frame: Some(48_000),
    };
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 220.0),
            )),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                timeleap_main_candidate_rows_ui(
                    ui,
                    std::slice::from_ref(&frame1_row),
                    true,
                    &[Some(0)],
                );
                confirmed_target_search_row(
                    ui,
                    &target_row,
                    &confirmed_target_search_column_widths(),
                    ui.cursor().left() + CONFIRMED_TARGET_SEARCH_OUTER_INSET,
                    0,
                    false,
                    false,
                    Some(0),
                );
            });
        },
    );

    assert_eq!(
        text_shape_font_id(&output, "13579"),
        text_shape_font_id(&output, "24680")
    );
    assert_eq!(
        TIMELEAP_FRAME1_CANDIDATE_COLUMN_WIDTHS,
        [90.0, 80.0, 115.0, 80.0, 80.0, 80.0]
    );
}

#[test]
fn frame1_and_target_search_rows_share_target_table_vertical_pitch() {
    let context = egui::Context::default();
    let frame1_rows = [
        TimeleapMainCandidateRow {
            values: [
                "100".into(),
                "2".into(),
                "200".into(),
                "0".into(),
                "1".into(),
                "2".into(),
            ],
            frame1: 100,
            target_frame: 200,
            frame1_group_tick_start: 1,
            frame1_group_tick_end: 1,
        },
        TimeleapMainCandidateRow {
            values: [
                "102".into(),
                "2".into(),
                "202".into(),
                "0".into(),
                "3".into(),
                "4".into(),
            ],
            frame1: 102,
            target_frame: 202,
            frame1_group_tick_start: 2,
            frame1_group_tick_end: 2,
        },
    ];
    let target_rows = [
        crate::domain::timeleap::ConfirmedTargetSearchRow {
            frame1: 100,
            target_frame: 200,
            drift_frames: 0,
            grace_frames: 0,
            generation_start_frame: Some(200),
        },
        crate::domain::timeleap::ConfirmedTargetSearchRow {
            frame1: 102,
            target_frame: 202,
            drift_frames: 2,
            grace_frames: 0,
            generation_start_frame: Some(202),
        },
    ];
    let mut frame1_rects = Vec::new();
    let mut target_rects = Vec::new();
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 300.0),
            )),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                frame1_rects =
                    timeleap_main_candidate_rows_ui(ui, &frame1_rows, true, &[Some(0); 2]).0;
                for (index, row) in target_rows.iter().enumerate() {
                    target_rects.push(
                        confirmed_target_search_row(
                            ui,
                            row,
                            &confirmed_target_search_column_widths(),
                            ui.cursor().left() + CONFIRMED_TARGET_SEARCH_OUTER_INSET,
                            index,
                            false,
                            false,
                            Some(0),
                        )
                        .0,
                    );
                }
            });
        },
    );

    let frame1_cell_height = frame1_rects[0].height();
    let target_cell_height = target_rects[0][0].height();
    let frame1_pitch = frame1_rects[6].top() - frame1_rects[0].top();
    let target_pitch = target_rects[1][0].top() - target_rects[0][0].top();
    assert!((frame1_cell_height - target_cell_height).abs() < 0.5);
    assert!(
            (frame1_pitch - target_pitch).abs() < 0.5,
            "Frame1 row pitch {frame1_pitch} should match the more readable TargetFrame pitch {target_pitch}"
        );
}

#[test]
fn offset_context_menu_labels_show_the_computed_value_or_explain_unavailable() {
    assert_eq!(offset_context_menu_label(Some(22)), "Offsetを22にセット");
    assert_eq!(
        offset_context_menu_label(None),
        "Offsetをセット（計算不可）"
    );
}

#[test]
fn target_frame_set_action_is_disabled_when_the_correction_policy_locks_it() {
    let context = egui::Context::default();
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(360.0, 120.0));
    let mut button_rect = egui::Rect::NOTHING;
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                button_rect = target_frame_set_menu_button(ui, false).rect;
            });
        },
    );

    let mut response = None;
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            events: vec![
                egui::Event::PointerMoved(button_rect.center()),
                egui::Event::PointerButton {
                    pos: button_rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
                egui::Event::PointerButton {
                    pos: button_rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                },
            ],
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                response = Some(target_frame_set_menu_button(ui, false));
            });
        },
    );

    let response = response.expect("TargetFrame Set action should still be rendered");
    assert!(!response.enabled());
    assert!(!response.clicked());
}
