//! 孵化タブの案内表示。
//!
//! 画面はアプリケーション状態が作成した不変の案内スナップショットだけを描画し、
//! 検索とタイマーの計算は状態管理側で行う。

use crate::application::state::AppState;
use crate::ui::i18n::Texts;
use crate::ui::widgets::ColorScheme;
use eframe::egui;
use std::time::Duration;

const LEFT_COLUMN_RATIO: f32 = 1.0 / 3.0;
const MIN_LEFT_COLUMN_WIDTH: f32 = 160.0;
const ROW_COUNT: f32 = 3.0;
const MIN_ROW_HEIGHT: f32 = 32.0;
const COUNT_LABEL_SIZE: f32 = 20.0;
const COUNT_VALUE_SIZE: f32 = 42.0;
const NEXT_LABEL_SIZE: f32 = 14.0;
const UNREACHABLE_LABEL_SIZE: f32 = 13.0;
const SYMBOL_HEIGHT_RATIO: f32 = 0.65;
const MIN_SYMBOL_SIZE: f32 = 110.0;
const MAX_SYMBOL_SIZE: f32 = 170.0;

pub(crate) fn hatch_guidance_ui(
    ui: &mut egui::Ui,
    state: &mut AppState,
    height: f32,
    scale: f32,
    color_scheme: ColorScheme,
    texts: &Texts,
) {
    let guidance = state.hatch_guidance();
    let guidance_searching = state.guidance_search_pending();
    if guidance_searching {
        ui.ctx().request_repaint_after(Duration::from_millis(20));
    }
    ui.set_min_height(height);
    // 左側の案内と右側の判定記号は、おおむね1:2の幅で配置する。
    let left_width = (ui.available_width() * LEFT_COLUMN_RATIO).max(MIN_LEFT_COLUMN_WIDTH);
    let row_height = (height / ROW_COUNT).max(MIN_ROW_HEIGHT);
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(left_width, height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                let current_known =
                    guidance.is_some_and(|value| value.current_reachability.is_some());
                let count_label = if state.target_dirty {
                    // Target入力中は到達判定を更新せず、判定表示を横棒で保留する。
                    // 未確定値を`?`で表示すると入力途中に案内が変化して見えるため、
                    // Target確定後にまとめて再計算する。
                    "—".into()
                } else if !current_known {
                    "?".into()
                } else if guidance.is_some_and(|value| value.next_circle_frame.is_some()) {
                    guidance
                        .map(|value| value.blinks_until_circle.to_string())
                        .unwrap_or_else(|| "—".into())
                } else if guidance_searching {
                    "?".into()
                } else {
                    "—".into()
                };
                let count_color = if count_label == "?" {
                    color_scheme.colors_for_ui(ui).neutral
                } else if guidance.is_some_and(|value| value.next_circle_frame.is_some()) {
                    color_scheme.colors_for_ui(ui).timer_text
                } else {
                    color_scheme.colors_for_ui(ui).neutral
                };
                ui.allocate_ui_with_layout(
                    egui::vec2(left_width, row_height),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.label(
                            egui::RichText::new(texts.next_circle_count())
                                .size(COUNT_LABEL_SIZE * scale)
                                .strong(),
                        );
                    },
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(left_width, row_height),
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    |ui| {
                        ui.label(
                            egui::RichText::new(count_label)
                                .size(COUNT_VALUE_SIZE * scale)
                                .strong()
                                .color(count_color),
                        );
                    },
                );
                // ○×案内欄の`next >>`は、観測した○の猶予時間を示す。
                // モード別の待機秒数は左側タイマーパネルの右上へ表示し、
                // この欄とは役割を分ける。
                let next_label = if guidance.is_some_and(|value| value.search_out_of_range) {
                    texts.search_out_of_range().to_string()
                } else if guidance.is_some_and(|value| value.target_unreachable) {
                    texts.target_unreachable().to_string()
                } else if guidance.is_some_and(|value| {
                    value.next_circle_frame.is_some() && value.grace_seconds.is_some()
                }) {
                    texts.next_seconds_optional(guidance.and_then(|value| value.grace_seconds))
                } else if guidance_searching {
                    texts.next_calculating()
                } else {
                    texts.next_seconds_optional(None)
                };
                ui.allocate_ui_with_layout(
                    egui::vec2(left_width, row_height),
                    egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                    |ui| {
                        let is_unreachable = guidance.is_some_and(|value| value.target_unreachable);
                        let is_out_of_range =
                            guidance.is_some_and(|value| value.search_out_of_range);
                        ui.label(
                            egui::RichText::new(next_label)
                                .size(
                                    (if is_unreachable || is_out_of_range {
                                        UNREACHABLE_LABEL_SIZE
                                    } else {
                                        NEXT_LABEL_SIZE
                                    }) * scale,
                                )
                                .color(if is_unreachable {
                                    color_scheme.colors_for_ui(ui).target_no
                                } else if is_out_of_range {
                                    color_scheme.colors_for_ui(ui).neutral
                                } else if guidance.is_some() && !guidance_searching {
                                    color_scheme.colors_for_ui(ui).timer_text
                                } else {
                                    color_scheme.colors_for_ui(ui).neutral
                                }),
                        );
                    },
                );
            },
        );
        let right_width = (ui.available_width()).max(0.0);
        ui.allocate_ui_with_layout(
            egui::vec2(right_width, height),
            egui::Layout::centered_and_justified(egui::Direction::TopDown),
            |ui| {
                let (symbol, color) = match guidance.and_then(|value| value.current_reachability) {
                    Some(true) => ("○", color_scheme.colors_for_ui(ui).target_yes),
                    Some(false) => ("×", color_scheme.colors_for_ui(ui).target_no),
                    None => ("—", color_scheme.colors_for_ui(ui).neutral),
                };
                ui.label(
                    egui::RichText::new(symbol)
                        .size(
                            (height * SYMBOL_HEIGHT_RATIO).clamp(MIN_SYMBOL_SIZE, MAX_SYMBOL_SIZE),
                        )
                        .strong()
                        .color(color),
                );
            },
        );
    });
}
