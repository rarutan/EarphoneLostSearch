use super::*;

#[test]
fn help_button_opens_its_explanation_on_click() {
    let context = egui::Context::default();
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0));
    let mut button_rect = egui::Rect::NOTHING;
    let mut popup_id = egui::Id::NULL;
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                popup_id = ui.make_persistent_id("test-help-popup");
                button_rect = help_button(ui, "test-help-popup", "説明文").rect;
            });
        },
    );
    let click_position = button_rect.center();
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(screen_rect),
            events: vec![
                egui::Event::PointerMoved(click_position),
                egui::Event::PointerButton {
                    pos: click_position,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
                egui::Event::PointerButton {
                    pos: click_position,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                },
            ],
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                help_button(ui, "test-help-popup", "説明文");
            });
        },
    );
    assert!(context.memory(|memory| memory.is_popup_open(popup_id)));
}

#[test]
fn main_seconds_one_help_explains_its_timeline_baseline() {
    assert!(TIMELEAP_MAIN_SECONDS_ONE_HELP.contains("現在時刻からFrame1まで"));
    assert!(TIMELEAP_MAIN_SECONDS_ONE_HELP.contains("内部で使われているタイムライン"));
    assert!(TIMELEAP_MAIN_SECONDS_ONE_HELP.contains("StartingFrameからFrame1まで"));
    assert_eq!(
        Texts::new(crate::ui::i18n::Language::English)
            .ui_label(TIMELEAP_MAIN_SECONDS_ONE_HELP),
        "Time remaining until Frame1. If the active timeline differs from the entered seed, StartingFrame, or NPC count, this is calculated from StartingFrame."
    );
}

#[test]
fn idx_inference_does_not_hide_the_confirmed_timeline_as_a_search() {
    let workers = FieldUiWorkerState {
        idx_inference_running: true,
        ..FieldUiWorkerState::default()
    };

    assert!(!field_search_is_calculating(workers, false));
}

#[test]
fn active_candidate_search_still_marks_the_view_as_calculating() {
    let workers = FieldUiWorkerState {
        search_running: true,
        ..FieldUiWorkerState::default()
    };

    assert!(field_search_is_calculating(workers, false));
    assert!(!field_search_is_calculating(workers, true));
}

#[test]
fn timeline_follow_checkbox_stays_visible_and_editable_during_timeleap() {
    let context = egui::Context::default();
    let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1_052.0, 700.0));
    let texts = Texts::new(crate::ui::i18n::Language::Japanese);
    let mut state = FieldState::default();
    state.timeleap_enabled = true;
    assert!(state.table_follow_current);

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
    let checkbox_label = text_shape_rect(&output, "現在位置を固定");
    assert!(checkbox_label.is_positive());

    let click = checkbox_label.center();
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
    assert!(!state.table_follow_current);

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
    assert!(text_shape_rect(&output, "現在位置を固定").is_positive());
    assert!(!state.table_follow_current);
}

#[test]
fn timeline_header_eight_digit_frames_do_not_overlap_follow_checkbox_at_app_width() {
    const MIN_PHYSICAL_GAP: f32 = 4.0;
    const PIXELS_PER_POINT: [f32; 4] = [1.0, 1.25, 1.5, 2.0];
    const STARTING_FRAME: i64 = 50_000_000;
    const FRAME1: i64 = 49_999_999;

    for requested_ppp in PIXELS_PER_POINT {
        let context = egui::Context::default();
        let texts = Texts::new(crate::ui::i18n::Language::Japanese);
        let mut state = FieldState::empty_timeline_for_test();
        state.timeleap_enabled = true;
        state.timeleap_stage = crate::application::field::TimeleapStage::Assumed;
        state.timeleap_selected_frame = Some(FRAME1);
        state.results[0].start_consumption = STARTING_FRAME;
        state.results[0]
            .timeline
            .push(crate::application::field::FieldTimelineRow {
                frame: STARTING_FRAME,
                timeline_frame: 0,
                duration_frames: 0,
            });
        state.set_timeleap_timeline_anchor_for_test(
            Some(std::time::Instant::now()),
            Some(STARTING_FRAME),
        );

        let mut measured = None;
        let mut raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(crate::WINDOW_WIDTH, crate::WINDOW_HEIGHT),
            )),
            ..egui::RawInput::default()
        };
        raw_input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .expect("the root viewport is present")
            .native_pixels_per_point = Some(requested_ppp);
        let output = context.run(raw_input, |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let text_scale = field_text_scale(ui.available_width());
                let top_height = (TOP_PANEL_HEIGHT * text_scale)
                    .min(300.0)
                    .min(ui.available_height() * 0.55);
                let (_, top_rect) = ui.allocate_space(egui::vec2(ui.available_width(), top_height));
                let (_, right_rect) = split_panel_rects(top_rect, LEFT_PANEL_WIDTH, PANEL_GAP);
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(right_rect), |ui| {
                    measured = Some((
                        right_rect,
                        text_scale,
                        field_timeline_header_ui(ui, &mut state, true, text_scale, &texts),
                    ));
                });
            });
        });

        let (panel_rect, text_scale, (summary_rect, checkbox_rect)) =
            measured.expect("the real Timeline header should be laid out");
        let actual_ppp = output.pixels_per_point;
        let gap_points = checkbox_rect.left() - summary_rect.right();
        let gap_pixels = gap_points * actual_ppp;
        let panel_width_pixels = panel_rect.width() * actual_ppp;
        let summary_left_points = summary_rect.left() - panel_rect.left();
        let summary_right_pixels = (summary_rect.right() - panel_rect.left()) * actual_ppp;
        let checkbox_left_pixels = (checkbox_rect.left() - panel_rect.left()) * actual_ppp;
        let checkbox_right_padding_points = panel_rect.right() - checkbox_rect.right();

        eprintln!(
            "Timeline header @ {actual_ppp:.2} px/pt: panel={:.1}pt ({panel_width_pixels:.1}px), summary left={summary_left_points:.2}pt, summary right={:.1}pt ({summary_right_pixels:.1}px), right-aligned checkbox left={:.1}pt ({checkbox_left_pixels:.1}px), right padding={checkbox_right_padding_points:.2}pt, gap={gap_points:.2}pt ({gap_pixels:.2}px)",
            panel_rect.width(),
            summary_rect.right() - panel_rect.left(),
            checkbox_rect.left() - panel_rect.left(),
        );

        assert!(summary_rect.is_positive());
        assert!(checkbox_rect.is_positive());
        assert!(summary_rect.left() >= panel_rect.left() - 0.5);
        assert!(summary_rect.right() <= panel_rect.right() + 0.5);
        assert!(
            summary_left_points <= 8.0,
            "summary should remain left-aligned"
        );
        assert!(checkbox_rect.right() <= panel_rect.right() + 0.5);
        assert!(
            checkbox_right_padding_points <= 8.0,
            "checkbox should remain right-aligned; it is {checkbox_right_padding_points:.2}pt from the panel edge at {actual_ppp:.2} px/pt",
        );
        assert!(
            !summary_rect.intersects(checkbox_rect),
            "8-digit summary {:?} overlaps fixed checkbox {:?} at {actual_ppp:.2} px/pt",
            summary_rect,
            checkbox_rect,
        );
        assert!(
            gap_pixels >= MIN_PHYSICAL_GAP,
            "only {gap_pixels:.2}px between 8-digit summary {:?} and checkbox {:?} at {actual_ppp:.2} px/pt",
            summary_rect,
            checkbox_rect,
        );
        let starting_label = text_shape_rect(&output, &texts.starting_frame(STARTING_FRAME));
        let frame1_label = text_shape_rect(&output, &format!("Frame1: {FRAME1}"));
        let starting_font_size =
            text_shape_font_size(&output, &texts.starting_frame(STARTING_FRAME));
        let frame1_font_size = text_shape_font_size(&output, &format!("Frame1: {FRAME1}"));
        let checkbox_label = text_shape_rect(&output, "現在位置を固定");
        assert!(
            (starting_font_size - frame1_font_size).abs() < f32::EPSILON,
            "StartingFrame font size {starting_font_size:.2} differs from Frame1 font size {frame1_font_size:.2} at {actual_ppp:.2} px/pt",
        );
        assert!(
            (starting_font_size - 14.0 * text_scale).abs() < f32::EPSILON,
            "Timeline header should use the enlarged main scale: expected {:.2}, got {:.2}",
            14.0 * text_scale,
            starting_font_size,
        );
        assert!(starting_label.right() <= summary_rect.right() + 0.5);
        assert!(frame1_label.right() <= summary_rect.right() + 0.5);
        assert!(checkbox_label.left() >= checkbox_rect.left() - 0.5);
        assert!(checkbox_label.right() <= checkbox_rect.right() + 0.5);
    }
}
