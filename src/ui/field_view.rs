use crate::application::field::{FieldState, FieldTimerKind};
use crate::application::state_types::SessionPhase;
use crate::domain::timeleap::TimeleapMode;
use crate::ui::i18n::Texts;
use crate::ui::layout::{
    split_panel_rects, LEFT_PANEL_WIDTH, PANEL_GAP, PANEL_PADDING,
    TIMER_PANEL_HEIGHT as TIMER_PANEL_HEIGHT_SHARED, TOP_PANEL_HEIGHT,
};
use crate::ui::widgets::{
    bounded_integer_text_edit, draw_timer_flash, fixed_group_panel, frame_spinner,
    horizontal_frame_spinner, iv_count_spinner, nonnegative_frame_spinner_with_commit, npc_spinner,
    numeric_text_edit, offset_spinner_with_commit, wide_button, ColorScheme, COMPACT_ACTION_SIZE,
};
use eframe::egui;
use std::time::Instant;
mod observation;
mod settings;
mod timeleap_controls;
mod timeline;
use observation::field_observation_ui;
use settings::field_settings_ui;
#[cfg(test)]
use timeleap_controls::timeleap_main_controls_ui;
use timeleap_controls::timeleap_main_controls_ui_with_texts;
pub(crate) use timeleap_controls::timeleap_window_keepalive_ui;
#[cfg(test)]
use timeleap_controls::TimeleapMainControlsActions;
#[cfg(test)]
use timeleap_controls::TimeleapMainControlsLayout;
#[cfg(test)]
use timeleap_controls::{
    main_timeleap_direct_encounter_enabled, main_timeleap_no_dex_enabled,
    main_timeleap_search_enabled,
};
use timeline::{field_text_scale, field_timeline_ui, format_timeleap_duration};
mod timeleap_search;
use timeleap_search::{
    actual_timeleap_frame1_for_display, apply_timeleap_method_selection,
    centered_search_button_with_texts, confirmed_target_search_tab_ui_with_worker_running,
    help_button, search_button_action, timeleap_candidate_rows_virtualized_ui,
    timeleap_candidate_search_button_with_texts, timeleap_category_pokemon_method_controls,
    timeleap_filter_range_ui_with_texts, timeleap_generation_start_frame_row_with_texts,
    timeleap_main_candidate_header_ui_with_texts, timeleap_main_candidate_rows_ui_at_with_texts,
    timeleap_search_column_widths, timeleap_search_header_ui,
    timeleap_search_metadata_ui_with_texts, timeleap_search_options_ui, SearchButtonAction,
    TimeleapMainCandidateRow, TIMELEAP_TAB_COURIER, TIMELEAP_TAB_FRAME1_SEARCH,
    TIMELEAP_TAB_SETTINGS, TIMELEAP_TAB_TARGET_SEARCH,
};
const TIMER_PANEL_HEIGHT: f32 = TIMER_PANEL_HEIGHT_SHARED;
const TIMER_SWITCH_SIZE: egui::Vec2 = egui::vec2(36.0, 36.0);
const CORRECTION_LABEL_SIZE: egui::Vec2 = egui::vec2(72.0, 24.0);
const CORRECTION_CONTROL_SIZE: egui::Vec2 = egui::vec2(156.0, 32.0);
const TABLE_SFMT_COLUMN_WIDTH: f32 = 150.0;
const TABLE_SECONDS_COLUMN_WIDTH: f32 = 180.0;
const TABLE_TARGET_BLINKS_COLUMN_WIDTH: f32 = 170.0;
const DERIVED_ROW_HEIGHT: f32 = 24.0;
const TIMELEAP_SEARCH_TABLE_ROW_HEIGHT: f32 = 24.0;

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
    pub timeleap_search_requested: bool,
    pub timeleap_search_cancel_requested: bool,
    pub timeleap_candidate_search_cancel_requested: bool,
    pub timeleap_frame1_preview_requested: bool,
    pub timeleap_reachability_search_requested: bool,
    pub confirmed_target_search_requested: bool,
    pub confirmed_target_search_cancel_requested: bool,
    pub confirmed_target_search_row: Option<crate::domain::timeleap::ConfirmedTargetSearchRow>,
    pub confirmed_target_offset_drift_frames: Option<i64>,
    pub timeleap_frame1_offset_frame: Option<i64>,
    pub timeleap_enabled_changed: bool,
    pub timeleap_ready_cancel_requested: bool,
    #[cfg(test)]
    timeleap_controls_layout: TimeleapMainControlsLayout,
    #[cfg(test)]
    timeleap_controls_panel: egui::Rect,
    #[cfg(test)]
    observation_panel: egui::Rect,
}

impl Default for FieldUiActions {
    fn default() -> Self {
        Self {
            target_committed: false,
            search_requested: false,
            adjust_changed: false,
            cancelled: false,
            observe_requested: false,
            idx_inference_requested: false,
            idx_inference_cancel_requested: false,
            correction_requested: false,
            target_offset_changed: false,
            open_offset_manager: false,
            timeleap_search_requested: false,
            timeleap_search_cancel_requested: false,
            timeleap_candidate_search_cancel_requested: false,
            timeleap_frame1_preview_requested: false,
            timeleap_reachability_search_requested: false,
            confirmed_target_search_requested: false,
            confirmed_target_search_cancel_requested: false,
            confirmed_target_search_row: None,
            confirmed_target_offset_drift_frames: None,
            timeleap_frame1_offset_frame: None,
            timeleap_enabled_changed: false,
            timeleap_ready_cancel_requested: false,
            #[cfg(test)]
            timeleap_controls_layout: TimeleapMainControlsLayout::default(),
            #[cfg(test)]
            timeleap_controls_panel: egui::Rect::NOTHING,
            #[cfg(test)]
            observation_panel: egui::Rect::NOTHING,
        }
    }
}

/// フィールド画面が表示に必要とする検索処理の状態。
/// ワーカーハンドルやキャンセル機構は描画層へ渡さない。
#[derive(Clone, Copy, Default)]
pub(crate) struct FieldUiWorkerState {
    pub search_running: bool,
    pub pool_generation_running: bool,
    pub idx_inference_running: bool,
    pub timeleap_search_running: bool,
    pub confirmed_target_search_running: bool,
    pub timeleap_frame1_running: bool,
    pub timeleap_frame1_preview_running: bool,
}

fn field_search_is_calculating(workers: FieldUiWorkerState, target_research_pending: bool) -> bool {
    (workers.search_running || workers.pool_generation_running) && !target_research_pending
}

fn apply_wondercard_settings(
    state: &mut FieldState,
    settings: crate::domain::wondercard::WondercardSettings,
) -> bool {
    state.timeleap_courier_species_id = settings.species_id;
    state.timeleap_courier_tid = settings.tid;
    state.timeleap_courier_sid = settings.sid;
    state.timeleap_courier_ec = settings.ec;
    state.timeleap_courier_pid = settings.pid;
    state.timeleap_courier_pid_type = settings.pid_type;
    state.timeleap_courier_fixed_ivs = settings.fixed_ivs;
    state.timeleap_courier_iv_count = settings.iv_count;
    state.timeleap_courier_ability_locked = settings.ability_locked;
    state.timeleap_courier_ability = settings.ability;
    state.timeleap_courier_nature_locked = settings.nature_locked;
    state.timeleap_courier_nature = settings.nature;
    state.timeleap_courier_gender_locked = settings.gender_locked;
    state.timeleap_courier_gender_ratio = settings.gender;
    state.timeleap_courier_your_id = settings.your_id;
    state.timeleap_courier_is_egg = settings.is_egg;
    state.timeleap_courier_other_info = true;
    state.courier_condition_changed()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn field_ui(
    ui: &mut egui::Ui,
    state: &mut FieldState,
    workers: FieldUiWorkerState,
    ctx: &egui::Context,
    fps: f64,
    timer_flashing: bool,
    wheel_offset_enabled: bool,
    color_scheme: ColorScheme,
    texts: &Texts,
    fastest_close: bool,
) -> FieldUiActions {
    state.normalize_timer_kind(fps);
    // Targetだけの再評価では観測候補、Timeline、瞬きタイマーを保持する。フィールド全体を
    // 検索画面へ戻したりTarget欄を固定したりせず、ワーカー結果が届くまで派生Target欄だけを
    // 一時的に空とする。
    // NPC番号推定は候補検索とは別の補正処理なので、検索中扱いにして確定表示を隠さない。
    let calculating = field_search_is_calculating(workers, state.target_research_pending);
    // ワーカーの完了状態をUIへ反映するため、計算中は定期的に再描画する。
    if calculating {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
    if workers.timeleap_search_running {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
    if workers.timeleap_frame1_running {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
    if workers.timeleap_frame1_preview_running || state.timeleap_frame1_preview_is_pending() {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
    if state.timeleap_search_window_open {
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
    let text_scale = field_text_scale(ui.available_width());
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
    let (left_rect, right_rect) = split_panel_rects(top_rect, LEFT_PANEL_WIDTH, PANEL_GAP);

    let mut timer_adjust_changed = false;
    top_ui.allocate_new_ui(
        egui::UiBuilder::new()
            .max_rect(left_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            // 親Uiの現在レイアウトや↕ボタンのallocate順に依存せず、
            // 左パネルそのものの絶対矩形を基準に配置する。
            let panel_rect = left_rect;
            let timeleap_observing = state.timeleap_waiting_for_blink_observation();
            let timeline_unique = !calculating
                && (matches!(
                    state.session_phase(),
                    SessionPhase::TimelineReady | SessionPhase::Timer | SessionPhase::Correction
                ) || (timeleap_observing && state.results.len() == 1));
            let mode_label = match (
                state.timer_kind(),
                state.timeleap_enabled && !state.timeleap_timeline_adopted(),
            ) {
                (FieldTimerKind::NextBlink, _) => texts.next_blink_timer(),
                (FieldTimerKind::Target, true) => texts.ui_label("Frame1まで"),
                (FieldTimerKind::Target, false) => texts.target_timer(),
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
            let correction_label = correction_auxiliary_timer_label(state, texts);
            let label_text = if correction_label.is_none()
                && timeline_unique
                && state.target_research_pending
            {
                texts.target_calculating().to_owned()
            } else if correction_label.is_none() && !timeline_unique {
                "—".to_owned()
            } else if correction_label.is_none() && state.timeleap_waiting_for_blink_observation() {
                texts.next_blink_seconds(0.0)
            } else if let Some(correction_label) = correction_label {
                correction_label.to_owned()
            } else if state.timeleap_timeline_adopted() {
                match post_timeleap_auxiliary_timer_kind(state.timer_kind()) {
                    FieldTimerKind::Target => {
                        if state.target_is_passed(fps) {
                            texts.target_passed().to_owned()
                        } else {
                            state
                                .preview_remaining(FieldTimerKind::Target, fps)
                                .map(|seconds| texts.target_seconds(seconds.max(0.0)))
                                .unwrap_or_else(|| "—".into())
                        }
                    }
                    FieldTimerKind::NextBlink => state
                        .preview_next_blink_remaining(fps)
                        .map(|seconds| texts.next_blink_seconds(seconds.max(0.0)))
                        .unwrap_or_else(|| "—".into()),
                }
            } else {
                match (state.timer_kind(), state.timeleap_enabled) {
                    (FieldTimerKind::NextBlink, true) => {
                        if state.timeleap_frame1_preview_is_pending() {
                            texts.ui_label("計算中…").into()
                        } else {
                            state
                                .timeleap_frame1_remaining(fps)
                                .map(|seconds| texts.frame1_seconds(seconds.max(0.0)))
                                .unwrap_or_else(|| "—".into())
                        }
                    }
                    (FieldTimerKind::NextBlink, false) => {
                        if state.target_is_passed(fps) {
                            texts.target_passed().to_owned()
                        } else {
                            state
                                .preview_remaining(FieldTimerKind::Target, fps)
                                .map(|seconds| texts.target_seconds(seconds.max(0.0)))
                                .unwrap_or_default()
                        }
                    }
                    (FieldTimerKind::Target, true) => state
                        .preview_next_blink_remaining(fps)
                        .map(|seconds| texts.next_blink_seconds(seconds.max(0.0)))
                        .unwrap_or_else(|| "—".into()),
                    (FieldTimerKind::Target, false) => {
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
                let seconds = if state.session_phase() == SessionPhase::Correction {
                    state.timer_seconds_value()
                } else if timeline_unique {
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
                            egui::RichText::new(if timeleap_observing {
                                texts.seconds(0.0)
                            } else if state.session_phase() == SessionPhase::Correction {
                                texts.seconds(state.timer_seconds_value())
                            } else if state.timeleap_enabled
                                && state.timer_kind() == FieldTimerKind::Target
                                && state.timeleap_frame1_preview_is_pending()
                            {
                                texts.ui_label("計算中…").into()
                            } else if state.next_blink_out_of_range
                                && state.timer_kind() == FieldTimerKind::NextBlink
                            {
                                "—".into()
                            } else {
                                texts.seconds(seconds)
                            })
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
                && !state.timeleap_waiting_for_blink_observation()
                && (state.timer_kind() == FieldTimerKind::Target
                    || state.target_timer_available(fps));
            let mut switch_clicked = false;
            ui.add_enabled_ui(!calculating && can_toggle, |ui| {
                switch_clicked = ui.put(switch_rect, egui::Button::new("↕")).clicked();
            });
            if switch_clicked {
                if let Err(message) = state.toggle_timer_kind(fps) {
                    if state.next_blink_out_of_range {
                        state.set_search_range_notice(texts.app_error(&message));
                    }
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
            ui.put(
                correction_label_rect,
                egui::Label::new(texts.blink_correction()),
            );
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
    let mut actions = FieldUiActions {
        adjust_changed: timer_adjust_changed,
        ..FieldUiActions::default()
    };
    // 孵化タブと同じく、下段の枠をウィンドウ下端まで伸ばす。
    let bottom_height = ui.available_height().max(0.0);
    let (_, bottom_rect) = ui.allocate_space(egui::vec2(ui.available_width(), bottom_height));
    let (bottom_left_rect, bottom_right_rect) =
        split_panel_rects(bottom_rect, LEFT_PANEL_WIDTH, PANEL_GAP);
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(bottom_left_rect), |ui| {
        fixed_group_panel(ui, |ui| {
            #[cfg(test)]
            {
                actions.observation_panel = ui.max_rect();
            }
            let (
                search_requested,
                cancelled,
                observe_requested,
                correction_requested,
                timeleap_ready_cancel_requested,
            ) = field_observation_ui(ui, state, calculating, fps, color_scheme, texts);
            actions.search_requested = search_requested;
            actions.cancelled = cancelled;
            actions.observe_requested = observe_requested;
            actions.correction_requested = correction_requested;
            actions.timeleap_ready_cancel_requested = timeleap_ready_cancel_requested;
        });
    });
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(bottom_right_rect), |ui| {
        fixed_group_panel(ui, |ui| {
            #[cfg(test)]
            {
                actions.timeleap_controls_panel = ui.max_rect();
            }
            let settings_actions = field_settings_ui(
                ui,
                state,
                calculating,
                workers.timeleap_search_running,
                workers.confirmed_target_search_running,
                workers.idx_inference_running,
                fps,
                color_scheme,
                wheel_offset_enabled,
                texts,
                fastest_close,
                false,
            );
            actions.target_committed = settings_actions.target_committed;
            actions.idx_inference_requested = settings_actions.idx_inference_requested;
            actions.idx_inference_cancel_requested =
                settings_actions.idx_inference_cancel_requested;
            actions.target_offset_changed = settings_actions.target_offset_changed;
            actions.open_offset_manager = settings_actions.open_offset_manager;
            actions.timeleap_frame1_preview_requested =
                settings_actions.timeleap_frame1_preview_requested;
            actions.timeleap_enabled_changed = settings_actions.timeleap_enabled_changed;
            actions.timeleap_search_requested |=
                settings_actions.timeleap_candidate_search_requested;
            actions.timeleap_search_cancel_requested =
                settings_actions.timeleap_session_cancel_requested;
            actions.timeleap_candidate_search_cancel_requested =
                settings_actions.timeleap_candidate_search_cancel_requested;
            actions.timeleap_reachability_search_requested =
                settings_actions.timeleap_reachability_search_requested;
            actions.confirmed_target_search_requested =
                settings_actions.confirmed_target_search_requested;
            actions.confirmed_target_search_cancel_requested =
                settings_actions.confirmed_target_search_cancel_requested;
            actions.confirmed_target_search_row = settings_actions.confirmed_target_search_row;
            actions.confirmed_target_offset_drift_frames =
                settings_actions.confirmed_target_offset_drift_frames;
            actions.timeleap_frame1_offset_frame = settings_actions.frame1_offset_to_set;
            #[cfg(test)]
            {
                actions.timeleap_controls_layout = settings_actions.timeleap_controls_layout;
            }
        });
    });
    actions
}

const TIMELEAP_FRAME1_CANDIDATE_COLUMN_WIDTHS: [f32; 6] = [90.0, 80.0, 115.0, 80.0, 80.0, 80.0];
const TIMELEAP_FRAME1_CANDIDATE_COLUMN_GAP: f32 = 8.0;
const TIMELEAP_SEARCH_WINDOW_RESET_BUTTON_WIDTH_REDUCTION: f32 = 36.0;
const TIMELEAP_SEARCH_WINDOW_WIDTH: f32 =
    920.0 - TIMELEAP_SEARCH_WINDOW_RESET_BUTTON_WIDTH_REDUCTION;
const TIMELEAP_SEARCH_WINDOW_HEIGHT: f32 = 610.0;
const TIMELEAP_MAX_DISPLAY_DURATION_SECONDS: i64 = 23 * 60 * 60 + 59 * 60 + 59;
const TIMELEAP_MAIN_FRAME1_GRACE_HELP: &str = "同じ生成開始Frameにたどり着けるTimelineへ移動できるFrame1が、前後にどれくらい連続しているかを2F単位で表示します。";
const TIMELEAP_MAIN_TARGET_GRACE_HELP: &str = "同じ生成開始Frameにたどり着けるTargetFrameが、前後にどれくらい連続しているかを2F単位で表示します。";
const TIMELEAP_MAIN_FRAME1_GRACE_FILTER_HELP: &str = "同じ生成開始Frameにたどり着けるタイムラインへ移動できるFrame1が、どれくらい連続しているか、その下限を設定します。";
const TIMELEAP_MAIN_TARGET_GRACE_FILTER_HELP: &str = "同じ生成開始Frameへ到達するA入力TargetFrameが、どれくらい連続しているか、その下限を設定します。";
const TIMELEAP_MAIN_SECONDS_ONE_HELP: &str = "現在時刻からFrame1までの残り秒数です。内部で使われているタイムラインと、入力されている情報が違う場合、StartingFrameからFrame1までの秒数になります。";
const TIMELEAP_MAIN_SECONDS_TWO_HELP: &str = "Frame1からTargetFrameまでの時間です。";
const TIMELEAP_TARGET_DRIFT_HELP: &str =
    "生成開始Frameからどれくらいフレームずれしているかを2F単位で表示します。";
const TIMELEAP_TARGET_GRACE_HELP: &str = "同じ生成開始Frameにたどり着けるTargetFrameが、前後にどれくらい連続しているかを2F単位で表示します。";

fn post_timeleap_auxiliary_timer_kind(primary: FieldTimerKind) -> FieldTimerKind {
    match primary {
        FieldTimerKind::NextBlink => FieldTimerKind::Target,
        FieldTimerKind::Target => FieldTimerKind::NextBlink,
    }
}

fn correction_auxiliary_timer_label(state: &FieldState, texts: &Texts) -> Option<&'static str> {
    (state.session_phase() == SessionPhase::Correction).then(|| texts.target_passed())
}

#[cfg(test)]
mod timer_label_tests {
    use super::*;

    #[test]
    fn post_timeleap_auxiliary_timer_is_opposite_the_primary_timer() {
        assert_eq!(
            post_timeleap_auxiliary_timer_kind(FieldTimerKind::Target),
            FieldTimerKind::NextBlink
        );
        assert_eq!(
            post_timeleap_auxiliary_timer_kind(FieldTimerKind::NextBlink),
            FieldTimerKind::Target
        );
    }

    #[test]
    fn correction_auxiliary_label_stays_static_instead_of_previewing_time() {
        let mut state = FieldState::default();
        state.enter_correction_mode(crate::domain::timing::DEFAULT_FPS);
        let texts = Texts::new(crate::ui::i18n::Language::Japanese);

        assert_eq!(
            correction_auxiliary_timer_label(&state, &texts),
            Some(texts.target_passed())
        );
        assert_eq!(
            correction_auxiliary_timer_label(&state, &texts),
            Some(texts.target_passed()),
            "the correction label does not depend on the live clock"
        );
    }
}
