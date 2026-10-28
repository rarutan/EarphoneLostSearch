use super::*;
use crate::application::field::timeleap_frame1_offset_drift;
use std::sync::Arc;

#[test]
fn correction_mode_does_not_schedule_live_single_timeline_repaints() {
    assert!(!should_refresh_single_field_timeline(1, true));
    assert!(should_refresh_single_field_timeline(1, false));
    assert!(!should_refresh_single_field_timeline(0, false));
}

fn app_with_empty_field_timeline() -> UiApp {
    let mut app = UiApp::default();
    app.field = FieldState::empty_timeline_for_test();
    app.field_timer_retry = true;
    app
}

const HIGH_START: i64 = 49_500_000;
const HIGH_END: i64 = HIGH_START + 100;
const TEST_SEED: u32 = 0x5C72_AE19;

fn first_text_rect(output: &egui::FullOutput, expected: &str) -> egui::Rect {
    output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.job.text == expected => {
                Some(text.visual_bounding_rect())
            }
            _ => None,
        })
        .min_by(|left, right| left.top().total_cmp(&right.top()))
        .unwrap_or_else(|| panic!("text shape {expected:?} was not rendered"))
}

fn divider_geometry_below_npc(output: &egui::FullOutput) -> (f32, f32) {
    let npc = first_text_rect(output, "NPC数");
    let (divider_y, divider_width) = output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::LineSegment { points, .. }
                if (points[0].y - points[1].y).abs() < 0.1
                    && (points[0].x - points[1].x).abs() > 100.0
                    && points[0].y > npc.bottom() =>
            {
                Some((points[0].y, (points[1].x - points[0].x).abs()))
            }
            _ => None,
        })
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .expect("horizontal divider should follow the NPC row");
    (divider_y - npc.bottom(), divider_width)
}

fn render_hatch_condition_panel() -> egui::FullOutput {
    let context = egui::Context::default();
    install_app_style(&context, ColorScheme::Standard);
    let mut state = AppState::default();
    let texts = Texts::new(Language::Japanese);
    context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1_300.0, 800.0),
            )),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                fixed_group_panel(ui, |ui| {
                    let _ = input_ui(ui, &mut state, false, false, &texts);
                });
            });
        },
    )
}

fn render_field_condition_panel(
    search_running: bool,
    target_research_pending: bool,
    multiple_npc_search: bool,
    timeleap_search_window_open: bool,
) -> (egui::FullOutput, bool) {
    let context = egui::Context::default();
    install_app_style(&context, ColorScheme::Standard);
    let mut state = if multiple_npc_search {
        FieldState::empty_timeline_for_test()
    } else {
        FieldState::default()
    };
    state.target_research_pending = target_research_pending;
    state.multiple_npc_search = multiple_npc_search;
    state.timeleap_search_window_open = timeleap_search_window_open;
    let workers = FieldUiWorkerState {
        search_running,
        ..FieldUiWorkerState::default()
    };
    let texts = Texts::new(Language::Japanese);
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1_300.0 + LEFT_PANEL_WIDTH + PANEL_GAP, 800.0),
            )),
            ..egui::RawInput::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                let _ = field_ui(
                    ui,
                    &mut state,
                    workers,
                    context,
                    30.0,
                    false,
                    false,
                    ColorScheme::Standard,
                    &texts,
                    false,
                );
            });
        },
    );
    (output, state.timeleap_search_window_open)
}

#[test]
fn field_npc_divider_matches_hatch_across_result_statuses() {
    let (hatch_gap, hatch_divider_width) =
        divider_geometry_below_npc(&render_hatch_condition_panel());
    for (
        search_running,
        target_research_pending,
        multiple_npc_search,
        timeleap_search_window_open,
    ) in [
        (false, false, false, false),
        (true, false, false, false),
        (false, true, false, false),
        (false, false, true, false),
        (false, false, false, true),
    ] {
        let (field_output, window_stayed_open) = render_field_condition_panel(
            search_running,
            target_research_pending,
            multiple_npc_search,
            timeleap_search_window_open,
        );
        if timeleap_search_window_open {
            assert!(
                window_stayed_open,
                "open Timeleap viewport should remain open"
            );
        }
        let (field_gap, field_divider_width) = divider_geometry_below_npc(&field_output);
        assert!(
            (field_gap - hatch_gap).abs() <= 1.0,
            "Field NPC divider gap {field_gap:.2}px should match Hatch {hatch_gap:.2}px (search_running={search_running}, target_research_pending={target_research_pending}, multiple_npc_search={multiple_npc_search}, timeleap_search_window_open={timeleap_search_window_open})"
        );
        assert!(
            (field_divider_width - hatch_divider_width).abs() <= 1.0,
            "test panels should have equal content widths: Field={field_divider_width:.2}px, Hatch={hatch_divider_width:.2}px"
        );
    }
}

mod persistence;
mod timer;
mod viewport;
mod worker;
