use super::*;

#[derive(Default)]
pub(super) struct TimeleapCategoryControlsActions {
    pub(super) preview_requested: bool,
    #[cfg(test)]
    combo_rects: Vec<egui::Rect>,
}

#[derive(Default)]
pub(super) struct TimeleapSearchOptionsActions {
    pub(super) preview_requested: bool,
    #[cfg(test)]
    consider_delay_rect: Option<egui::Rect>,
    #[cfg(test)]
    no_dex_rect: Option<egui::Rect>,
    #[cfg(test)]
    direct_encounter_rect: Option<egui::Rect>,
}

pub(super) fn timeleap_category_pokemon_method_controls(
    ui: &mut egui::Ui,
    state: &mut FieldState,
    texts: &Texts,
    id_scope: &'static str,
    form_column_width: f32,
    menu_heights: Option<(f32, f32)>,
) -> TimeleapCategoryControlsActions {
    let catalog = crate::domain::fixed_rng_catalog::categories(state.timeleap_game_edition);
    let courier_category = catalog.len();
    let category_label = catalog
        .get(state.timeleap_search_category)
        .map(|category| texts.fixed_rng_category(category.name))
        .unwrap_or(texts.ui_label("配達員"))
        .to_owned();
    let mut actions = TimeleapCategoryControlsActions::default();
    // ComboBox::width is a minimum content width. egui subtracts button padding
    // internally, so compensate for the app's wider-than-default horizontal padding
    // to keep the painted outer frame aligned with fixed-width buttons.
    let combo_width = form_column_width
        + 2.0 * (ui.spacing().button_padding.x - egui::style::Spacing::default().button_padding.x);
    let mut category_changed = false;
    ui.label(texts.ui_label("カテゴリー"));
    let category_combo = egui::ComboBox::from_id_salt(("confirmed-target-category", id_scope))
        .selected_text(category_label)
        .width(combo_width)
        .truncate();
    let category_combo = if let Some((category_height, _)) = menu_heights {
        category_combo.height(category_height)
    } else {
        category_combo
    };
    let _category_response = category_combo.show_ui(ui, |ui| {
        for (index, category) in catalog.iter().enumerate() {
            category_changed |= ui
                .selectable_value(
                    &mut state.timeleap_search_category,
                    index,
                    texts.fixed_rng_category(category.name),
                )
                .changed();
        }
        category_changed |= ui
            .selectable_value(
                &mut state.timeleap_search_category,
                courier_category,
                texts.ui_label("配達員"),
            )
            .changed();
    });
    #[cfg(test)]
    actions.combo_rects.push(_category_response.response.rect);
    if category_changed {
        actions.preview_requested |= apply_timeleap_category_selection(state, courier_category);
    }

    ui.label(texts.ui_label("ポケモン"));
    let selected_preset = (state.timeleap_search_category != courier_category)
        .then(|| {
            crate::domain::fixed_rng_catalog::selected_preset_for(
                state.timeleap_game_edition,
                state.timeleap_search_category,
                state.timeleap_search_preset,
            )
        })
        .flatten();
    let selected_name = selected_preset
        .map(|preset| texts.fixed_rng_preset_name(preset))
        .unwrap_or_else(|| texts.ui_label("配達員設定").to_owned());
    let mut preset_changed = false;
    let preset_combo = egui::ComboBox::from_id_salt(("confirmed-target-pokemon", id_scope))
        .selected_text(selected_name)
        .width(combo_width)
        .truncate();
    let preset_combo = if let Some((_, preset_height)) = menu_heights {
        preset_combo.height(preset_height)
    } else {
        preset_combo
    };
    let _preset_response = preset_combo.show_ui(ui, |ui| {
        if state.timeleap_search_category == courier_category {
            ui.label(texts.ui_label("配達員個体は配達員設定タブで指定"));
        } else if let Some(category) = catalog.get(state.timeleap_search_category) {
            for (index, preset) in category.presets.iter().enumerate() {
                preset_changed |= ui
                    .selectable_value(
                        &mut state.timeleap_search_preset,
                        index,
                        texts.fixed_rng_preset_name(preset),
                    )
                    .changed();
            }
        }
    });
    #[cfg(test)]
    actions.combo_rects.push(_preset_response.response.rect);
    if preset_changed {
        actions.preview_requested |= apply_timeleap_preset_selection(state);
    }

    let mut method = crate::application::field::TimeleapMethodSelection::from_state(
        state.timeleap_mode,
        state.timeleap_is_boy,
    );
    ui.label(texts.ui_label("Timeleap方式"));
    let _method_response = egui::ComboBox::from_id_salt(("confirmed-target-method", id_scope))
        .selected_text(texts.timeleap_method(method))
        .width(combo_width)
        .truncate()
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
    #[cfg(test)]
    actions.combo_rects.push(_method_response.response.rect);
    let (mode, is_boy) = method.state_values();
    if mode != state.timeleap_mode || is_boy != state.timeleap_is_boy {
        actions.preview_requested |= apply_timeleap_method_selection(state, method);
    }
    actions
}

fn apply_timeleap_category_selection(state: &mut FieldState, courier_category: usize) -> bool {
    state.timeleap_search_preset = 0;
    if state.timeleap_search_category == courier_category {
        state.timeleap_mode = TimeleapMode::Courier;
        state.refresh_timeleap_courier_delay();
    } else if let Some(preset) = crate::domain::fixed_rng_catalog::selected_preset_for(
        state.timeleap_game_edition,
        state.timeleap_search_category,
        0,
    ) {
        state.apply_timeleap_preset(preset);
    }
    state.invalidate_timeleap_frame1_preview_after_input_change()
}

fn apply_timeleap_preset_selection(state: &mut FieldState) -> bool {
    if let Some(preset) = crate::domain::fixed_rng_catalog::selected_preset_for(
        state.timeleap_game_edition,
        state.timeleap_search_category,
        state.timeleap_search_preset,
    ) {
        state.apply_timeleap_preset(preset);
    }
    state.invalidate_timeleap_frame1_preview_after_input_change()
}

pub(super) fn apply_timeleap_method_selection(
    state: &mut FieldState,
    method: crate::application::field::TimeleapMethodSelection,
) -> bool {
    (state.timeleap_mode, state.timeleap_is_boy) = method.state_values();
    if state.timeleap_is_courier_category() {
        state.refresh_timeleap_courier_delay();
    }
    state.invalidate_timeleap_frame1_preview_after_input_change()
}

pub(super) fn timeleap_search_options_ui(
    ui: &mut egui::Ui,
    state: &mut FieldState,
    texts: &Texts,
) -> TimeleapSearchOptionsActions {
    let mut actions = TimeleapSearchOptionsActions::default();
    ui.horizontal(|ui| {
        #[cfg(test)]
        let delay_response =
            ui.checkbox(&mut state.timeleap_consider_delay_enabled, "Consider Delay");
        #[cfg(not(test))]
        ui.checkbox(&mut state.timeleap_consider_delay_enabled, "Consider Delay");
        #[cfg(test)]
        {
            actions.consider_delay_rect = Some(delay_response.rect);
        }
        ui.add_enabled_ui(state.timeleap_consider_delay_enabled, |ui| {
            bounded_integer_text_edit(ui, &mut state.timeleap_consider_delay_frames, 46.0, 0, 9998);
            ui.label("F +4");
        });
    });
    ui.horizontal(|ui| {
        let no_dex_response = ui.add_enabled_ui(state.timeleap_uses_courier_method(), |ui| {
            ui.checkbox(&mut state.timeleap_no_dex, "No Dex")
        });
        #[cfg(test)]
        {
            actions.no_dex_rect = Some(no_dex_response.inner.rect);
        }
        actions.preview_requested = no_dex_response.inner.changed()
            && state.timeleap_uses_courier_method()
            && state.courier_condition_changed();

        let _direct_encounter = ui.add_enabled_ui(
            super::timeleap_controls::main_timeleap_direct_encounter_enabled(state),
            |ui| {
                ui.checkbox(
                    &mut state.timeleap_direct_encounter,
                    texts.ui_label("そのままエンカウント"),
                )
            },
        );
        #[cfg(test)]
        {
            actions.direct_encounter_rect = Some(_direct_encounter.inner.rect);
        }
    });
    actions
}

pub(super) fn help_button(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash,
    description: &str,
) -> egui::Response {
    let response = ui.small_button("?").on_hover_text(description);
    show_help_popup(ui, response, id_source, description)
}

fn help_button_at_rect(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash,
    description: &str,
    rect: egui::Rect,
) -> egui::Response {
    let response = ui
        .put(rect, egui::Button::new("?").small())
        .on_hover_text(description);
    show_help_popup(ui, response, id_source, description)
}

fn show_help_popup(
    ui: &mut egui::Ui,
    response: egui::Response,
    id_source: impl std::hash::Hash,
    description: &str,
) -> egui::Response {
    let popup_id = ui.make_persistent_id(id_source);
    if response.clicked() {
        ui.memory_mut(|memory| memory.toggle_popup(popup_id));
    }
    egui::popup::popup_below_widget(
        ui,
        popup_id,
        &response,
        egui::popup::PopupCloseBehavior::CloseOnClickOutside,
        |ui| {
            ui.set_max_width(320.0);
            ui.label(description);
        },
    );
    response
}

#[cfg(test)]
pub(super) fn centered_search_button(
    ui: &mut egui::Ui,
    enabled: bool,
    width: f32,
) -> egui::Response {
    centered_button(ui, enabled, width, "検索")
}

pub(super) fn centered_search_button_with_texts(
    ui: &mut egui::Ui,
    enabled: bool,
    width: f32,
    texts: &Texts,
) -> egui::Response {
    centered_button(ui, enabled, width, texts.ui_label("検索"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SearchButtonAction {
    Search,
    Cancel,
}

pub(super) fn search_button_action(running: bool) -> SearchButtonAction {
    if running {
        SearchButtonAction::Cancel
    } else {
        SearchButtonAction::Search
    }
}

fn search_button_label(running: bool) -> &'static str {
    match search_button_action(running) {
        SearchButtonAction::Search => "検索",
        SearchButtonAction::Cancel => "キャンセル",
    }
}

pub(super) fn centered_search_or_cancel_button_with_texts(
    ui: &mut egui::Ui,
    enabled: bool,
    width: f32,
    texts: &Texts,
    running: bool,
) -> egui::Response {
    centered_button(
        ui,
        enabled,
        width,
        texts.ui_label(search_button_label(running)),
    )
}

fn centered_button(ui: &mut egui::Ui, enabled: bool, width: f32, label: &str) -> egui::Response {
    let size = egui::vec2(width, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(
        size,
        egui::Layout::centered_and_justified(egui::Direction::TopDown),
        |ui| ui.add_enabled_ui(enabled, |ui| ui.add_sized(size, egui::Button::new(label))),
    )
    .inner
    .inner
}

#[cfg(test)]
pub(super) fn timeleap_candidate_search_button(
    ui: &mut egui::Ui,
    enabled: bool,
    width: f32,
) -> egui::Response {
    centered_button(ui, enabled, width, "検索")
}

pub(super) fn timeleap_candidate_search_button_with_texts(
    ui: &mut egui::Ui,
    enabled: bool,
    width: f32,
    texts: &Texts,
    running: bool,
) -> egui::Response {
    centered_button(
        ui,
        enabled,
        width,
        texts.ui_label(search_button_label(running)),
    )
}

#[cfg(test)]
pub(super) fn timeleap_generation_start_frame_row(
    ui: &mut egui::Ui,
    width: f32,
    help_id: &str,
    add_input: impl FnOnce(&mut egui::Ui) -> egui::Response,
) -> [egui::Rect; 4] {
    timeleap_generation_start_frame_row_with_texts(
        ui,
        width,
        help_id,
        &Texts::new(crate::ui::i18n::Language::Japanese),
        add_input,
    )
}

pub(super) fn timeleap_generation_start_frame_row_with_texts(
    ui: &mut egui::Ui,
    width: f32,
    help_id: &str,
    texts: &Texts,
    add_input: impl FnOnce(&mut egui::Ui) -> egui::Response,
) -> [egui::Rect; 4] {
    ui.allocate_ui_with_layout(
        egui::vec2(width, ui.spacing().interact_size.y),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(width);
            ui.set_max_width(width);
            let label = ui.label(texts.ui_label("生成開始Frame"));
            let input = add_input(ui);
            let unit = ui.label("F");
            let help = help_button(
                ui,
                help_id,
                texts.ui_label("実際に欲しい個体が生成されるFrameを指定します。Consider Delayを使う場合は、A入力Frameとは異なります。"),
            );
            [label.rect, input.rect, unit.rect, help.rect]
        },
    )
    .inner
}

pub(super) struct ConfirmedTargetSearchTabActions {
    pub(super) search_requested: bool,
    pub(super) cancel_requested: bool,
    pub(super) preview_requested: bool,
    pub(super) selected_row: Option<crate::domain::timeleap::ConfirmedTargetSearchRow>,
    pub(super) offset_drift_frames: Option<i64>,
    #[cfg(test)]
    search_button_rect: egui::Rect,
    #[cfg(test)]
    selection_combo_rects: Vec<egui::Rect>,
    #[cfg(test)]
    selection_separator_rect: egui::Rect,
    #[cfg(test)]
    generation_start_row_rects: [egui::Rect; 4],
    #[cfg(test)]
    result_header_cells: [egui::Rect; 5],
    #[cfg(test)]
    result_body_rect: egui::Rect,
    #[cfg(test)]
    result_status_rect: egui::Rect,
}

#[cfg(test)]
pub(super) fn confirmed_target_search_tab_ui(
    ui: &mut egui::Ui,
    state: &mut FieldState,
    texts: &Texts,
    _calculating: bool,
    main_seed_editable: bool,
    fastest_close: bool,
) -> ConfirmedTargetSearchTabActions {
    confirmed_target_search_tab_ui_with_worker_running(
        ui,
        state,
        texts,
        _calculating,
        main_seed_editable,
        fastest_close,
        false,
    )
}

pub(super) fn confirmed_target_search_tab_ui_with_worker_running(
    ui: &mut egui::Ui,
    state: &mut FieldState,
    texts: &Texts,
    _calculating: bool,
    main_seed_editable: bool,
    fastest_close: bool,
    worker_running: bool,
) -> ConfirmedTargetSearchTabActions {
    let mut search_requested = false;
    let mut cancel_requested = false;
    let mut preview_requested = false;
    let mut selected_row = None;
    let mut offset_drift_frames = None;
    #[cfg(test)]
    let mut search_button_rect = egui::Rect::NOTHING;
    #[cfg(test)]
    let mut selection_combo_rects = Vec::new();
    #[cfg(test)]
    let mut selection_separator_rect = egui::Rect::NOTHING;
    #[cfg(test)]
    let mut generation_start_row_rects = [egui::Rect::NOTHING; 4];
    #[cfg(test)]
    let mut result_header_cells = [egui::Rect::NOTHING; 5];
    #[cfg(test)]
    let mut result_body_rect = egui::Rect::NOTHING;
    #[cfg(test)]
    let mut result_status_rect = egui::Rect::NOTHING;
    let main_frame1_editable = state.can_edit_timeleap_frame1_input();
    let metadata = timeleap_search_metadata_ui_with_texts(
        ui,
        state,
        true,
        main_seed_editable,
        main_frame1_editable,
        texts,
    );
    preview_requested |= metadata.frame1_preview_requested;
    ui.separator();
    state.initialize_confirmed_target_search_form();

    let content_width = ui.available_width();
    let content_height = ui.available_height();
    let column_gap = ui.spacing().item_spacing.x;
    let (left_width, right_width) = timeleap_search_column_widths(content_width, column_gap);
    let running = worker_running || state.confirmed_target_search_is_running();
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(left_width, content_height),
            egui::Layout::top_down_justified(egui::Align::Min),
            |ui| {
                ui.set_min_width(left_width);
                ui.set_max_width(left_width);
                let selection_controls = timeleap_category_pokemon_method_controls(
                    ui,
                    state,
                    texts,
                    "confirmed-target",
                    left_width,
                    None,
                );
                preview_requested |= selection_controls.preview_requested;
                #[cfg(test)]
                selection_combo_rects.extend(selection_controls.combo_rects);
                preview_requested |=
                    timeleap_search_options_ui(ui, state, texts).preview_requested;
                let _separator_response = ui.separator();
                #[cfg(test)]
                {
                    selection_separator_rect = _separator_response.rect;
                }
                let _generation_start_row = timeleap_generation_start_frame_row_with_texts(
                    ui,
                    left_width,
                    "confirmed-target-generation-start",
                    texts,
                    |ui| {
                        ui.add(
                            egui::TextEdit::singleline(
                                &mut state.timeleap_generation_start_frame,
                            )
                            .desired_width(103.0)
                            .horizontal_align(egui::Align::Max),
                        )
                    },
                );
                #[cfg(test)]
                {
                    generation_start_row_rects = _generation_start_row;
                }
                ui.horizontal(|ui| {
                    let form = &mut state.confirmed_target_search_form;
                    ui.checkbox(
                        &mut form.drift_search_enabled,
                        texts.ui_label("ズレ検索"),
                    );
                    ui.add_enabled_ui(
                        form.drift_search_enabled,
                        |ui| {
                            ui.add(
                                egui::TextEdit::singleline(
                                    &mut form.drift_search_generation_start_frame,
                                )
                                .desired_width(70.0),
                            )
                        },
                    );
                    ui.label("F ±");
                    ui.add(
                        egui::TextEdit::singleline(&mut form.drift_range_ticks)
                            .desired_width(55.0),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(texts.ui_label("猶予"));
                    ui.add(
                        egui::TextEdit::singleline(
                            &mut state.confirmed_target_search_form.grace_min_frames,
                        )
                        .desired_width(55.0),
                    );
                    ui.label(texts.ui_label("F以上"));
                });
                let search_width = left_width;
                let source_ready = state.parsed_timeleap_source().is_ok()
                    && state
                        .timeleap_source_frame1
                        .trim()
                        .parse::<i64>()
                        .is_ok_and(|frame| {
                            frame >= 0 && frame <= crate::domain::rng::MAX_SFMT_FRAME
                        })
                    && state.committed_target_frame().is_some();
                let search_response = centered_search_or_cancel_button_with_texts(
                    ui,
                    running || confirmed_target_search_can_search(source_ready, running),
                    search_width,
                    texts,
                    running,
                );
                #[cfg(test)]
                {
                    search_button_rect = search_response.rect;
                }
                if search_response.clicked() {
                    match search_button_action(running) {
                        SearchButtonAction::Search => search_requested = true,
                        SearchButtonAction::Cancel => cancel_requested = true,
                    }
                }
                if !running && !source_ready {
                    ui.weak(texts.ui_label("Initial Seed・StartingFrame・NPC数・Frame1とTargetFrameを入力すると検索できます。"));
                }
            },
        );
        ui.allocate_ui_with_layout(
            egui::vec2(right_width, content_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                let form = &state.confirmed_target_search_form;
                let column_widths = confirmed_target_search_column_widths();
                let worker_running_status =
                    crate::application::field::ConfirmedTargetSearchStatus::Running;
                let status = if running {
                    &worker_running_status
                } else {
                    &form.status
                };
                let actions = confirmed_target_search_table_ui_with_texts(
                    ui,
                    status,
                    form.result_generation_start_frame,
                    form.result_drift_search_generation_start_frame,
                    form.result_drift_marker_boundary,
                    &column_widths,
                    state,
                    fastest_close,
                    texts,
                );
                selected_row = actions.selected_row;
                offset_drift_frames = actions.offset_drift_frames;
                #[cfg(test)]
                {
                    result_header_cells = actions.header_cells;
                    result_body_rect = actions.body_rect;
                    result_status_rect = actions.status_rect;
                }
            },
        );
    });
    ConfirmedTargetSearchTabActions {
        search_requested,
        cancel_requested,
        preview_requested,
        selected_row,
        offset_drift_frames,
        #[cfg(test)]
        search_button_rect,
        #[cfg(test)]
        selection_combo_rects,
        #[cfg(test)]
        selection_separator_rect,
        #[cfg(test)]
        generation_start_row_rects,
        #[cfg(test)]
        result_header_cells,
        #[cfg(test)]
        result_body_rect,
        #[cfg(test)]
        result_status_rect,
    }
}

#[derive(Default)]
pub(super) struct TimeleapSearchMetadataActions {
    edited: bool,
    frame1_preview_requested: bool,
    #[cfg(test)]
    reset_button_rect: Option<egui::Rect>,
    #[cfg(test)]
    starting_frame_rect: Option<egui::Rect>,
    #[cfg(test)]
    frame1_rect: Option<egui::Rect>,
}

#[cfg(test)]
pub(super) fn timeleap_search_metadata_ui(
    ui: &mut egui::Ui,
    state: &mut FieldState,
    show_frame1: bool,
    main_seed_editable: bool,
    main_frame1_editable: bool,
) -> TimeleapSearchMetadataActions {
    timeleap_search_metadata_ui_with_texts(
        ui,
        state,
        show_frame1,
        main_seed_editable,
        main_frame1_editable,
        &Texts::new(crate::ui::i18n::Language::Japanese),
    )
}

pub(super) fn timeleap_search_metadata_ui_with_texts(
    ui: &mut egui::Ui,
    state: &mut FieldState,
    show_frame1: bool,
    main_seed_editable: bool,
    main_frame1_editable: bool,
    texts: &Texts,
) -> TimeleapSearchMetadataActions {
    state.initialize_timeleap_source_inputs();
    let mut actions = TimeleapSearchMetadataActions::default();
    ui.horizontal(|ui| {
        ui.label(texts.ui_label("初期SEED:"));
        let seed_changed = ui
            .add(egui::TextEdit::singleline(&mut state.timeleap_source_seed).desired_width(105.0))
            .changed();
        actions.edited |= seed_changed;
        if seed_changed && main_seed_editable {
            state.set_timeleap_source_seed(state.timeleap_source_seed.clone(), true);
        }
        ui.separator();
        ui.label("StartingFrame:");
        let starting_frame = ui.add(
            egui::TextEdit::singleline(&mut state.timeleap_source_starting_frame)
                .desired_width(90.0),
        );
        actions.edited |= starting_frame.changed();
        #[cfg(test)]
        {
            actions.starting_frame_rect = Some(starting_frame.rect);
        }
        ui.separator();
        ui.label(texts.ui_label("NPC数:"));
        actions.edited |= ui
            .add(
                egui::TextEdit::singleline(&mut state.timeleap_source_npc_count)
                    .desired_width(48.0),
            )
            .changed();
        if show_frame1 {
            ui.separator();
            ui.label("Frame1:");
            let previous_main_frame1 = state.timeleap_frame1_input.clone();
            let frame1 = ui.add(
                egui::TextEdit::singleline(&mut state.timeleap_source_frame1).desired_width(72.0),
            );
            let frame1_changed = frame1.changed();
            actions.edited |= frame1_changed;
            #[cfg(test)]
            {
                actions.frame1_rect = Some(frame1.rect);
            }
            if frame1_changed {
                state.set_timeleap_source_frame1(
                    state.timeleap_source_frame1.clone(),
                    main_frame1_editable,
                );
                if main_frame1_editable && previous_main_frame1 != state.timeleap_frame1_input {
                    state.invalidate_timeleap_frame1_preview();
                    actions.frame1_preview_requested = true;
                }
            }
            ui.label("F");
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let response = ui.button("Reset");
            #[cfg(test)]
            {
                actions.reset_button_rect = Some(response.rect);
            }
            if response.clicked() {
                let previous_main_frame1 = state.timeleap_frame1_input.clone();
                state.reset_timeleap_source_inputs();
                actions.edited = true;
                if main_frame1_editable && previous_main_frame1 != state.timeleap_source_frame1 {
                    state.timeleap_frame1_input = state.timeleap_source_frame1.clone();
                    state.invalidate_timeleap_frame1_preview();
                    actions.frame1_preview_requested = true;
                }
            }
        });
    });
    actions
}

pub(super) const TIMELEAP_TAB_FRAME1_SEARCH: u8 = 0;
pub(super) const TIMELEAP_TAB_TARGET_SEARCH: u8 = 1;
pub(super) const TIMELEAP_TAB_COURIER: u8 = 2;
pub(super) const TIMELEAP_TAB_SETTINGS: u8 = 3;

pub(super) struct TimeleapSearchHeaderActions {
    pub(super) preview_requested: bool,
    #[cfg(test)]
    pub(super) tab_rects: [egui::Rect; 4],
    #[cfg(test)]
    pub(super) edition_rect: egui::Rect,
}

impl Default for TimeleapSearchHeaderActions {
    fn default() -> Self {
        Self {
            preview_requested: false,
            #[cfg(test)]
            tab_rects: [egui::Rect::NOTHING; 4],
            #[cfg(test)]
            edition_rect: egui::Rect::NOTHING,
        }
    }
}

pub(super) fn timeleap_search_header_ui(
    ui: &mut egui::Ui,
    state: &mut FieldState,
    texts: &Texts,
) -> TimeleapSearchHeaderActions {
    const TABS: [(u8, &str); 4] = [
        (TIMELEAP_TAB_FRAME1_SEARCH, "Frame1検索"),
        (TIMELEAP_TAB_TARGET_SEARCH, "TargetFrame検索"),
        (TIMELEAP_TAB_COURIER, "配達員"),
        (TIMELEAP_TAB_SETTINGS, "設定"),
    ];
    let mut actions = TimeleapSearchHeaderActions::default();
    state.timeleap_search_tab = state.timeleap_search_tab.min(TIMELEAP_TAB_SETTINGS);
    ui.horizontal(|ui| {
        for (index, (tab, label)) in TABS.into_iter().enumerate() {
            let response =
                ui.selectable_value(&mut state.timeleap_search_tab, tab, texts.ui_label(label));
            #[cfg(test)]
            {
                actions.tab_rects[index] = response.rect;
            }
            #[cfg(not(test))]
            let _ = (index, response);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut selected_edition = state.timeleap_game_edition;
            let response = egui::ComboBox::from_id_salt("timeleap-game-edition")
                .selected_text(selected_edition.label())
                .width(76.0)
                .show_ui(ui, |ui| {
                    for edition in crate::domain::fixed_rng_catalog::GameEdition::ALL {
                        ui.selectable_value(&mut selected_edition, edition, edition.label());
                    }
                });
            #[cfg(test)]
            {
                actions.edition_rect = response.response.rect;
            }
            #[cfg(not(test))]
            let _ = response;
            if selected_edition != state.timeleap_game_edition {
                actions.preview_requested = state.set_timeleap_game_edition(selected_edition);
            }
        });
    });
    actions
}

#[cfg(test)]
pub(super) fn timeleap_search_tabs_ui(ui: &mut egui::Ui, selected_tab: &mut u8) -> [egui::Rect; 4] {
    timeleap_search_tabs_ui_with_texts(
        ui,
        selected_tab,
        &Texts::new(crate::ui::i18n::Language::Japanese),
    )
}

#[cfg(test)]
pub(super) fn timeleap_search_tabs_ui_with_texts(
    ui: &mut egui::Ui,
    selected_tab: &mut u8,
    texts: &Texts,
) -> [egui::Rect; 4] {
    const TABS: [(u8, &str); 4] = [
        (TIMELEAP_TAB_FRAME1_SEARCH, "Frame1検索"),
        (TIMELEAP_TAB_TARGET_SEARCH, "TargetFrame検索"),
        (TIMELEAP_TAB_COURIER, "配達員"),
        (TIMELEAP_TAB_SETTINGS, "設定"),
    ];
    ui.horizontal(|ui| {
        let mut rects = [egui::Rect::NOTHING; 4];
        for (index, (tab, label)) in TABS.into_iter().enumerate() {
            rects[index] = ui
                .selectable_value(selected_tab, tab, texts.ui_label(label))
                .rect;
        }
        rects
    })
    .inner
}

pub(super) fn actual_timeleap_frame1_for_display(state: &FieldState) -> Option<i64> {
    state.timeleap_lifecycle_policy().summary_frame1
}

fn confirmed_target_search_can_search(adopted: bool, running: bool) -> bool {
    adopted && !running
}

pub(super) fn timeleap_search_column_widths(content_width: f32, gap: f32) -> (f32, f32) {
    let available_width = (content_width - gap).max(0.0);
    let left_width = available_width * 0.28;
    (left_width, available_width - left_width)
}

const TIMELEAP_SECONDS_FILTER_LABEL: &str = "時間①秒数";

#[cfg(test)]
pub(super) fn timeleap_filter_range_ui(
    ui: &mut egui::Ui,
    state: &mut FieldState,
) -> [egui::Rect; 2] {
    timeleap_filter_range_ui_with_texts(ui, state, &Texts::new(crate::ui::i18n::Language::Japanese))
}

pub(super) fn timeleap_filter_range_ui_with_texts(
    ui: &mut egui::Ui,
    state: &mut FieldState,
    texts: &Texts,
) -> [egui::Rect; 2] {
    let mode_row = ui.horizontal(|ui| {
        ui.label(texts.ui_label("範囲指定"));
        egui::ComboBox::from_id_salt("timeleap-filter-mode")
            .selected_text(if state.timeleap_filter_mode == 0 {
                texts.ui_label(TIMELEAP_SECONDS_FILTER_LABEL)
            } else {
                texts.ui_label("Frame1")
            })
            .width((ui.available_width() - 4.0).max(72.0))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut state.timeleap_filter_mode,
                    0,
                    texts.ui_label(TIMELEAP_SECONDS_FILTER_LABEL),
                );
                ui.selectable_value(&mut state.timeleap_filter_mode, 1, texts.ui_label("Frame1"));
            });
    });
    let values_row = ui.horizontal(|ui| {
        let (min, max, width, unit) = if state.timeleap_filter_mode == 0 {
            (
                &mut state.timeleap_seconds_min,
                &mut state.timeleap_seconds_max,
                48.0,
                "秒",
            )
        } else {
            (
                &mut state.timeleap_frame1_min,
                &mut state.timeleap_frame1_max,
                64.0,
                "F",
            )
        };
        numeric_text_edit(ui, min, width, false, false);
        ui.label(texts.ui_label("～"));
        numeric_text_edit(ui, max, width, false, false);
        ui.label(if unit == "秒" {
            texts.ui_label(unit)
        } else {
            unit
        });
    });
    [mode_row.response.rect, values_row.response.rect]
}

#[cfg(test)]
pub(super) fn timeleap_main_candidate_header_ui(ui: &mut egui::Ui) -> Vec<egui::Rect> {
    timeleap_main_candidate_header_ui_with_texts(
        ui,
        &Texts::new(crate::ui::i18n::Language::Japanese),
    )
}

pub(super) fn timeleap_main_candidate_header_ui_with_texts(
    ui: &mut egui::Ui,
    texts: &Texts,
) -> Vec<egui::Rect> {
    let mut rects = Vec::with_capacity(12);
    let (row_rect, _) = ui.allocate_exact_size(
        egui::vec2(
            timeleap_main_candidate_table_width(),
            TIMELEAP_SEARCH_TABLE_ROW_HEIGHT,
        ),
        egui::Sense::hover(),
    );
    let mut left = row_rect.left();
    for ((label, description), width) in [
        ("Frame1", "実際にタイムリープを行うFrameです。"),
        ("猶予①", TIMELEAP_MAIN_FRAME1_GRACE_HELP),
        (
            "TargetFrame",
            "実際にエンカウントするためAボタンを押すFrameです。",
        ),
        ("猶予②", TIMELEAP_MAIN_TARGET_GRACE_HELP),
        ("時間①", TIMELEAP_MAIN_SECONDS_ONE_HELP),
        ("時間②", TIMELEAP_MAIN_SECONDS_TWO_HELP),
    ]
    .into_iter()
    .zip(TIMELEAP_FRAME1_CANDIDATE_COLUMN_WIDTHS)
    {
        let cell_rect = egui::Rect::from_min_size(
            egui::pos2(left, row_rect.top()),
            egui::vec2(width, row_rect.height()),
        );
        let label_rect =
            egui::Rect::from_min_size(cell_rect.min, egui::vec2(width - 30.0, row_rect.height()));
        let help_rect = egui::Rect::from_min_size(
            egui::pos2(cell_rect.right() - 24.0, row_rect.top()),
            egui::vec2(24.0, row_rect.height()),
        );
        let label_response = ui.put(
            label_rect,
            egui::Label::new(egui::RichText::new(texts.ui_label(label)).strong()),
        );
        let help = help_button_at_rect(
            ui,
            ("main-candidate-header", label),
            texts.ui_label(description),
            help_rect,
        );
        rects.push(cell_rect);
        rects.push(label_response.rect);
        rects.push(help.rect);
        left = cell_rect.right() + TIMELEAP_FRAME1_CANDIDATE_COLUMN_GAP;
    }
    rects
}

pub(super) struct TimeleapMainCandidateRow {
    pub(super) values: [String; 6],
    pub(super) frame1: i64,
    pub(super) target_frame: i64,
    pub(super) frame1_group_tick_start: i64,
    pub(super) frame1_group_tick_end: i64,
}

fn timeleap_main_candidate_rows_start_new_group(
    previous: &TimeleapMainCandidateRow,
    current: &TimeleapMainCandidateRow,
) -> bool {
    current.frame1_group_tick_start > previous.frame1_group_tick_end.saturating_add(1)
}

#[cfg(test)]
fn timeleap_main_candidate_rows_ui(
    ui: &mut egui::Ui,
    rows: &[TimeleapMainCandidateRow],
    can_set: bool,
    offset_values: &[Option<i64>],
) -> (Vec<egui::Rect>, Option<(i64, i64)>, Option<i64>) {
    timeleap_main_candidate_rows_ui_at_with_texts(
        ui,
        rows,
        0,
        None,
        can_set,
        &Texts::new(crate::ui::i18n::Language::Japanese),
        |row_index, _frame1| offset_values.get(row_index).copied().flatten(),
    )
}

#[cfg(test)]
pub(super) fn timeleap_main_candidate_rows_ui_at(
    ui: &mut egui::Ui,
    rows: &[TimeleapMainCandidateRow],
    first_row_index: usize,
    previous_row: Option<&TimeleapMainCandidateRow>,
    can_set: bool,
    offset_for_row: impl Fn(usize, i64) -> Option<i64>,
) -> (Vec<egui::Rect>, Option<(i64, i64)>, Option<i64>) {
    timeleap_main_candidate_rows_ui_at_with_texts(
        ui,
        rows,
        first_row_index,
        previous_row,
        can_set,
        &Texts::new(crate::ui::i18n::Language::Japanese),
        offset_for_row,
    )
}

pub(super) fn timeleap_main_candidate_rows_ui_at_with_texts(
    ui: &mut egui::Ui,
    rows: &[TimeleapMainCandidateRow],
    first_row_index: usize,
    previous_row: Option<&TimeleapMainCandidateRow>,
    can_set: bool,
    texts: &Texts,
    offset_for_row: impl Fn(usize, i64) -> Option<i64>,
) -> (Vec<egui::Rect>, Option<(i64, i64)>, Option<i64>) {
    let mut rects = Vec::with_capacity(rows.len() * 6);
    let mut selected = None;
    let mut selected_offset_frame1 = None;
    let mut previous_row_rect = None;
    for (local_row_index, row) in rows.iter().enumerate() {
        let row_index = first_row_index + local_row_index;
        let previous = if local_row_index == 0 {
            previous_row
        } else {
            rows.get(local_row_index - 1)
        };
        let starts_new_group = previous
            .is_some_and(|previous| timeleap_main_candidate_rows_start_new_group(previous, row));
        let (row_rect, _) = ui.allocate_exact_size(
            egui::vec2(
                timeleap_main_candidate_table_width(),
                TIMELEAP_SEARCH_TABLE_ROW_HEIGHT,
            ),
            egui::Sense::hover(),
        );
        if row_index % 2 == 0 {
            ui.painter()
                .rect_filled(row_rect, 0.0, ui.visuals().faint_bg_color);
        }
        let mut left = row_rect.left();
        for (value, width) in row
            .values
            .iter()
            .zip(TIMELEAP_FRAME1_CANDIDATE_COLUMN_WIDTHS)
        {
            let cell_rect = egui::Rect::from_min_size(
                egui::pos2(left, row_rect.top()),
                egui::vec2(width, row_rect.height()),
            );
            let response = ui.interact(
                cell_rect,
                ui.make_persistent_id(("timeleap-frame1-candidate-cell", row_index, rects.len())),
                egui::Sense::click(),
            );
            ui.painter().text(
                cell_rect.center(),
                egui::Align2::CENTER_CENTER,
                value,
                timeleap_candidate_list_font(ui),
                ui.visuals().text_color(),
            );
            rects.push(cell_rect);
            response.context_menu(|menu_ui| {
                if menu_ui
                    .add_enabled(
                        can_set,
                        egui::Button::new(texts.ui_label("Frame1とTargetFrameをセット")),
                    )
                    .clicked()
                {
                    selected = Some((row.frame1, row.target_frame));
                    menu_ui.close_menu();
                }
                let offset_value = offset_for_row(row_index, row.frame1);
                let offset_label = texts.offset_set_label(offset_value);
                if menu_ui
                    .add_enabled(
                        can_set && offset_value.is_some(),
                        egui::Button::new(offset_label),
                    )
                    .clicked()
                {
                    selected_offset_frame1 = Some(row.frame1);
                    menu_ui.close_menu();
                }
            });
            left = cell_rect.right() + TIMELEAP_FRAME1_CANDIDATE_COLUMN_GAP;
        }
        if starts_new_group {
            ui.painter().hline(
                row_rect.x_range(),
                previous_row_rect.map_or(row_rect.top(), |previous: egui::Rect| previous.bottom()),
                ui.visuals().widgets.noninteractive.bg_stroke,
            );
        }
        previous_row_rect = Some(row_rect);
    }
    (rects, selected, selected_offset_frame1)
}

pub(super) fn timeleap_candidate_rows_virtualized_ui(
    ui: &mut egui::Ui,
    row_count: usize,
    add_visible_rows: impl FnMut(&mut egui::Ui, std::ops::Range<usize>),
) {
    egui::ScrollArea::vertical()
        .id_salt("timeleap-frame1-candidates")
        .auto_shrink([false, false])
        .show_rows(
            ui,
            TIMELEAP_SEARCH_TABLE_ROW_HEIGHT,
            row_count,
            add_visible_rows,
        );
}

fn timeleap_main_candidate_table_width() -> f32 {
    TIMELEAP_FRAME1_CANDIDATE_COLUMN_WIDTHS.iter().sum::<f32>()
        + TIMELEAP_FRAME1_CANDIDATE_COLUMN_GAP * 5.0
}

fn timeleap_candidate_list_font(ui: &egui::Ui) -> egui::FontId {
    ui.style().text_styles[&egui::TextStyle::Body].clone()
}

#[cfg(test)]
fn offset_context_menu_label(offset_after_drift: Option<i64>) -> String {
    offset_context_menu_label_with_texts(
        offset_after_drift,
        &Texts::new(crate::ui::i18n::Language::Japanese),
    )
}

fn offset_context_menu_label_with_texts(offset_after_drift: Option<i64>, texts: &Texts) -> String {
    texts.offset_set_label(offset_after_drift)
}

fn is_exact_generation_start(
    row: &crate::domain::timeleap::ConfirmedTargetSearchRow,
    requested: Option<i64>,
) -> bool {
    requested == row.generation_start_frame
}

fn is_exact_drift_search_generation_start(
    row: &crate::domain::timeleap::ConfirmedTargetSearchRow,
    requested: Option<i64>,
) -> bool {
    requested == row.generation_start_frame
}

#[cfg(test)]
fn drift_search_marker_boundary(
    rows: &[crate::domain::timeleap::ConfirmedTargetSearchRow],
    requested: i64,
) -> Option<usize> {
    crate::domain::timeleap::confirmed_target_marker_boundary(rows, requested)
}

fn confirmed_target_search_marker(ui: &egui::Ui, table_left: f32, table_right: f32, y: f32) {
    ui.painter().line_segment(
        [egui::pos2(table_left, y), egui::pos2(table_right, y)],
        egui::Stroke::new(3.0, egui::Color32::BLACK),
    );
}

const CONFIRMED_TARGET_SEARCH_COLUMN_GAP: f32 = 8.0;
const CONFIRMED_TARGET_SEARCH_CELL_INSET: f32 = 4.0;
const CONFIRMED_TARGET_SEARCH_OUTER_INSET: f32 = 6.0;

fn confirmed_target_search_column_widths() -> [f32; 5] {
    // Header labels and help buttons need their own real estate. Keep fixed widths like the
    // Main candidate table so automatic sizing cannot let a help button shift later columns.
    [80.0, 116.0, 68.0, 66.0, 150.0]
}

fn confirmed_target_search_row_rects(row_rect: egui::Rect, widths: &[f32; 5]) -> [egui::Rect; 5] {
    let mut left = row_rect.left();
    std::array::from_fn(|index| {
        let rect = egui::Rect::from_min_size(
            egui::pos2(left, row_rect.top()),
            egui::vec2(widths[index], row_rect.height()),
        );
        left = rect.right() + CONFIRMED_TARGET_SEARCH_COLUMN_GAP;
        rect
    })
}

#[cfg(test)]
fn confirmed_target_search_header(
    ui: &mut egui::Ui,
    widths: &[f32; 5],
    table_left: f32,
) -> [egui::Rect; 5] {
    confirmed_target_search_header_with_texts(
        ui,
        widths,
        table_left,
        &Texts::new(crate::ui::i18n::Language::Japanese),
    )
}

fn confirmed_target_search_header_with_texts(
    ui: &mut egui::Ui,
    widths: &[f32; 5],
    table_left: f32,
    texts: &Texts,
) -> [egui::Rect; 5] {
    let table_width = confirmed_target_search_content_width(widths);
    let (allocated_rect, _) = ui.allocate_exact_size(
        egui::vec2(
            table_width + CONFIRMED_TARGET_SEARCH_OUTER_INSET * 2.0,
            TIMELEAP_SEARCH_TABLE_ROW_HEIGHT,
        ),
        egui::Sense::hover(),
    );
    let row_rect = egui::Rect::from_min_size(
        egui::pos2(table_left, allocated_rect.top()),
        egui::vec2(table_width, allocated_rect.height()),
    );
    let cells = confirmed_target_search_row_rects(row_rect, widths);
    let body_font = ui.style().text_styles[&egui::TextStyle::Body].clone();
    for (index, ((label, explanation), cell_rect)) in [
        ("Frame1", "実際にTimeleapしたFrameです。"),
        ("TargetFrame", "Aボタンを入力するFrameです。"),
        ("ずれ", TIMELEAP_TARGET_DRIFT_HELP),
        ("猶予", TIMELEAP_TARGET_GRACE_HELP),
        ("生成開始Frame", "実際に個体生成が始まるFrameです。"),
    ]
    .into_iter()
    .zip(cells)
    .enumerate()
    {
        if !label.is_empty() {
            ui.painter().text(
                egui::pos2(
                    cell_rect.left() + CONFIRMED_TARGET_SEARCH_CELL_INSET,
                    cell_rect.center().y,
                ),
                egui::Align2::LEFT_CENTER,
                texts.ui_label(label),
                body_font.clone(),
                ui.visuals().strong_text_color(),
            );
            let help_rect = confirmed_target_search_help_rect(cell_rect);
            help_button_at_rect(
                ui,
                ("confirmed-target-header", index),
                texts.ui_label(explanation),
                help_rect,
            );
        }
    }
    cells
}

fn confirmed_target_search_help_rect(cell_rect: egui::Rect) -> egui::Rect {
    const HELP_SIDE: f32 = 20.0;
    egui::Rect::from_center_size(
        egui::pos2(
            cell_rect.right() - CONFIRMED_TARGET_SEARCH_CELL_INSET - HELP_SIDE / 2.0,
            cell_rect.center().y,
        ),
        egui::vec2(HELP_SIDE, HELP_SIDE),
    )
}

fn confirmed_target_search_content_width(widths: &[f32; 5]) -> f32 {
    widths.iter().sum::<f32>() + CONFIRMED_TARGET_SEARCH_COLUMN_GAP * (widths.len() - 1) as f32
}

struct ConfirmedTargetSearchRowsActions {
    selected_row: Option<crate::domain::timeleap::ConfirmedTargetSearchRow>,
    offset_drift_frames: Option<i64>,
    #[cfg(test)]
    visible_row_count: usize,
    #[cfg(test)]
    body_rect: egui::Rect,
    #[cfg(test)]
    scroll_offset: egui::Vec2,
    #[cfg(test)]
    content_size: egui::Vec2,
    #[cfg(test)]
    first_row_cells: [egui::Rect; 5],
}

struct ConfirmedTargetSearchTableActions {
    selected_row: Option<crate::domain::timeleap::ConfirmedTargetSearchRow>,
    offset_drift_frames: Option<i64>,
    #[cfg(test)]
    header_cells: [egui::Rect; 5],
    #[cfg(test)]
    body_rect: egui::Rect,
    #[cfg(test)]
    status_rect: egui::Rect,
    #[cfg(test)]
    scroll_offset: egui::Vec2,
    #[cfg(test)]
    content_size: egui::Vec2,
    #[cfg(test)]
    first_row_cells: [egui::Rect; 5],
}

impl Default for ConfirmedTargetSearchRowsActions {
    fn default() -> Self {
        Self {
            selected_row: None,
            offset_drift_frames: None,
            #[cfg(test)]
            visible_row_count: 0,
            #[cfg(test)]
            body_rect: egui::Rect::NOTHING,
            #[cfg(test)]
            scroll_offset: egui::Vec2::ZERO,
            #[cfg(test)]
            content_size: egui::Vec2::ZERO,
            #[cfg(test)]
            first_row_cells: [egui::Rect::NOTHING; 5],
        }
    }
}

impl Default for ConfirmedTargetSearchTableActions {
    fn default() -> Self {
        Self {
            selected_row: None,
            offset_drift_frames: None,
            #[cfg(test)]
            header_cells: [egui::Rect::NOTHING; 5],
            #[cfg(test)]
            body_rect: egui::Rect::NOTHING,
            #[cfg(test)]
            status_rect: egui::Rect::NOTHING,
            #[cfg(test)]
            scroll_offset: egui::Vec2::ZERO,
            #[cfg(test)]
            content_size: egui::Vec2::ZERO,
            #[cfg(test)]
            first_row_cells: [egui::Rect::NOTHING; 5],
        }
    }
}

#[cfg(test)]
fn confirmed_target_search_table_ui(
    ui: &mut egui::Ui,
    status: &crate::application::field::ConfirmedTargetSearchStatus,
    requested_generation_start: Option<i64>,
    drift_search_generation_start: Option<i64>,
    marker_boundary: Option<usize>,
    column_widths: &[f32; 5],
    state: &FieldState,
    fastest_close: bool,
) -> ConfirmedTargetSearchTableActions {
    confirmed_target_search_table_ui_with_texts(
        ui,
        status,
        requested_generation_start,
        drift_search_generation_start,
        marker_boundary,
        column_widths,
        state,
        fastest_close,
        &Texts::new(crate::ui::i18n::Language::Japanese),
    )
}

fn confirmed_target_search_table_ui_with_texts(
    ui: &mut egui::Ui,
    status: &crate::application::field::ConfirmedTargetSearchStatus,
    requested_generation_start: Option<i64>,
    drift_search_generation_start: Option<i64>,
    marker_boundary: Option<usize>,
    column_widths: &[f32; 5],
    state: &FieldState,
    fastest_close: bool,
    texts: &Texts,
) -> ConfirmedTargetSearchTableActions {
    use crate::application::field::ConfirmedTargetSearchStatus;

    let viewport_size = ui.available_size();
    let mut actions = ConfirmedTargetSearchTableActions::default();
    let table_width = confirmed_target_search_content_width(column_widths)
        + CONFIRMED_TARGET_SEARCH_OUTER_INSET * 2.0;

    egui::ScrollArea::horizontal()
        .id_salt("confirmed-target-horizontal")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_min_width(viewport_size.x);
            ui.set_min_height(viewport_size.y);
            let table_left = ui.cursor().left() + CONFIRMED_TARGET_SEARCH_OUTER_INSET;
            #[cfg(test)]
            {
                actions.header_cells =
                    confirmed_target_search_header_with_texts(ui, column_widths, table_left, texts);
            }
            #[cfg(not(test))]
            confirmed_target_search_header_with_texts(ui, column_widths, table_left, texts);

            match status {
                ConfirmedTargetSearchStatus::Results(rows) => {
                    let row_actions = confirmed_target_search_rows_virtualized_ui(
                        ui,
                        rows,
                        requested_generation_start,
                        drift_search_generation_start,
                        marker_boundary,
                        column_widths,
                        table_left,
                        state,
                        fastest_close,
                        texts,
                    );
                    actions.selected_row = row_actions.selected_row;
                    actions.offset_drift_frames = row_actions.offset_drift_frames;
                    #[cfg(test)]
                    {
                        actions.body_rect = row_actions.body_rect;
                        actions.scroll_offset = row_actions.scroll_offset;
                        actions.content_size = row_actions.content_size;
                        actions.first_row_cells = row_actions.first_row_cells;
                    }
                }
                status => {
                    let body_height = ui.available_height().max(0.0);
                    let (body_rect, _) = ui.allocate_exact_size(
                        egui::vec2(table_width.max(ui.available_width()), body_height),
                        egui::Sense::hover(),
                    );
                    let visible_body_rect = body_rect.intersect(ui.clip_rect());
                    #[cfg(test)]
                    {
                        let status_rect = confirmed_target_search_status_ui_with_texts(
                            ui,
                            status,
                            visible_body_rect,
                            texts,
                        );
                        actions.body_rect = visible_body_rect;
                        actions.status_rect = status_rect;
                    }
                    #[cfg(not(test))]
                    confirmed_target_search_status_ui_with_texts(
                        ui,
                        status,
                        visible_body_rect,
                        texts,
                    );
                }
            }
        });
    actions
}

const SEARCH_PROGRESS_SPINNER_SIZE: f32 = 18.0;
const SEARCH_PROGRESS_GAP: f32 = 8.0;

pub(super) fn search_progress_ui(
    ui: &mut egui::Ui,
    body_rect: egui::Rect,
    texts: &Texts,
) -> egui::Rect {
    if !body_rect.is_positive() {
        return egui::Rect::NOTHING;
    }

    let message = texts.ui_label("検索中…").to_owned();
    let color = ui.visuals().weak_text_color();
    let font_id = egui::TextStyle::Body.resolve(ui.style());
    let message_max_width =
        (body_rect.width() - SEARCH_PROGRESS_SPINNER_SIZE - SEARCH_PROGRESS_GAP).max(1.0);
    let galley = ui
        .painter()
        .layout(message, font_id, color, message_max_width);
    let message_size = galley.size();
    let group_width = SEARCH_PROGRESS_SPINNER_SIZE + SEARCH_PROGRESS_GAP + message_size.x;
    let group_left = body_rect.center().x - group_width / 2.0;
    let message_rect = egui::Rect::from_center_size(
        egui::pos2(
            group_left + SEARCH_PROGRESS_SPINNER_SIZE + SEARCH_PROGRESS_GAP + message_size.x / 2.0,
            body_rect.center().y,
        ),
        message_size,
    );
    ui.painter().galley(message_rect.min, galley, color);

    let spinner_rect = egui::Rect::from_center_size(
        egui::pos2(
            group_left + SEARCH_PROGRESS_SPINNER_SIZE / 2.0,
            body_rect.center().y,
        ),
        egui::vec2(SEARCH_PROGRESS_SPINNER_SIZE, SEARCH_PROGRESS_SPINNER_SIZE),
    );
    ui.put(spinner_rect, egui::Spinner::new());
    message_rect.union(spinner_rect)
}

pub(super) fn frame1_search_progress_ui(ui: &mut egui::Ui, texts: &Texts) -> egui::Rect {
    let (body_rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ui.available_height()),
        egui::Sense::hover(),
    );
    let visible_body_rect = body_rect.intersect(ui.clip_rect());
    search_progress_ui(ui, visible_body_rect, texts)
}

fn confirmed_target_search_status_ui_with_texts(
    ui: &mut egui::Ui,
    status: &crate::application::field::ConfirmedTargetSearchStatus,
    body_rect: egui::Rect,
    texts: &Texts,
) -> egui::Rect {
    use crate::application::field::ConfirmedTargetSearchStatus;

    if !body_rect.is_positive() || matches!(status, ConfirmedTargetSearchStatus::Idle) {
        return egui::Rect::NOTHING;
    }

    if matches!(status, ConfirmedTargetSearchStatus::Running) {
        return search_progress_ui(ui, body_rect, texts);
    }

    let (text, color) = match status {
        ConfirmedTargetSearchStatus::Idle => return egui::Rect::NOTHING,
        ConfirmedTargetSearchStatus::NoResults => (
            texts
                .ui_label("条件に一致する候補はありません。")
                .to_owned(),
            ui.visuals().weak_text_color(),
        ),
        ConfirmedTargetSearchStatus::Frame1Unreachable => (
            texts
                .ui_label("指定したFrame1には到達できません。")
                .to_owned(),
            ui.visuals().weak_text_color(),
        ),
        ConfirmedTargetSearchStatus::GenerationFrameUnreachable => (
            texts
                .ui_label("指定した生成開始Frameには到達できません。")
                .to_owned(),
            egui::Color32::LIGHT_RED,
        ),
        ConfirmedTargetSearchStatus::Failed(message) => {
            (texts.notice(message), egui::Color32::LIGHT_RED)
        }
        ConfirmedTargetSearchStatus::Running => unreachable!(),
        ConfirmedTargetSearchStatus::Results(_) => unreachable!(),
    };
    let font_id = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui
        .painter()
        .layout(text, font_id, color, body_rect.width().max(1.0));
    let text_size = galley.size();
    let text_rect = egui::Rect::from_center_size(body_rect.center(), text_size);
    ui.painter().galley(text_rect.min, galley, color);
    text_rect
}

fn confirmed_target_search_rows_virtualized_ui(
    ui: &mut egui::Ui,
    rows: &[crate::domain::timeleap::ConfirmedTargetSearchRow],
    requested_generation_start: Option<i64>,
    drift_search_generation_start: Option<i64>,
    marker_boundary: Option<usize>,
    column_widths: &[f32; 5],
    table_left: f32,
    state: &FieldState,
    fastest_close: bool,
    texts: &Texts,
) -> ConfirmedTargetSearchRowsActions {
    let mut actions = ConfirmedTargetSearchRowsActions::default();
    let mut previous_generation_cell = None;

    let scroll_output = egui::ScrollArea::vertical()
        .id_salt("confirmed-target-vertical")
        .auto_shrink([false, false])
        .show_rows(
            ui,
            TIMELEAP_SEARCH_TABLE_ROW_HEIGHT,
            rows.len(),
            |ui, visible_rows| {
                #[cfg(test)]
                let first_visible_index = visible_rows.start;
                for visible_index in visible_rows {
                    let index = visible_index;
                    let row = &rows[index];
                    let (cells, set_clicked, offset_clicked) =
                        confirmed_target_search_row_with_set_policy_and_texts(
                            ui,
                            row,
                            column_widths,
                            table_left,
                            index,
                            is_exact_generation_start(row, requested_generation_start),
                            is_exact_drift_search_generation_start(
                                row,
                                drift_search_generation_start,
                            ),
                            state.preview_timeleap_offset_drift(row.drift_frames, fastest_close),
                            state.can_set_confirmed_target_frame_from_search(),
                            texts,
                        );
                    if marker_boundary == Some(index) {
                        let line_y = previous_generation_cell
                            .map_or(cells[4].top(), |previous: egui::Rect| {
                                (previous.bottom() + cells[4].top()) / 2.0
                            });
                        confirmed_target_search_marker(
                            ui,
                            cells[0].left(),
                            cells[4].right(),
                            line_y,
                        );
                    }
                    if marker_boundary == Some(rows.len()) && index + 1 == rows.len() {
                        confirmed_target_search_marker(
                            ui,
                            cells[0].left(),
                            cells[4].right(),
                            cells[4].bottom(),
                        );
                    }
                    previous_generation_cell = Some(cells[4]);
                    #[cfg(test)]
                    {
                        actions.visible_row_count += 1;
                        if index == first_visible_index {
                            actions.first_row_cells = cells;
                        }
                    }
                    if set_clicked {
                        actions.selected_row = Some(row.clone());
                    }
                    if offset_clicked {
                        actions.offset_drift_frames = Some(row.drift_frames);
                    }
                }
            },
        );
    #[cfg(test)]
    {
        actions.body_rect = scroll_output.inner_rect;
        actions.scroll_offset = scroll_output.state.offset;
        actions.content_size = scroll_output.content_size;
    }
    #[cfg(not(test))]
    let _ = scroll_output;
    actions
}

#[cfg(test)]
fn confirmed_target_search_table_left(
    ui: &egui::Ui,
    _table_width: f32,
    _viewport_width: f32,
) -> f32 {
    ui.cursor().left() + CONFIRMED_TARGET_SEARCH_OUTER_INSET
}

#[cfg(test)]
fn target_frame_set_menu_button(ui: &mut egui::Ui, enabled: bool) -> egui::Response {
    target_frame_set_menu_button_with_texts(
        ui,
        enabled,
        &Texts::new(crate::ui::i18n::Language::Japanese),
    )
}

fn target_frame_set_menu_button_with_texts(
    ui: &mut egui::Ui,
    enabled: bool,
    texts: &Texts,
) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(texts.ui_label("TargetFrameをセット")),
    )
}

#[cfg(test)]
fn confirmed_target_search_row(
    ui: &mut egui::Ui,
    row: &crate::domain::timeleap::ConfirmedTargetSearchRow,
    widths: &[f32; 5],
    table_left: f32,
    row_index: usize,
    highlighted: bool,
    outlined: bool,
    offset_after_drift: Option<i64>,
) -> ([egui::Rect; 5], bool, bool) {
    confirmed_target_search_row_with_set_policy_and_texts(
        ui,
        row,
        widths,
        table_left,
        row_index,
        highlighted,
        outlined,
        offset_after_drift,
        true,
        &Texts::new(crate::ui::i18n::Language::Japanese),
    )
}

fn confirmed_target_search_row_with_set_policy_and_texts(
    ui: &mut egui::Ui,
    row: &crate::domain::timeleap::ConfirmedTargetSearchRow,
    widths: &[f32; 5],
    table_left: f32,
    row_index: usize,
    highlighted: bool,
    outlined: bool,
    offset_after_drift: Option<i64>,
    can_set_target: bool,
    texts: &Texts,
) -> ([egui::Rect; 5], bool, bool) {
    let table_width = confirmed_target_search_content_width(widths);
    let (allocated_rect, _) = ui.allocate_exact_size(
        egui::vec2(
            table_width + CONFIRMED_TARGET_SEARCH_OUTER_INSET * 2.0,
            TIMELEAP_SEARCH_TABLE_ROW_HEIGHT,
        ),
        egui::Sense::hover(),
    );
    let row_rect = egui::Rect::from_min_size(
        egui::pos2(table_left, allocated_rect.top()),
        egui::vec2(table_width, allocated_rect.height()),
    );
    if highlighted {
        ui.painter()
            .rect_filled(row_rect, 0.0, ui.visuals().selection.bg_fill);
    } else if row_index % 2 == 0 {
        ui.painter()
            .rect_filled(row_rect, 0.0, ui.visuals().faint_bg_color);
    }
    let mut set_clicked = false;
    let mut offset_clicked = false;
    let row_response = ui.interact(
        row_rect,
        ui.make_persistent_id(("confirmed-target-row", row.frame1, row.target_frame)),
        egui::Sense::click(),
    );
    row_response.context_menu(|menu_ui| {
        if target_frame_set_menu_button_with_texts(menu_ui, can_set_target, texts).clicked() {
            set_clicked = true;
            menu_ui.close_menu();
        }
        let offset_label = offset_context_menu_label_with_texts(offset_after_drift, texts);
        if menu_ui
            .add_enabled(
                offset_after_drift.is_some(),
                egui::Button::new(offset_label),
            )
            .clicked()
        {
            offset_clicked = true;
            menu_ui.close_menu();
        }
    });
    let cells = confirmed_target_search_row_rects(row_rect, widths);
    let body_font = timeleap_candidate_list_font(ui);
    let values = [
        row.frame1.to_string(),
        row.target_frame.to_string(),
        format!("{:+}F", row.drift_frames),
        format!("{}F", row.grace_frames),
        row.generation_start_frame
            .map_or_else(|| "—".to_owned(), |frame| frame.to_string()),
    ];
    for (value, cell_rect) in values.iter().zip(cells.iter().take(5)) {
        ui.painter().text(
            cell_rect.center(),
            egui::Align2::CENTER_CENTER,
            value,
            body_font.clone(),
            ui.visuals().text_color(),
        );
    }
    if outlined {
        ui.painter().rect_stroke(
            row_rect.shrink(1.0),
            0.0,
            egui::Stroke::new(1.5, egui::Color32::BLACK),
        );
    }
    (
        cells,
        set_clicked,
        offset_after_drift.is_some() && offset_clicked,
    )
}

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;
