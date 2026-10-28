use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) struct TimeleapMainControlsLayout {
    pub(super) row: egui::Rect,
    pub(super) frame1: egui::Rect,
    pub(super) frame1_indicator_row: egui::Rect,
    pub(super) method: egui::Rect,
    pub(super) no_dex: egui::Rect,
    pub(super) direct_encounter: egui::Rect,
    pub(super) search: egui::Rect,
    #[cfg(test)]
    pub(super) timeleap_checkbox: egui::Rect,
    #[cfg(test)]
    pub(super) method_label: egui::Rect,
    #[cfg(test)]
    pub(super) npc_row: egui::Rect,
    #[cfg(test)]
    pub(super) separator_before_timeleap: egui::Rect,
    #[cfg(test)]
    pub(super) separator_before_correction: egui::Rect,
    #[cfg(test)]
    pub(super) correction_row: egui::Rect,
}

impl Default for TimeleapMainControlsLayout {
    fn default() -> Self {
        Self {
            row: egui::Rect::NOTHING,
            frame1: egui::Rect::NOTHING,
            frame1_indicator_row: egui::Rect::NOTHING,
            method: egui::Rect::NOTHING,
            no_dex: egui::Rect::NOTHING,
            direct_encounter: egui::Rect::NOTHING,
            search: egui::Rect::NOTHING,
            #[cfg(test)]
            timeleap_checkbox: egui::Rect::NOTHING,
            #[cfg(test)]
            method_label: egui::Rect::NOTHING,
            #[cfg(test)]
            npc_row: egui::Rect::NOTHING,
            #[cfg(test)]
            separator_before_timeleap: egui::Rect::NOTHING,
            #[cfg(test)]
            separator_before_correction: egui::Rect::NOTHING,
            #[cfg(test)]
            correction_row: egui::Rect::NOTHING,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct TimeleapMainControlsActions {
    pub(super) enabled_changed: bool,
    pub(super) frame1_preview_requested: bool,
    pub(super) search_cancel_requested: bool,
    pub(super) layout: TimeleapMainControlsLayout,
}

#[cfg(test)]
pub(super) fn timeleap_main_controls_ui(
    ui: &mut egui::Ui,
    state: &mut FieldState,
    fps: f64,
    row_width: f32,
    color_scheme: ColorScheme,
) -> TimeleapMainControlsActions {
    timeleap_main_controls_ui_with_texts(
        ui,
        state,
        fps,
        row_width,
        color_scheme,
        &Texts::new(crate::ui::i18n::Language::Japanese),
    )
}

pub(super) fn timeleap_main_controls_ui_with_texts(
    ui: &mut egui::Ui,
    state: &mut FieldState,
    fps: f64,
    row_width: f32,
    color_scheme: ColorScheme,
    texts: &Texts,
) -> TimeleapMainControlsActions {
    let row_height = ui.spacing().interact_size.y + 4.0;
    let mut actions = TimeleapMainControlsActions::default();
    ui.allocate_ui_with_layout(
        egui::vec2(row_width, row_height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let row_rect = ui.max_rect();
            let mut layout = TimeleapMainControlsLayout {
                row: row_rect,
                ..TimeleapMainControlsLayout::default()
            };
            let mut enabled_changed = false;

            // Keep the checkbox's stroke/text inset from the panel edge like the search button.
            ui.add_space(4.0);
            ui.add_enabled_ui(
                !state.timeleap_lifecycle_policy().configuration_locked
                    && !state.timeleap_frame1_preparing,
                |ui| {
                    let checkbox = ui.checkbox(&mut state.timeleap_enabled, "Timeleap");
                    #[cfg(test)]
                    {
                        layout.timeleap_checkbox = checkbox.rect;
                    }
                    enabled_changed = checkbox.changed();
                },
            );
            if enabled_changed {
                actions.enabled_changed = true;
                actions.frame1_preview_requested = true;
                if state.timeleap_enabled {
                    state.select_timeleap_frame1_timer();
                    state.invalidate_timeleap_frame1_preview();
                } else {
                    state.clear_timeleap_candidates();
                    actions.search_cancel_requested = true;
                    if state.timer_kind() == FieldTimerKind::Target
                        && state.start_target_timer(fps).is_err()
                    {
                        state.normalize_timer_kind(fps);
                    }
                }
            }
            let response = ui.add_enabled_ui(state.can_edit_timeleap_frame1_input(), |ui| {
                numeric_text_edit(ui, &mut state.timeleap_frame1_input, 78.0, false, false)
            });
            layout.frame1 = response.inner.rect;
            if response.inner.changed() && state.timeleap_enabled {
                state.sync_timeleap_source_frame1_from_main();
                state.invalidate_timeleap_frame1_preview();
                actions.frame1_preview_requested = true;
            }
            let frame1_status = if state.timeleap_enabled {
                state
                    .timeleap_frame1_indicator()
                    .map(|indicator| match indicator {
                        crate::application::field::TimeleapFrame1Indicator::Reachable => {
                            ("○", color_scheme.colors_for_ui(ui).target_yes)
                        }
                        crate::application::field::TimeleapFrame1Indicator::Unreachable => {
                            ("×", color_scheme.colors_for_ui(ui).target_no)
                        }
                    })
            } else {
                None
            };
            if let Some((symbol, color)) = frame1_status {
                ui.colored_label(color, "Frame1");
                layout.frame1_indicator_row = ui.colored_label(color, symbol).rect;
            }
            if !layout.frame1_indicator_row.is_positive() {
                layout.frame1_indicator_row = layout.frame1;
            }

            let mut search_rect = egui::Rect::NOTHING;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(4.0);
                let search = centered_search_button_with_texts(
                    ui,
                    main_timeleap_search_enabled(state),
                    52.0,
                    texts,
                );
                search_rect = search.rect;
                if search.clicked() {
                    state.timeleap_search_window_open = true;
                }
            });
            layout.search = search_rect;
            debug_assert!((layout.row.width() - row_width).abs() < 0.5);
            actions.layout = layout;
        },
    );

    let mut method_rect = egui::Rect::NOTHING;
    let mut no_dex_rect = egui::Rect::NOTHING;
    let mut direct_encounter_rect = egui::Rect::NOTHING;
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        let _method_label = ui.label(texts.ui_label("方式"));
        #[cfg(test)]
        {
            actions.layout.method_label = _method_label.rect;
        }
        ui.add_enabled_ui(
            state.timeleap_enabled
                && !state.timeleap_lifecycle_policy().configuration_locked
                && !state.timeleap_frame1_preparing,
            |ui| {
                let mut method = crate::application::field::TimeleapMethodSelection::from_state(
                    state.timeleap_mode,
                    state.timeleap_is_boy,
                );
                let combo = egui::ComboBox::from_id_salt("main-timeleap-method")
                    .selected_text(texts.timeleap_method(method))
                    .width((row_width * 0.5).max(170.0))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut method,
                            crate::application::field::TimeleapMethodSelection::Dialogue,
                            texts.ui_label("会話送り"),
                        );
                        ui.selectable_value(
                            &mut method,
                            crate::application::field::TimeleapMethodSelection::MenuBoy,
                            texts.ui_label("メニュー閉じ（男の子）"),
                        );
                        ui.selectable_value(
                            &mut method,
                            crate::application::field::TimeleapMethodSelection::MenuGirl,
                            texts.ui_label("メニュー閉じ（女の子）"),
                        );
                        ui.selectable_value(
                            &mut method,
                            crate::application::field::TimeleapMethodSelection::Courier,
                            texts.ui_label("配達員"),
                        );
                    });
                method_rect = combo.response.rect;
                let (mode, is_boy) = method.state_values();
                if mode != state.timeleap_mode || is_boy != state.timeleap_is_boy {
                    actions.frame1_preview_requested |=
                        apply_timeleap_method_selection(state, method);
                }
            },
        );
        let no_dex = ui.add_enabled_ui(main_timeleap_no_dex_enabled(state), |ui| {
            ui.checkbox(&mut state.timeleap_no_dex, "No Dex")
        });
        no_dex_rect = no_dex.inner.rect;
        if no_dex.inner.changed() {
            actions.frame1_preview_requested |= state.courier_condition_changed();
        }
        let direct_encounter =
            ui.add_enabled_ui(main_timeleap_direct_encounter_enabled(state), |ui| {
                ui.checkbox(
                    &mut state.timeleap_direct_encounter,
                    texts.ui_label("そのままエンカウント"),
                )
            });
        direct_encounter_rect = direct_encounter.inner.rect;
    });
    actions.layout.method = method_rect;
    actions.layout.no_dex = no_dex_rect;
    actions.layout.direct_encounter = direct_encounter_rect;

    actions
}

pub(super) fn main_timeleap_search_enabled(state: &FieldState) -> bool {
    state.timeleap_lifecycle_policy().candidate_search_enabled
}

pub(super) fn main_timeleap_no_dex_enabled(state: &FieldState) -> bool {
    let lifecycle = state.timeleap_lifecycle_policy();
    state.timeleap_enabled
        && state.timeleap_uses_courier_method()
        && !lifecycle.configuration_locked
        && !state.timeleap_frame1_preparing
        && !state.timeleap_frame1_has_passed()
}

pub(super) fn main_timeleap_direct_encounter_enabled(state: &FieldState) -> bool {
    let lifecycle = state.timeleap_lifecycle_policy();
    state.timeleap_enabled
        && !lifecycle.configuration_locked
        && !state.timeleap_frame1_preparing
        && !state.timeleap_frame1_has_passed()
}

/// Keep the independent Timeleap viewport alive without laying out the main Field
/// timer, observation table, or observation controls on another selected tab.
#[allow(clippy::too_many_arguments)]
pub(crate) fn timeleap_window_keepalive_ui(
    ui: &mut egui::Ui,
    state: &mut FieldState,
    workers: FieldUiWorkerState,
    ctx: &egui::Context,
    fps: f64,
    wheel_offset_enabled: bool,
    color_scheme: ColorScheme,
    texts: &Texts,
    fastest_close: bool,
) -> FieldUiActions {
    let calculating = field_search_is_calculating(workers, state.target_research_pending);
    if state.timeleap_search_window_open {
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
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
        true,
    );
    FieldUiActions {
        target_committed: settings_actions.target_committed,
        idx_inference_requested: settings_actions.idx_inference_requested,
        idx_inference_cancel_requested: settings_actions.idx_inference_cancel_requested,
        target_offset_changed: settings_actions.target_offset_changed,
        open_offset_manager: settings_actions.open_offset_manager,
        timeleap_frame1_preview_requested: settings_actions.timeleap_frame1_preview_requested,
        timeleap_enabled_changed: settings_actions.timeleap_enabled_changed,
        timeleap_search_requested: settings_actions.timeleap_candidate_search_requested,
        timeleap_search_cancel_requested: settings_actions.timeleap_session_cancel_requested,
        timeleap_candidate_search_cancel_requested: settings_actions
            .timeleap_candidate_search_cancel_requested,
        timeleap_reachability_search_requested: settings_actions
            .timeleap_reachability_search_requested,
        confirmed_target_search_requested: settings_actions.confirmed_target_search_requested,
        confirmed_target_search_cancel_requested: settings_actions
            .confirmed_target_search_cancel_requested,
        confirmed_target_search_row: settings_actions.confirmed_target_search_row,
        confirmed_target_offset_drift_frames: settings_actions.confirmed_target_offset_drift_frames,
        timeleap_frame1_offset_frame: settings_actions.frame1_offset_to_set,
        ..Default::default()
    }
}
