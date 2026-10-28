//! 孵化タブの案内表示。
//!
//! 画面はアプリケーション状態が作成した不変の案内スナップショットだけを描画し、
//! 検索とタイマーの計算は状態管理側で行う。

use crate::application::state_types::HatchGuidance;
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

fn next_circle_count_text(
    guidance: Option<crate::application::state_types::HatchGuidance>,
    target_dirty: bool,
    guidance_searching: bool,
) -> (String, bool) {
    if target_dirty {
        ("—".into(), false)
    } else if let Some(value) = guidance.filter(|value| value.next_circle_frame.is_some()) {
        // 現在Frameの到達判定がWorkerから届く前でも、先の○が既知ならその数を
        // 維持する。未計算状態を優先すると、計算チャンクの境目で数字が`?`へ
        // 一瞬切り替わり、また数字へ戻るちらつきが起きる。
        (value.blinks_until_circle.to_string(), false)
    } else if guidance.is_some_and(|value| value.current_reachability.is_none())
        || guidance_searching
    {
        ("?".into(), true)
    } else {
        ("—".into(), false)
    }
}

pub(crate) fn hatch_guidance_ui(
    ui: &mut egui::Ui,
    guidance: Option<HatchGuidance>,
    target_dirty: bool,
    guidance_searching: bool,
    height: f32,
    scale: f32,
    color_scheme: ColorScheme,
    texts: &Texts,
) {
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
                // Target入力中は到達判定を更新せず、判定表示を横棒で保留する。
                // 既知の次の○がある場合は、現在Frameの判定更新中も値を維持する。
                let (count_label, count_unknown) =
                    next_circle_count_text(guidance, target_dirty, guidance_searching);
                let count_color = if count_unknown {
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

#[cfg(test)]
mod tests {
    use super::next_circle_count_text;
    use crate::application::state_types::HatchGuidance;

    fn guidance(
        current_reachability: Option<bool>,
        next_circle_frame: Option<i64>,
    ) -> HatchGuidance {
        HatchGuidance {
            current_frame: 100,
            current_reachability,
            reachable_now: current_reachability == Some(true),
            blinks_until_circle: 7,
            next_circle_frame,
            next_circle_seconds: None,
            grace_seconds: None,
            target_wait_seconds: None,
            target_unreachable: false,
            search_out_of_range: false,
        }
    }

    #[test]
    fn known_next_circle_stays_visible_while_current_result_is_pending() {
        assert_eq!(
            next_circle_count_text(Some(guidance(None, Some(107))), false, true),
            ("7".into(), false)
        );
    }

    #[test]
    fn unknown_count_is_used_only_when_no_next_circle_is_known() {
        assert_eq!(
            next_circle_count_text(Some(guidance(None, None)), false, true),
            ("?".into(), true)
        );
        assert_eq!(
            next_circle_count_text(Some(guidance(Some(false), None)), false, false),
            ("—".into(), false)
        );
    }

    #[test]
    fn target_edit_keeps_the_count_hidden() {
        assert_eq!(
            next_circle_count_text(Some(guidance(Some(false), Some(107))), true, false),
            ("—".into(), false)
        );
    }
}
