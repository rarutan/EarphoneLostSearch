//! 通常タイマー画面の描画を担当する。
//!
//! 区間の計時と通知音はアプリケーション状態が管理し、このモジュールは
//! 受け取った表示値をレイアウトへ配置する。

use crate::application::state::NormalTimerState;
use crate::domain::timing::game_frames_to_seconds;
use crate::ui::i18n::Texts;
use crate::ui::layout::PANEL_GAP;
use crate::ui::widgets::{
    draw_timer_flash, frame_spinner, nonnegative_frame_spinner, wide_button, ColorScheme,
};
use eframe::egui;

const MIN_TIMER_PANEL_HEIGHT: f32 = 380.0;
const PANEL_INSET: f32 = 12.0;
const PANEL_PADDING: f32 = 8.0;
const NEXT_LABEL_HEIGHT: f32 = 24.0;
const TIMER_DISPLAY_HEIGHT: f32 = 112.0;
const ACTION_BUTTON_HEIGHT: f32 = 32.0;
const ACTION_BUTTON_BOTTOM_MARGIN: f32 = 40.0;
const WAIT_INPUT_WIDTH: f32 = 120.0;
const OFFSET_INPUT_WIDTH: f32 = 100.0;
const OFFSET_SCROLL_RESERVE: f32 = 78.0;
const MIN_OFFSET_SCROLL_HEIGHT: f32 = 80.0;

pub(crate) fn normal_timer_ui(
    ui: &mut egui::Ui,
    timer: &mut NormalTimerState,
    fps: f64,
    timer_flashing: bool,
    color_scheme: ColorScheme,
    wheel_offset_enabled: bool,
    texts: &Texts,
) -> (bool, bool) {
    let running = timer.is_running();
    let mut open_manager = false;
    let plan = timer.segment_plan().ok();
    let seconds = if running {
        timer.remaining()
    } else if timer.finished {
        0.0
    } else {
        plan.as_ref()
            .and_then(|segments| segments.first())
            .map(|(_, frames)| game_frames_to_seconds(*frames as f64, fps))
            .unwrap_or(0.0)
    };
    let next_seconds = if running {
        timer.next_segment_seconds(fps)
    } else if timer.finished {
        None
    } else {
        plan.as_ref()
            .and_then(|segments| segments.get(1))
            .map(|(_, frames)| game_frames_to_seconds(*frames as f64, fps))
    };

    let width = ui.available_width();
    let height = ui.available_height().max(MIN_TIMER_PANEL_HEIGHT);
    let (_, panels_rect) = ui.allocate_space(egui::vec2(width, height));
    // 固定した8pxの間隔を除き、タイマーと設定を2:1の幅で配置してタイマーを主表示にする。
    let panel_width = (width - PANEL_GAP).max(0.0);
    let left_width = panel_width * (2.0 / 3.0);
    let right_width = (panel_width - left_width).max(0.0);
    let left_rect = egui::Rect::from_min_size(
        panels_rect.min,
        egui::vec2(left_width, panels_rect.height()),
    );
    let right_rect = egui::Rect::from_min_size(
        egui::pos2(left_rect.right() + PANEL_GAP, panels_rect.top()),
        egui::vec2(right_width, panels_rect.height()),
    );

    let mut toggle = false;
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(left_rect), |ui| {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_size(left_rect.size() - egui::vec2(PANEL_INSET, PANEL_INSET));
            let panel_rect = ui.max_rect();
            let next_rect = egui::Rect::from_min_size(
                panel_rect.min + egui::vec2(PANEL_PADDING, PANEL_PADDING),
                egui::vec2(panel_rect.width() - PANEL_PADDING * 2.0, NEXT_LABEL_HEIGHT),
            );
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(next_rect), |ui| {
                ui.label(
                    next_seconds
                        .map(|value| texts.next_seconds(value))
                        .unwrap_or_else(|| " ".into()),
                );
            });
            let timer_rect = egui::Rect::from_center_size(
                panel_rect.center(),
                egui::vec2(
                    panel_rect.width() - PANEL_PADDING * 2.0,
                    TIMER_DISPLAY_HEIGHT,
                ),
            );
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(timer_rect), |ui| {
                ui.with_layout(
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    |ui| {
                        ui.label(
                            egui::RichText::new(texts.seconds(seconds))
                                .size(86.0)
                                .strong()
                                .color(color_scheme.colors_for_ui(ui).timer_text),
                        );
                    },
                );
            });
            let button_rect = egui::Rect::from_min_size(
                egui::pos2(
                    panel_rect.left() + PANEL_PADDING,
                    panel_rect.bottom() - ACTION_BUTTON_BOTTOM_MARGIN,
                ),
                egui::vec2(
                    panel_rect.width() - PANEL_PADDING * 2.0,
                    ACTION_BUTTON_HEIGHT,
                ),
            );
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(button_rect), |ui| {
                let button_label = if running {
                    texts.cancel_space()
                } else {
                    texts.start_space()
                };
                toggle = wide_button(ui, true, button_label);
            });
        });
    });
    draw_timer_flash(ui, left_rect, timer_flashing, color_scheme);

    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(right_rect), |ui| {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_size(right_rect.size() - egui::vec2(PANEL_INSET, PANEL_INSET));
            ui.add_enabled_ui(!running, |ui| {
                ui.horizontal(|ui| {
                    ui.label(texts.actual_wait());
                    nonnegative_frame_spinner(
                        ui,
                        &mut timer.wait_frames,
                        WAIT_INPUT_WIDTH,
                        2,
                        wheel_offset_enabled,
                    );
                });

                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(texts.offsets());
                    if ui.button(format!("＋{}", texts.add())).clicked() {
                        timer.offsets.push("0".into());
                        timer.offset_enabled.push(true);
                    }
                    if ui.button(format!("＋{}", texts.edit())).clicked() {
                        open_manager = true;
                    }
                });

                egui::ScrollArea::vertical()
                    .id_salt("normal_timer_offsets")
                    .max_height(
                        (ui.available_height() - OFFSET_SCROLL_RESERVE)
                            .max(MIN_OFFSET_SCROLL_HEIGHT),
                    )
                    .show(ui, |ui| {
                        if !timer.offsets.is_empty() {
                            while timer.offset_enabled.len() < timer.offsets.len() {
                                timer.offset_enabled.push(true);
                            }
                            let mut remove = None;
                            for (index, value) in timer.offsets.iter_mut().enumerate() {
                                ui.horizontal(|ui| {
                                    ui.label(format!("{}:", index + 1));
                                    ui.checkbox(&mut timer.offset_enabled[index], "");
                                    frame_spinner(
                                        ui,
                                        value,
                                        OFFSET_INPUT_WIDTH,
                                        2,
                                        wheel_offset_enabled,
                                    );
                                    if ui.button(texts.delete()).clicked() {
                                        remove = Some(index);
                                    }
                                });
                            }
                            if let Some(index) = remove {
                                timer.offsets.remove(index);
                                timer.offset_enabled.remove(index);
                            }
                        }
                    });
            });

            match timer.total_frames() {
                Ok(total) => ui.label(texts.total_summary(
                    total,
                    game_frames_to_seconds(total as f64, fps),
                    fps,
                )),
                Err(message) => ui.colored_label(color_scheme.colors_for_ui(ui).error, message),
            };
        });
    });
    (toggle, open_manager)
}
