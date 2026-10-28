use super::*;

pub(super) fn field_observation_ui(
    ui: &mut egui::Ui,
    state: &mut FieldState,
    calculating: bool,
    fps: f64,
    color_scheme: ColorScheme,
    texts: &Texts,
) -> (bool, bool, bool, bool, bool) {
    ui.heading(texts.observation());
    let target_before = (state.target_model.clone(), state.target_mode);
    let before = (
        state.range_start.clone(),
        state.range_end.clone(),
        state.tolerance.clone(),
    );
    let editable =
        !calculating && !state.timer_is_running() && state.can_edit_observation_settings();
    let mut search_requested = false;
    let mut cancelled = false;
    let mut observe_requested = false;
    let mut correction_requested = false;

    // 観測の開始・キャンセル・リセットは見出し直下に固定する。
    ui.add_space(3.0);
    if state.is_observing() {
        if wide_button(ui, true, texts.cancel_observation()) {
            state.cancel_observation();
            cancelled = true;
        }
    } else if state.results.is_empty() {
        if wide_button(ui, !calculating, texts.start_observation()) && state.configs().is_ok() {
            state.start_observation();
        }
    } else if wide_button(ui, true, texts.reset_observation()) {
        state.cancel_observation();
        cancelled = true;
    }

    if state.timeleap_frame1_has_passed() {
        ui.horizontal(|ui| {
            ui.label(texts.ui_label("再検索範囲（Frame1） ±"));
            ui.add_enabled_ui(state.can_edit_timeleap_research_range(), |ui| {
                numeric_text_edit(ui, &mut state.timeleap_search_range, 58.0, false, false);
            });
            ui.label("F");
        });
    } else {
        ui.add_enabled_ui(editable, |ui| {
            ui.horizontal(|ui| {
                ui.label(texts.range());
                // 孵化側の観測欄と同じ幅にする。フィールド側だけ広いと、
                // 同じ横一列にある Set ボタンが押しつぶされる。
                numeric_text_edit(ui, &mut state.range_start, 72.0, false, false);
                ui.label(texts.range_separator());
                numeric_text_edit(ui, &mut state.range_end, 72.0, false, false);
                ui.label(texts.start_plus());
                numeric_text_edit(ui, &mut state.range_after_start, 62.0, false, false);
                if ui.button(texts.set()).clicked() {
                    state.set_range_from_start();
                }
            });
        });
    }
    let recheck_radius_active = state.timeleap_recheck_position_radius_active();
    let tolerance_input_enabled = if recheck_radius_active {
        !calculating && !state.timeleap_shift_resolution_pending()
    } else {
        !calculating && !state.timeleap_accepts_shift()
    };
    ui.add_enabled_ui(tolerance_input_enabled, |ui| {
        ui.horizontal(|ui| {
            if recheck_radius_active {
                ui.label(texts.ui_label("予測現在地 ±"));
            } else {
                ui.label(texts.tolerance());
            }
            numeric_text_edit(ui, &mut state.tolerance, 72.0, false, false);
            if recheck_radius_active {
                ui.label("F");
            }
        });
    });
    let row_width = ui.available_width();
    ui.allocate_ui_with_layout(
        egui::vec2(row_width, 24.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.add_enabled_ui(editable, |ui| {
                ui.label(texts.model_number());
                let response = numeric_text_edit(ui, &mut state.target_model, 72.0, false, false);
                if response.lost_focus() && state.target_model.trim().is_empty() {
                    state.target_model = "0".to_string();
                }
            });
            if let Some(matched) = state.selected_npc_number_match() {
                let remaining = ui.available_width();
                ui.allocate_ui_with_layout(
                    egui::vec2(remaining, 24.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        let color = if matched {
                            color_scheme.colors_for_ui(ui).target_yes
                        } else {
                            color_scheme.colors_for_ui(ui).target_no
                        };
                        ui.colored_label(
                            color,
                            format!(
                                "{}:{}",
                                texts.model_number(),
                                if matched { "○" } else { "×" }
                            ),
                        );
                    },
                );
            }
        },
    );
    let after = (
        state.range_start.clone(),
        state.range_end.clone(),
        state.tolerance.clone(),
    );
    if editable && before != after {
        state.invalidate_search_results();
        search_requested = state.intervals.len() >= crate::application::state::MIN_BLINK_INTERVALS;
    }
    let target_after = (state.target_model.clone(), state.target_mode);
    if editable && target_before != target_after {
        state.invalidate_search_results();
        search_requested = state.intervals.len() >= crate::application::state::MIN_BLINK_INTERVALS;
    }

    // 瞬き間隔と検索結果はその直下へ置く。
    let visible_intervals = state
        .timeleap_recheck_intervals(fps)
        .unwrap_or_else(|| state.intervals.clone());
    let intervals_label = if visible_intervals.is_empty() {
        format!("{}: —", texts.observation_intervals_frames())
    } else {
        format!(
            "{}: {}",
            texts.observation_intervals_frames(),
            visible_intervals
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let intervals_width = ui.available_width();
    ui.allocate_ui_with_layout(
        egui::vec2(intervals_width, 22.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.add(
                egui::Label::new(intervals_label.clone())
                    .truncate()
                    .halign(egui::Align::LEFT),
            )
            .on_hover_text(intervals_label);
        },
    );
    // 明示的な遷移ボタンは孵化タブと同じ左下枠に常設する。実行中Timelineを停止し、
    // 実測Frame／idx補正欄を解放する唯一の操作で、Enterも同じ遷移を呼び出す。
    let shift_resolution_pending = state.timeleap_shift_resolution_pending();
    let timeleap_waiting = state.timeleap_accepts_shift();
    let timeleap_frame1_waiting = state.timeleap_frame1_waiting();
    let timeleap_failed = state.timeleap_failed();
    let timeleap_succeeded = state.timeleap_succeeded();
    let timeleap_assumed = state.timeleap_assumed();
    let transition_is_blink_input = state.is_observing() || state.results.is_empty();
    let timeleap_ready_enabled = state.timeleap_enabled
        && !state.timeleap_frame1_preparing
        && state.has_timeleap_frame1_preview()
        && !calculating
        && state.results.len() == 1
        && state.timeleap_frame1_input.trim().parse::<i64>().is_ok();
    let transition_enabled = if state.is_correction_mode() {
        false
    } else if timeleap_frame1_waiting {
        true
    } else if state.timeleap_frame1_preparing || timeleap_waiting || shift_resolution_pending {
        false
    } else if timeleap_failed || timeleap_succeeded || timeleap_assumed {
        true
    } else if state.timeleap_enabled && !transition_is_blink_input {
        timeleap_ready_enabled
    } else if transition_is_blink_input {
        state.is_observing()
    } else {
        !calculating && state.results.len() == 1 && !state.is_correction_mode()
    };
    let transition_label = if state.is_correction_mode() {
        texts.correction()
    } else if state.timeleap_frame1_preparing {
        texts.ui_label("Frame1準備中…")
    } else if state.timeleap_enabled && state.timeleap_frame1_preview_is_pending() {
        texts.ui_label("Frame1計算中…")
    } else if shift_resolution_pending {
        texts.ui_label("瞬き判定中…")
    } else if timeleap_frame1_waiting {
        texts.ui_label("キャンセル")
    } else if timeleap_failed {
        texts.ui_label("リトライ")
    } else if timeleap_succeeded {
        if state.target_is_passed(fps) {
            texts.ui_label("ずれ確認")
        } else {
            texts.ui_label("瞬き状態を再確認")
        }
    } else if timeleap_assumed {
        if state.target_is_passed(fps) {
            texts.ui_label("ずれ確認")
        } else {
            texts.ui_label("瞬き状態を再確認")
        }
    } else if timeleap_waiting {
        texts.record_blink()
    } else if state.timeleap_enabled && !transition_is_blink_input {
        "Ready (Enter)"
    } else if transition_is_blink_input {
        texts.record_blink()
    } else {
        texts.correction_start()
    };
    let panel_rect = ui.max_rect();
    let transition_rect = egui::Rect::from_min_size(
        egui::pos2(panel_rect.left() + 4.0, panel_rect.bottom() - 36.0),
        egui::vec2((panel_rect.width() - 8.0).max(0.0), 30.0),
    );
    let mut transition_clicked = false;
    ui.add_enabled_ui(transition_enabled, |ui| {
        transition_clicked = ui
            .put(
                transition_rect,
                egui::Button::new(transition_label).min_size(egui::vec2(
                    transition_rect.width(),
                    transition_rect.height(),
                )),
            )
            .clicked();
    });
    let mut timeleap_ready_cancel_requested = false;
    if transition_clicked {
        if timeleap_frame1_waiting {
            timeleap_ready_cancel_requested = true;
        } else if timeleap_waiting || timeleap_failed || timeleap_succeeded || timeleap_assumed {
            correction_requested = true;
        } else if state.is_observing() {
            observe_requested = true;
        } else {
            correction_requested = true;
        }
    }
    (
        search_requested,
        cancelled,
        observe_requested,
        correction_requested,
        timeleap_ready_cancel_requested,
    )
}
