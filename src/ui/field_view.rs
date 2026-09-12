use crate::application::field::{
    FieldIdxInference, FieldSearchOutcome, FieldSearchPool, FieldState, FieldTimerKind,
};
use crate::application::state_types::SessionPhase;
use crate::ui::i18n::Texts;
use crate::ui::layout::{
    LEFT_PANEL_WIDTH, PANEL_GAP, PANEL_PADDING, TIMER_PANEL_HEIGHT as TIMER_PANEL_HEIGHT_SHARED,
    TOP_PANEL_HEIGHT,
};
use crate::ui::widgets::{
    draw_timer_flash, fixed_group_panel, horizontal_frame_spinner,
    nonnegative_frame_spinner_with_commit, npc_spinner, numeric_text_edit,
    offset_spinner_with_commit, wide_button, ColorScheme, COMPACT_ACTION_SIZE,
};
use eframe::egui;
use std::thread::JoinHandle;

const TIMER_PANEL_HEIGHT: f32 = TIMER_PANEL_HEIGHT_SHARED;
const TIMER_SWITCH_SIZE: egui::Vec2 = egui::vec2(36.0, 36.0);
const CORRECTION_LABEL_SIZE: egui::Vec2 = egui::vec2(72.0, 24.0);
const CORRECTION_CONTROL_SIZE: egui::Vec2 = egui::vec2(156.0, 32.0);
const TABLE_SFMT_COLUMN_WIDTH: f32 = 150.0;
const TABLE_SECONDS_COLUMN_WIDTH: f32 = 180.0;
const TABLE_TARGET_BLINKS_COLUMN_WIDTH: f32 = 170.0;
const DERIVED_ROW_HEIGHT: f32 = 24.0;

pub(crate) struct FieldUiActions {
    pub target_committed: bool,
    pub search_requested: bool,
    pub adjust_changed: bool,
    pub cancelled: bool,
    pub observe_requested: bool,
    pub idx_inference_requested: bool,
    pub idx_inference_cancel_requested: bool,
    pub correction_requested: bool,
    pub target_offset_changed: bool,
    pub open_offset_manager: bool,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn field_ui(
    ui: &mut egui::Ui,
    state: &mut FieldState,
    generation: &mut Option<JoinHandle<Result<(FieldSearchOutcome, FieldSearchPool), String>>>,
    pool_generation: &mut Option<JoinHandle<Result<FieldSearchPool, String>>>,
    idx_inference: &mut Option<JoinHandle<Result<FieldIdxInference, String>>>,
    ctx: &egui::Context,
    fps: f64,
    timer_flashing: bool,
    wheel_offset_enabled: bool,
    color_scheme: ColorScheme,
    texts: &Texts,
) -> FieldUiActions {
    state.normalize_timer_kind(fps);
    let worker_running =
        generation.is_some() || pool_generation.is_some() || idx_inference.is_some();
    // Targetだけの再評価では観測候補、Timeline、瞬きタイマーを保持する。フィールド全体を
    // 検索画面へ戻したりTarget欄を固定したりせず、ワーカー結果が届くまで派生Target欄だけを
    // 一時的に空とする。
    let calculating = worker_running && !state.target_research_pending;
    // ワーカー実行中もスピナーを動かす。完了時にも再描画を要求するが、処理中の
    // ネイティブウィンドウが中間状態を描画できるよう一定間隔でも再描画する。
    if worker_running {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
    let text_scale = (ui.available_width() / 1000.0).clamp(1.0, 1.6);
    let top_height = (TOP_PANEL_HEIGHT * text_scale)
        .min(300.0)
        .min(ui.available_height() * 0.55);

    // 孵化タブと同じ固定4区画。候補数が増えても下段が押し下げられない。
    let (_, top_rect) = ui.allocate_space(egui::vec2(ui.available_width(), top_height));
    let mut top_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(top_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let right_panel_width = (top_rect.width() - LEFT_PANEL_WIDTH - PANEL_GAP).max(0.0);
    let left_rect =
        egui::Rect::from_min_size(top_rect.min, egui::vec2(LEFT_PANEL_WIDTH, top_height));
    let right_rect = egui::Rect::from_min_size(
        egui::pos2(left_rect.right() + PANEL_GAP, top_rect.top()),
        egui::vec2(right_panel_width, top_height),
    );

    let mut timer_adjust_changed = false;
    top_ui.allocate_new_ui(
        egui::UiBuilder::new()
            .max_rect(left_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            // 親Uiの現在レイアウトや↕ボタンのallocate順に依存せず、
            // 左パネルそのものの絶対矩形を基準に配置する。
            let panel_rect = left_rect;
            let timeline_unique = !calculating
                && matches!(
                    state.session_phase(),
                    SessionPhase::TimelineReady | SessionPhase::Timer
                );
            let mode_label = match state.timer_kind() {
                FieldTimerKind::NextBlink => texts.next_blink_timer(),
                FieldTimerKind::Target => texts.target_timer(),
            };
            // 2つの隅ラベルは枠の絶対座標へ直接配置する。子レイアウトに任せると、
            // 先に確保された要素（特に↕ボタン）の幅が左端の基準になり、狭い枠で
            // ラベルが目に見えてずれる。
            let label_color = color_scheme.colors_for_ui(ui).text;
            ui.painter().text(
                egui::pos2(
                    panel_rect.left() + PANEL_PADDING,
                    panel_rect.top() + PANEL_PADDING,
                ),
                egui::Align2::LEFT_TOP,
                mode_label,
                egui::FontId::proportional(16.0 * text_scale),
                label_color,
            );
            // 間隔入力中も候補検索は継続する。Timelineが一意になるまで派生する
            // タイマー表示を固定し、途中候補が案内へ混ざらないようにする。
            let label_text = if !timeline_unique {
                "—".to_owned()
            } else {
                match state.timer_kind() {
                    FieldTimerKind::NextBlink => {
                        if state.target_is_passed(fps) {
                            texts.target_passed().to_owned()
                        } else {
                            state
                                .preview_remaining(FieldTimerKind::Target, fps)
                                .map(|seconds| texts.target_seconds(seconds.max(0.0)))
                                .unwrap_or_default()
                        }
                    }
                    FieldTimerKind::Target => {
                        if state.target_is_passed(fps) {
                            texts.target_passed().to_owned()
                        } else {
                            state
                                .preview_next_blink_remaining(fps)
                                .map(|seconds| texts.next_blink_seconds(seconds.max(0.0)))
                                .unwrap_or_else(|| "—".into())
                        }
                    }
                }
            };
            // 補助表示は右上へ固定する。左上のモード名とは独立しており、
            // ↕ボタンの位置やallocate順に影響されない。
            ui.painter().text(
                egui::pos2(
                    panel_rect.right() - PANEL_PADDING,
                    panel_rect.top() + PANEL_PADDING,
                ),
                egui::Align2::RIGHT_TOP,
                label_text,
                egui::FontId::proportional(14.0 * text_scale),
                label_color,
            );
            let timer_rect = egui::Rect::from_center_size(
                egui::pos2(panel_rect.center().x, panel_rect.center().y - 4.0),
                egui::vec2(
                    panel_rect.width() - PANEL_PADDING * 2.0,
                    TIMER_PANEL_HEIGHT * text_scale,
                ),
            );
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(timer_rect), |ui| {
                // 複数候補の先頭行を使ったプレビューは、実タイマーが
                // 動いているように見えてしまう。Timelineが一意になる
                // まではタイマーを開始せず、表示も初期値に固定する。
                let seconds = if timeline_unique {
                    state
                        .preview_remaining(state.timer_kind(), fps)
                        .unwrap_or(0.0)
                } else {
                    0.0
                };
                ui.with_layout(
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    |ui| {
                        ui.label(
                            egui::RichText::new(texts.seconds(seconds))
                                .size(48.0 * text_scale)
                                .strong()
                                .color(color_scheme.colors_for_ui(ui).timer_text),
                        );
                    },
                );
            });
            let switch_rect = egui::Rect::from_center_size(
                egui::pos2(panel_rect.left() + 28.0, panel_rect.center().y),
                TIMER_SWITCH_SIZE,
            );
            // 左上パネル自身へ直接putする。ボタン専用の子Uiを作ると、
            // その子Uiの最小矩形が親の左端として扱われる実装差で、
            // ラベルやタイマーの相対位置がずれることがある。
            let can_toggle = timeline_unique
                && (state.timer_kind() == FieldTimerKind::Target
                    || state.target_timer_available(fps));
            let mut switch_clicked = false;
            ui.add_enabled_ui(!calculating && can_toggle, |ui| {
                switch_clicked = ui.put(switch_rect, egui::Button::new("↕")).clicked();
            });
            if switch_clicked {
                if let Err(message) = state.toggle_timer_kind(fps) {
                    state.set_actionable_status(message);
                }
            }
            // 孵化タブと同じく、補正行は左下へ固定し、ラベルは左、
            // 横向きスピナーはその行の中央へ置く。スピナー自身が`F`を
            // 描画するため外側には追加しない。中央の↕ボタンとは別の行にする。
            let correction_top = panel_rect.bottom() - 4.0 - 32.0;
            let correction_label_rect = egui::Rect::from_min_size(
                egui::pos2(panel_rect.left() + PANEL_PADDING, correction_top + 4.0),
                CORRECTION_LABEL_SIZE,
            );
            ui.put(correction_label_rect, egui::Label::new(texts.correction()));
            let correction_rect = egui::Rect::from_min_size(
                egui::pos2(panel_rect.center().x - 78.0, correction_top),
                CORRECTION_CONTROL_SIZE,
            );
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(correction_rect), |ui| {
                ui.add_enabled_ui(state.timer_kind() == FieldTimerKind::NextBlink, |ui| {
                    let mut adjust_input = state.adjust.to_string();
                    timer_adjust_changed = horizontal_frame_spinner(
                        ui,
                        &mut adjust_input,
                        52.0,
                        1,
                        wheel_offset_enabled,
                    );
                    if timer_adjust_changed {
                        if let Ok(value) = adjust_input.parse::<i64>() {
                            state.adjust_by(value.saturating_sub(state.adjust), fps);
                        }
                    }
                });
            });
        },
    );
    draw_timer_flash(&top_ui, left_rect, timer_flashing, color_scheme);
    top_ui.allocate_new_ui(
        egui::UiBuilder::new()
            .max_rect(right_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| field_timeline_ui(ui, state, calculating, fps, text_scale, color_scheme, texts),
    );

    ui.separator();
    let right_panel_width = (ui.available_width() - LEFT_PANEL_WIDTH - PANEL_GAP).max(0.0);
    let mut actions = FieldUiActions {
        target_committed: false,
        search_requested: false,
        adjust_changed: timer_adjust_changed,
        cancelled: false,
        observe_requested: false,
        idx_inference_requested: false,
        idx_inference_cancel_requested: false,
        correction_requested: false,
        target_offset_changed: false,
        open_offset_manager: false,
    };
    // 孵化タブと同じく、下段の枠をウィンドウ下端まで伸ばす。
    let bottom_height = ui.available_height().max(0.0);
    let (_, bottom_rect) = ui.allocate_space(egui::vec2(ui.available_width(), bottom_height));
    let bottom_left_rect = egui::Rect::from_min_size(
        bottom_rect.min,
        egui::vec2(LEFT_PANEL_WIDTH, bottom_rect.height()),
    );
    let bottom_right_rect = egui::Rect::from_min_size(
        egui::pos2(bottom_left_rect.right() + PANEL_GAP, bottom_rect.top()),
        egui::vec2(right_panel_width, bottom_rect.height()),
    );
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(bottom_left_rect), |ui| {
        fixed_group_panel(ui, |ui| {
            let (search_requested, cancelled, observe_requested, correction_requested) =
                field_observation_ui(ui, state, calculating, color_scheme, texts);
            actions.search_requested = search_requested;
            actions.cancelled = cancelled;
            actions.observe_requested = observe_requested;
            actions.correction_requested = correction_requested;
        });
    });
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(bottom_right_rect), |ui| {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_size(bottom_right_rect.size() - egui::vec2(12.0, 12.0));
            let (
                target_committed,
                idx_inference_requested,
                idx_inference_cancel_requested,
                target_offset_changed,
                open_offset_manager,
            ) = field_settings_ui(
                ui,
                state,
                calculating,
                idx_inference.is_some(),
                fps,
                color_scheme,
                wheel_offset_enabled,
                texts,
            );
            actions.target_committed = target_committed;
            actions.idx_inference_requested = idx_inference_requested;
            actions.idx_inference_cancel_requested = idx_inference_cancel_requested;
            actions.target_offset_changed = target_offset_changed;
            actions.open_offset_manager = open_offset_manager;
        });
    });
    actions
}

fn field_timeline_ui(
    ui: &mut egui::Ui,
    state: &mut FieldState,
    calculating: bool,
    fps: f64,
    scale: f32,
    color_scheme: ColorScheme,
    texts: &Texts,
) {
    // Enterで補正モードへ移った後も、タイマーだけを停止して
    // 観測済みTimelineは閲覧できるようにする。
    let timeline_unique = !calculating && state.results.len() == 1;
    ui.horizontal(|ui| {
        let selected = state.results.get(state.selected_result);
        if timeline_unique {
            if let Some(result) = selected.filter(|result| !result.timeline.is_empty()) {
                ui.label(
                    egui::RichText::new(texts.starting_frame(result.start_consumption))
                        .size(14.0 * scale)
                        .strong(),
                );
                if ui.button(texts.copy()).clicked() {
                    ui.output_mut(|output| {
                        output.copied_text = result.start_consumption.to_string();
                    });
                }
            } else {
                ui.label(
                    egui::RichText::new("StartingFrame : —")
                        .size(14.0 * scale)
                        .strong(),
                );
            }
        } else {
            ui.label(
                egui::RichText::new("StartingFrame : —")
                    .size(14.0 * scale)
                    .strong(),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.checkbox(
                &mut state.table_follow_current,
                texts.timeline_follow_current(),
            );
        });
    });
    let Some(result) = state.results.get(state.selected_result) else {
        return;
    };
    if !timeline_unique {
        return;
    }
    if result.timeline.is_empty() {
        return;
    }

    let current_timeline_frame = state.current_timeline_frame(fps);
    let current_row = state
        .current_timeline_row(fps)
        .unwrap_or(0)
        .min(result.timeline.len().saturating_sub(1));
    let target_timeline_frame = result
        .target_wait_frame
        .map(|wait| result.last_wait_frame.saturating_add(wait));
    ui.horizontal(|ui| {
        for (width, label) in [
            (140.0, texts.timeline_sfmt_frame()),
            (180.0, texts.timeline_next_blink_seconds()),
            (170.0, texts.timeline_target_blink_count()),
        ] {
            ui.add_sized(
                [width * scale, 20.0 * scale],
                egui::Label::new(egui::RichText::new(label).size(13.0 * scale).strong()),
            );
        }
    });

    let row_height = 22.0 * scale;
    // 表の操作に必要なフッターだけを確保する。固定の大きな予約領域は行の下に
    // 不要な余白を残すため、表示に必要な高さへ限定する。
    let table_height = (ui.available_height() - 26.0 * scale).max(80.0 * scale);
    // スクロール内容の上下に余白を設け、先頭・末尾の行もビューポート中央へ置ける
    // ようにする。通常のスクロール値はeguiが内容範囲で丸めるため端行を中央にできない。
    let center_padding = ((table_height - row_height) * 0.5).max(0.0);
    // 追従モードでは毎フレーム補正し、ホイール操作で青い現在行が中央から外れない
    // ようにする。自由モードでは補正せず、任意の行を確認できるようにする。
    if state.table_follow_current {
        state.table_last_followed_row = Some(current_row);
    }
    let scroll = egui::ScrollArea::vertical()
        .id_salt("field-timeline-scroll")
        .max_height(table_height)
        .min_scrolled_height(table_height)
        .auto_shrink([false, false]);
    scroll.show(ui, |ui| {
        ui.add_space(center_padding);
        for (index, row) in result.timeline.iter().enumerate() {
            let (row_rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), row_height),
                egui::Sense::hover(),
            );
            if index == current_row {
                ui.painter()
                    .rect_filled(row_rect, 2.0, color_scheme.colors_for_ui(ui).current_row);
                if state.table_follow_current {
                    // 推定値ではなくeguiの実際の表示領域を使う。スクロールバー、枠の高さ、
                    // DPI、行余白が変わっても正しく、アニメーション1フレームの遅れなく追従する。
                    ui.scroll_to_rect_animation(
                        row_rect,
                        Some(egui::Align::Center),
                        egui::style::ScrollAnimation::none(),
                    );
                }
            }
            ui.allocate_new_ui(
                egui::UiBuilder::new()
                    .max_rect(row_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.add_sized(
                        [TABLE_SFMT_COLUMN_WIDTH * scale, row_height],
                        egui::Label::new(if index == current_row {
                            format!("▶ {}", row.frame)
                        } else {
                            row.frame.to_string()
                        }),
                    );
                    let seconds = crate::domain::timing::game_frames_to_seconds(
                        row.duration_frames.max(0) as f64,
                        fps,
                    );
                    ui.add_sized(
                        [TABLE_SECONDS_COLUMN_WIDTH * scale, row_height],
                        egui::Label::new(format!("{seconds:.2}s")),
                    );
                    let target_blinks = target_timeline_frame
                        .filter(|target| *target > row.timeline_frame)
                        .map(|target| {
                            result
                                .timeline
                                .iter()
                                .filter(|future| {
                                    future.timeline_frame > row.timeline_frame
                                        && future.timeline_frame < target
                                })
                                .count()
                        })
                        .map(|count| count.to_string())
                        .unwrap_or_else(|| "—".into());
                    ui.add_sized(
                        [TABLE_TARGET_BLINKS_COLUMN_WIDTH * scale, row_height],
                        egui::Label::new(target_blinks),
                    );
                },
            );
        }
        ui.add_space(center_padding);
    });
    if current_timeline_frame.is_some() {
        let fallback_sfmt = result.timeline[current_row].frame;
        let current_sfmt = state.current_sfmt_frame(fps).unwrap_or(fallback_sfmt);
        let current_size = egui::vec2(ui.available_width(), 24.0 * scale);
        ui.allocate_ui_with_layout(
            current_size,
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add(egui::Label::new(
                    egui::RichText::new(format!("Current >> {current_sfmt}")).size(14.0 * scale),
                ));
            },
        );
    }
}

fn field_observation_ui(
    ui: &mut egui::Ui,
    state: &mut FieldState,
    calculating: bool,
    color_scheme: ColorScheme,
    texts: &Texts,
) -> (bool, bool, bool, bool) {
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
        if wide_button(ui, !calculating, texts.start_observation()) {
            match state.configs() {
                Ok(_) => state.start_observation(),
                Err(message) => state.set_actionable_status(message),
            }
        }
    } else if wide_button(ui, true, texts.reset_observation()) {
        state.cancel_observation();
        cancelled = true;
    }

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
        ui.horizontal(|ui| {
            ui.label(texts.tolerance());
            numeric_text_edit(ui, &mut state.tolerance, 72.0, false, false);
        });
        ui.horizontal(|ui| {
            ui.label(texts.model_number());
            numeric_text_edit(ui, &mut state.target_model, 72.0, false, false);
        });
    });
    let after = (
        state.range_start.clone(),
        state.range_end.clone(),
        state.tolerance.clone(),
    );
    if editable && before != after {
        state.invalidate_search_results("条件を変更しました。観測結果を再計算します。");
        search_requested = state.intervals.len() >= crate::application::state::MIN_BLINK_INTERVALS;
    }
    let target_after = (state.target_model.clone(), state.target_mode);
    if editable && target_before != target_after {
        state.invalidate_search_results("NPC番号を変更しました。再検索します。");
        search_requested = state.intervals.len() >= crate::application::state::MIN_BLINK_INTERVALS;
    }

    if calculating {
        ui.horizontal(|ui| {
            ui.spinner();
        });
    }
    // 瞬き間隔と検索結果はその直下へ置く。
    let intervals_label = if state.intervals.is_empty() {
        format!("{}: —", texts.observation_intervals_frames())
    } else {
        format!(
            "{}: {}",
            texts.observation_intervals_frames(),
            state
                .intervals
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
    if state.status_is_field_notice() {
        let status = texts.status(&state.status);
        ui.colored_label(color_scheme.colors_for_ui(ui).error, status);
    }
    // 明示的な遷移ボタンは孵化タブと同じ左下枠に常設する。実行中Timelineを停止し、
    // 実測Frame／idx補正欄を解放する唯一の操作で、Enterも同じ遷移を呼び出す。
    let transition_is_blink_input = state.is_observing() || state.results.is_empty();
    let transition_enabled = if transition_is_blink_input {
        state.is_observing()
    } else {
        !calculating && state.results.len() == 1 && !state.is_correction_mode()
    };
    let transition_label = if transition_is_blink_input {
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
    if transition_clicked {
        if state.is_observing() {
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
    )
}

#[allow(clippy::too_many_arguments)]
fn field_settings_ui(
    ui: &mut egui::Ui,
    state: &mut FieldState,
    calculating: bool,
    idx_inference_running: bool,
    fps: f64,
    color_scheme: ColorScheme,
    wheel_offset_enabled: bool,
    texts: &Texts,
) -> (bool, bool, bool, bool, bool) {
    let mut target_committed = false;
    let mut idx_inference_requested = false;
    let mut idx_inference_cancel_requested = false;
    let mut open_offset_manager = false;
    ui.heading(texts.conditions());
    let general_editable =
        !calculating && !state.timer_is_running() && state.can_edit_general_settings();
    let correction_editable =
        !calculating && state.can_edit_correction_inputs() && !state.timer_is_running();
    let seed_npc_editable = general_editable && state.session_phase() == SessionPhase::Input;
    let other_editable = general_editable || correction_editable;
    let before = (
        state.seed.clone(),
        state.npc_count.clone(),
        state.multiple_npc_search,
        state.encounter_offset.clone(),
        state.use_offset,
    );
    let actual_frame_before = state.actual_frame.clone();
    // 孵化側と同じく、基本条件を左、オフセットを右の列へ置く。
    ui.columns(2, |columns| {
        columns[0].vertical(|ui| {
            ui.add_enabled_ui(seed_npc_editable, |ui| {
                ui.horizontal(|ui| {
                    ui.label(texts.initial_seed());
                    ui.add(egui::TextEdit::singleline(&mut state.seed).desired_width(120.0));
                });
            });
            // TargetFrameは観測結果確定後も、明示的にEnterで補正へ進むまでは
            // 瞬きタイマーを動かしたまま編集できる。検索ワーカーの先行計算中も
            // 入力を止めず、確定後にTargetだけを再評価する。
            ui.add_enabled_ui(state.can_edit_target(), |ui| {
                ui.horizontal(|ui| {
                    ui.label(texts.target_frame());
                    let (changed, committed) = nonnegative_frame_spinner_with_commit(
                        ui,
                        &mut state.target_consumption,
                        120.0,
                        1,
                        wheel_offset_enabled,
                    );
                    if changed {
                        state.target_dirty = true;
                    }
                    if committed && state.target_dirty {
                        state.target_dirty = false;
                        if state.target_consumption.trim().is_empty() {
                            state.clear_target_prediction(fps);
                            state.clear_status();
                        } else if state.results.len() == 1 {
                            state.prepare_target_research(
                                fps,
                                "Targetを計算中です。瞬きタイマーは継続します。",
                            );
                            target_committed = true;
                        } else {
                            state.invalidate_search_results(
                                "Targetを変更しました。確定後に再検索します。",
                            );
                            target_committed = true;
                        }
                    }
                });
            });
            ui.horizontal(|ui| {
                // Timeline確定後は検索条件のNPC入力を固定するが、確定したNPC数と
                // そのCopy操作はタイマー実行中も利用できる。
                ui.add_enabled_ui(seed_npc_editable, |ui| {
                    ui.label(texts.model_count());
                    if state.multiple_npc_search {
                        let mut parts = state.npc_count.split(['~', '～']);
                        let mut range_start = parts.next().unwrap_or_default().trim().to_owned();
                        let fallback_end = range_start.clone();
                        let mut range_end = parts.next().unwrap_or(&fallback_end).trim().to_owned();
                        let start_changed =
                            npc_spinner(ui, &mut range_start, 58.0, wheel_offset_enabled);
                        ui.label("~");
                        let end_changed =
                            npc_spinner(ui, &mut range_end, 58.0, wheel_offset_enabled);
                        if start_changed || end_changed {
                            state.npc_count = format!("{}~{}", range_start, range_end);
                        }
                    } else {
                        npc_spinner(ui, &mut state.npc_count, 120.0, wheel_offset_enabled);
                    }
                    let changed = ui
                        .checkbox(&mut state.multiple_npc_search, texts.multiple_search())
                        .changed();
                    if changed {
                        if state.multiple_npc_search {
                            if let Ok(value) = state.npc_count.trim().parse::<usize>() {
                                state.npc_count = format!("{value}~{value}");
                            }
                        } else if let Some(start) = state.npc_count.split(['~', '～']).next() {
                            state.npc_count = start.trim().to_owned();
                        }
                    }
                });
            });
        });
        columns[1].vertical(|ui| {
            ui.add_enabled_ui(
                !calculating && (other_editable || state.results.len() == 1),
                |ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut state.use_offset, texts.offset());
                        ui.add_enabled_ui(state.use_offset, |ui| {
                            let _ = offset_spinner_with_commit(
                                ui,
                                &mut state.encounter_offset,
                                74.0,
                                2,
                                wheel_offset_enabled,
                                -crate::domain::rng::MAX_SFMT_FRAME,
                                crate::domain::rng::MAX_SFMT_FRAME,
                            );
                            ui.label("F");
                            if ui.button("＋").clicked() {
                                open_offset_manager = true;
                            }
                        });
                    });
                },
            );
            // 派生値はオフセットと同じ条件列の右端へ置く。条件ごとに行を分け、
            // オフセット操作部品が他ラベルの配置基準にならないようにする。
            let target_row_height = DERIVED_ROW_HEIGHT;
            if !calculating && state.results.len() == 1 {
                if let Some(result) = state.results.get(state.selected_result) {
                    if let Some(target_exact) = result.target_exact {
                        let (color, label) = if target_exact {
                            (
                                color_scheme.colors_for_ui(ui).target_yes,
                                texts.target_result(true),
                            )
                        } else {
                            (
                                color_scheme.colors_for_ui(ui).target_no,
                                texts.target_result(false),
                            )
                        };
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), target_row_height),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                ui.colored_label(color, label);
                            },
                        );
                    } else {
                        ui.allocate_space(egui::vec2(ui.available_width(), target_row_height));
                    }
                } else {
                    ui.allocate_space(egui::vec2(ui.available_width(), target_row_height));
                }
            } else {
                ui.allocate_space(egui::vec2(ui.available_width(), target_row_height));
            }
            let npc_row_height = DERIVED_ROW_HEIGHT;
            if !calculating
                && state.results.len() == 1
                && state.multiple_npc_search
                && state.results.len() == 1
            {
                if let Some(result) = state.results.get(state.selected_result) {
                    let npc_text = result.models.saturating_sub(1).to_string();
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), npc_row_height),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if ui.button(texts.copy()).clicked() {
                                ui.output_mut(|output| {
                                    output.copied_text = npc_text.clone();
                                });
                            }
                            ui.label(format!("{}: {}", texts.model_count(), npc_text));
                        },
                    );
                } else {
                    ui.allocate_space(egui::vec2(ui.available_width(), npc_row_height));
                }
            } else {
                ui.allocate_space(egui::vec2(ui.available_width(), npc_row_height));
            }
        });
    });
    ui.separator();
    ui.horizontal(|ui| {
        ui.add_space(2.0);
        let actual_frame_valid = state.actual_frame.trim().parse::<i64>().is_ok();
        let idx_button_enabled = idx_inference_running
            || (correction_editable && state.results.len() == 1 && actual_frame_valid);
        let button_label = if idx_inference_running {
            texts.cancel_idx_inference()
        } else {
            texts.correction()
        };
        if ui
            .add_enabled(
                idx_button_enabled,
                egui::Button::new(button_label)
                    .min_size(egui::vec2(COMPACT_ACTION_SIZE[0], COMPACT_ACTION_SIZE[1])),
            )
            .clicked()
        {
            if idx_inference_running {
                idx_inference_cancel_requested = true;
            } else {
                idx_inference_requested = true;
            }
        }
        ui.label(texts.actual_frame());
        ui.add_enabled_ui(correction_editable, |ui| {
            numeric_text_edit(ui, &mut state.actual_frame, 90.0, false, false);
        });
        if idx_inference_running {
            ui.spinner();
        }
    });
    if correction_editable && actual_frame_before != state.actual_frame {
        state.correction_calculated = false;
        state.correction_suggested_offset = None;
        state.inferred_indices.clear();
    }
    let after = (
        state.seed.clone(),
        state.npc_count.clone(),
        state.multiple_npc_search,
        state.encounter_offset.clone(),
        state.use_offset,
    );
    if seed_npc_editable && before != after {
        state.invalidate_search_results("条件を変更しました。観測結果を再計算します。");
        target_committed = state.intervals.len() >= crate::application::state::MIN_BLINK_INTERVALS;
    }
    let target_offset_changed = before.3 != after.3 || before.4 != after.4;
    (
        target_committed,
        idx_inference_requested,
        idx_inference_cancel_requested,
        target_offset_changed,
        open_offset_manager,
    )
}
