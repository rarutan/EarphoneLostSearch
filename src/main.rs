#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod application;
mod domain;
mod infrastructure;
mod ui;

// 下段パネルのボタン1つ分の余白を取り除いた固定ウィンドウ高。
const ACTION_BUTTON_HEIGHT: f32 = 32.0;
// 孵化条件枠のオフセット行右側に残っていた余白を取り除く。
const WINDOW_WIDTH: f32 = 1100.0 - ACTION_BUTTON_HEIGHT * 1.5;
// 孵化条件枠の開始位置行が下端へ食い込まないよう、半ボタン分だけ高さを確保する。
const WINDOW_HEIGHT: f32 = 660.0 - ACTION_BUTTON_HEIGHT * 1.2 + ACTION_BUTTON_HEIGHT * 0.5;

fn main() -> eframe::Result {
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([WINDOW_WIDTH, WINDOW_HEIGHT])
        .with_min_inner_size([WINDOW_WIDTH, WINDOW_HEIGHT])
        .with_max_inner_size([WINDOW_WIDTH, WINDOW_HEIGHT])
        .with_resizable(false);

    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/earphone_lost_search_amber_cyan.png"
    ))) {
        viewport = viewport.with_icon(icon);
    }

    eframe::run_native(
        "EarphoneLostSearch",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(ui::UiApp::new(cc)))),
    )
}
