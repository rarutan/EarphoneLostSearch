use super::*;

fn courier_import_notice_visible(
    currently_visible: bool,
    courier_panel_interacted: bool,
    import_failed: bool,
) -> bool {
    if import_failed {
        true
    } else if courier_panel_interacted {
        false
    } else {
        currently_visible
    }
}

pub(super) struct FieldSettingsActions {
    pub(super) target_committed: bool,
    pub(super) idx_inference_requested: bool,
    pub(super) idx_inference_cancel_requested: bool,
    pub(super) target_offset_changed: bool,
    pub(super) open_offset_manager: bool,
    pub(super) timeleap_frame1_preview_requested: bool,
    pub(super) timeleap_enabled_changed: bool,
    pub(super) timeleap_candidate_search_requested: bool,
    /// Timeleap自体をオフにしたときの、関連状態を破棄するキャンセル。
    pub(super) timeleap_session_cancel_requested: bool,
    /// Frame1検索workerだけを停止する明示的なキャンセル。
    pub(super) timeleap_candidate_search_cancel_requested: bool,
    pub(super) timeleap_reachability_search_requested: bool,
    pub(super) confirmed_target_search_requested: bool,
    pub(super) confirmed_target_search_cancel_requested: bool,
    pub(super) confirmed_target_search_row:
        Option<crate::domain::timeleap::ConfirmedTargetSearchRow>,
    pub(super) confirmed_target_offset_drift_frames: Option<i64>,
    pub(super) frame1_offset_to_set: Option<i64>,
    #[cfg(test)]
    pub(super) timeleap_controls_layout: TimeleapMainControlsLayout,
}

pub(super) fn field_settings_ui(
    ui: &mut egui::Ui,
    state: &mut FieldState,
    calculating: bool,
    timeleap_search_running: bool,
    confirmed_target_search_worker_running: bool,
    idx_inference_running: bool,
    fps: f64,
    color_scheme: ColorScheme,
    wheel_offset_enabled: bool,
    texts: &Texts,
    fastest_close: bool,
    viewport_only: bool,
) -> FieldSettingsActions {
    let panel_width = ui.max_rect().width();
    let mut target_committed = false;
    let mut idx_inference_requested = false;
    let mut idx_inference_cancel_requested = false;
    let mut open_offset_manager = false;
    let mut timeleap_frame1_preview_requested = false;
    let mut timeleap_candidate_search_requested = false;
    let mut timeleap_candidate_search_cancel_requested = false;
    let timeleap_reachability_search_requested = false;
    let mut confirmed_target_search_requested = false;
    let mut confirmed_target_search_cancel_requested = false;
    let mut confirmed_target_search_row = None;
    let mut confirmed_target_offset_drift_frames = None;
    let mut frame1_offset_to_set = None;
    if !viewport_only {
        ui.heading(texts.conditions());
    }
    let inputs_locked = calculating || idx_inference_running;
    let general_editable =
        !inputs_locked && !state.timer_is_running() && state.can_edit_general_settings();
    let correction_editable =
        !inputs_locked && state.can_edit_correction_inputs() && !state.timer_is_running();
    let seed_npc_editable = general_editable && state.session_phase() == SessionPhase::Input;
    let other_editable = general_editable || correction_editable;
    #[cfg(test)]
    let mut npc_row_rect = egui::Rect::NOTHING;
    // 孵化側と同じく、基本条件を左、オフセットを右の列へ置く。
    ui.columns(2, |columns| {
        columns[0].vertical(|ui| {
            if !viewport_only {
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
                        match state.commit_target_input() {
                            Err(_) => state.clear_notice(),
                            Ok(()) if state.target_consumption.trim().is_empty() => {
                                state.clear_target_prediction(fps);
                                state.clear_notice();
                            }
                            Ok(()) if state.results.len() == 1 => {
                                state.prepare_target_research(fps);
                                target_committed = true;
                            }
                            Ok(()) => {
                                state.invalidate_search_results();
                                target_committed = true;
                            }
                        }
                    }
                });
            });
            let _npc_row = ui.horizontal(|ui| {
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
            #[cfg(test)]
            {
                // Order regression tests use the real NPC row rectangle from the left column.
                // The controls layout is returned after both columns have been rendered.
                npc_row_rect = _npc_row.response.rect;
            }
            }
            // Native viewportの生成は表示行ではない。UIレイアウトを確保しないブロックで
            // 呼び出し、空のhorizontalが確保していた既定の行高を追加しない。
            {
                if state.timeleap_search_window_open {
                    let viewport_id = egui::ViewportId::from_hash_of("timeleap-search");
                    let viewport_builder = egui::ViewportBuilder::default()
                        .with_title(texts.ui_label("Timeleap検索"))
                        .with_inner_size([TIMELEAP_SEARCH_WINDOW_WIDTH, TIMELEAP_SEARCH_WINDOW_HEIGHT])
                        .with_min_inner_size([TIMELEAP_SEARCH_WINDOW_WIDTH, TIMELEAP_SEARCH_WINDOW_HEIGHT])
                        .with_max_inner_size([TIMELEAP_SEARCH_WINDOW_WIDTH, TIMELEAP_SEARCH_WINDOW_HEIGHT]);
                    let should_close = ui.ctx().show_viewport_immediate(
                        viewport_id,
                        viewport_builder,
                        |viewport_ctx, viewport_class| {
                            let mut render_contents = |ui: &mut egui::Ui| {
                            state.timeleap_search_tab = state
                                .timeleap_search_tab
                                .min(TIMELEAP_TAB_SETTINGS);
                            let header = timeleap_search_header_ui(ui, state, texts);
                            timeleap_frame1_preview_requested |= header.preview_requested;
                            ui.separator();
                            if state.timeleap_search_tab == TIMELEAP_TAB_TARGET_SEARCH {
                                state.initialize_confirmed_target_search_form();
                            }
                            if state.timeleap_search_tab == TIMELEAP_TAB_FRAME1_SEARCH {
                                let _ = timeleap_search_metadata_ui_with_texts(
                                    ui,
                                    state,
                                    false,
                                    seed_npc_editable,
                                    false,
                                    texts,
                                );
                                ui.separator();

                                let mut frame1_to_set = None;
                                let content_width = ui.available_width();
                                let content_height = ui.available_height();
                                let column_gap = ui.spacing().item_spacing.x;
                                let (left_width, right_width) =
                                    timeleap_search_column_widths(content_width, column_gap);
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
                                            "frame1-search",
                                            left_width,
                                            Some((420.0, 460.0)),
                                        );
                                        timeleap_frame1_preview_requested |=
                                            selection_controls.preview_requested;
                                        timeleap_frame1_preview_requested |=
                                            timeleap_search_options_ui(ui, state, texts)
                                                .preview_requested;
                                        ui.separator();
                                        let _ = timeleap_generation_start_frame_row_with_texts(
                                            ui,
                                            left_width,
                                            "main-generation-start",
                                            texts,
                                            |ui| {
                                                numeric_text_edit(
                                                    ui,
                                                    &mut state.timeleap_generation_start_frame,
                                                    103.0,
                                                    false,
                                                    false,
                                                )
                                            },
                                        );
                                        let _range_rows =
                                            timeleap_filter_range_ui_with_texts(ui, state, texts);
                                        ui.horizontal(|ui| {
                                            ui.label(texts.ui_label("猶予①"));
                                            numeric_text_edit(ui, &mut state.timeleap_grace_min, 48.0, false, false);
                                            ui.label(texts.ui_label("F以上"));
                                            help_button(ui, "main-frame1-grace", texts.ui_label(TIMELEAP_MAIN_FRAME1_GRACE_FILTER_HELP));
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label(texts.ui_label("猶予②"));
                                            numeric_text_edit(ui, &mut state.timeleap_target_grace_min, 48.0, false, false);
                                            ui.label(texts.ui_label("F以上"));
                                            help_button(ui, "main-target-grace", texts.ui_label(TIMELEAP_MAIN_TARGET_GRACE_FILTER_HELP));
                                        });
                                        let can_search = state.timeleap_enabled
                                            && state.timeleap_lifecycle_policy().candidate_search_enabled
                                            && !state.timeleap_frame1_preparing
                                            && state.timeleap_candidate_search_inputs_ready(fps);
                                        let search_response = timeleap_candidate_search_button_with_texts(
                                            ui,
                                            can_search || timeleap_search_running,
                                            left_width,
                                            texts,
                                            timeleap_search_running,
                                        );
                                        if search_response.clicked() {
                                            match search_button_action(timeleap_search_running) {
                                                SearchButtonAction::Search => {
                                                    timeleap_candidate_search_requested = true;
                                                }
                                                SearchButtonAction::Cancel => {
                                                    timeleap_candidate_search_cancel_requested =
                                                        true;
                                                }
                                            }
                                        }
                                        },
                                    );

                                    ui.allocate_ui_with_layout(
                                        egui::vec2(right_width, content_height),
                                        egui::Layout::top_down_justified(egui::Align::Min),
                                        |ui| {
                                        let _ = timeleap_main_candidate_header_ui_with_texts(ui, texts);
                                        if timeleap_search_running {
                                            super::timeleap_search::frame1_search_progress_ui(
                                                ui, texts,
                                            );
                                        } else if state.timeleap_search_completed_without_candidates() {
                                            ui.allocate_ui_with_layout(
                                                egui::vec2(ui.available_width(), ui.available_height()),
                                                egui::Layout::centered_and_justified(
                                                    egui::Direction::TopDown,
                                                ),
                                                |ui| {
                                                    ui.label(texts.ui_label("検索結果なし"));
                                                },
                                            );
                                        } else {
                                            let groups = &state.timeleap_candidate_groups;
                                            let now = Instant::now();
                                            let can_set = !state
                                                .timeleap_lifecycle_policy()
                                                .configuration_locked
                                                && !state.timeleap_frame1_preparing;
                                            let mut selected = None;
                                            let mut selected_offset = None;
                                            timeleap_candidate_rows_virtualized_ui(
                                                    ui,
                                                    groups.len(),
                                                    |ui, row_range| {
                                                        let rows = groups[row_range.clone()]
                                                            .iter()
                                                            .map(|group| {
                                                                let frame1 = group.representative_frame1;
                                                                let remaining = group
                                                                    .representative_candidate()
                                                                    .map(|candidate| {
                                                                        state.timeleap_candidate_seconds_remaining(
                                                                            candidate.frame1_tick,
                                                                            fps,
                                                                            now,
                                                                        )
                                                                    })
                                                                    .unwrap_or_default();
                                                                let to_target = crate::domain::timing::blink_ticks_to_seconds(
                                                                    group.frame2_ticks,
                                                                    fps,
                                                                );
                                                                TimeleapMainCandidateRow {
                                                                    values: [
                                                                        frame1.to_string(),
                                                                        group.grace_frames.to_string(),
                                                                        group.representative_target_frame.to_string(),
                                                                        group.target_grace_frames.to_string(),
                                                                        format_timeleap_duration(remaining),
                                                                        format_timeleap_duration(to_target.floor() as i64),
                                                                    ],
                                                                    frame1,
                                                                    target_frame: group.representative_target_frame,
                                                                    frame1_group_tick_start: group.frame1_tick_start,
                                                                    frame1_group_tick_end: group.frame1_tick_end,
                                                                }
                                                            })
                                                            .collect::<Vec<_>>();
                                                        let previous = row_range
                                                            .start
                                                            .checked_sub(1)
                                                            .and_then(|index| groups.get(index))
                                                            .map(|group| TimeleapMainCandidateRow {
                                                                values: Default::default(),
                                                                frame1: group.representative_frame1,
                                                                target_frame: group.representative_target_frame,
                                                                frame1_group_tick_start: group.frame1_tick_start,
                                                                frame1_group_tick_end: group.frame1_tick_end,
                                                            });
                                                        let (_, row_selection, offset_selection) =
                                                            timeleap_main_candidate_rows_ui_at_with_texts(
                                                                ui,
                                                                &rows,
                                                                row_range.start,
                                                                previous.as_ref(),
                                                                can_set,
                                                                texts,
                                                                |_, frame1| {
                                                                    state.preview_timeleap_frame1_offset(
                                                                        frame1,
                                                                        fastest_close,
                                                                    )
                                                                },
                                                            );
                                                        selected = row_selection.or(selected);
                                                        selected_offset = offset_selection.or(selected_offset);
                                                    },
                                                );
                                            frame1_to_set = selected.or(frame1_to_set);
                                            frame1_offset_to_set =
                                                selected_offset.or(frame1_offset_to_set);
                                        }
                                        },
                                    );
                                });
                                if let Some((frame1, target_frame)) = frame1_to_set {
                                    if state.set_timeleap_candidate_inputs(frame1, target_frame) {
                                        if state.results.len() == 1 {
                                            state.prepare_target_research(fps);
                                        } else {
                                            state.invalidate_search_results();
                                        }
                                        target_committed = true;
                                    }
                                    state.invalidate_timeleap_frame1_preview();
                                    timeleap_frame1_preview_requested = true;
                                }
                            } else if state.timeleap_search_tab == TIMELEAP_TAB_COURIER {
                            const COURIER_UI_SCALE: f32 = 1.3;
                            let ctx = ui.ctx().clone();
                            let courier_panel_rect = ui.available_rect_before_wrap();
                            let courier_import_error_id =
                                egui::Id::new("timeleap-courier-import-error");
                            let courier_panel_clicked = ctx.input(|input| {
                                input.pointer.any_pressed()
                                    && input
                                        .pointer
                                        .interact_pos()
                                        .is_some_and(|position| courier_panel_rect.contains(position))
                            });
                            let courier_import_failed_before = ctx
                                .data(|data| data.get_temp::<bool>(courier_import_error_id))
                                .unwrap_or(false);
                            let mut courier_import_failed = courier_import_notice_visible(
                                courier_import_failed_before,
                                courier_panel_clicked,
                                false,
                            );
                            if courier_panel_clicked {
                                courier_import_failed = false;
                                ctx.data_mut(|data| {
                                    data.insert_temp(courier_import_error_id, false)
                                });
                            }
                            let original_style = ui.style().clone();
                            let mut courier_style = original_style.as_ref().clone();
                            for font in courier_style.text_styles.values_mut() {
                                font.size *= COURIER_UI_SCALE;
                            }
                            courier_style.spacing.item_spacing *= COURIER_UI_SCALE;
                            courier_style.spacing.button_padding *= COURIER_UI_SCALE;
                            courier_style.spacing.interact_size *= COURIER_UI_SCALE;
                            courier_style.spacing.combo_width *= COURIER_UI_SCALE;
                            courier_style.spacing.text_edit_width *= COURIER_UI_SCALE;
                            courier_style.spacing.icon_width *= COURIER_UI_SCALE;
                            courier_style.spacing.icon_width_inner *= COURIER_UI_SCALE;
                            courier_style.spacing.icon_spacing *= COURIER_UI_SCALE;
                            courier_style.spacing.combo_height *= COURIER_UI_SCALE;
                            ui.set_style(courier_style);
                            ui.add_enabled_ui(
                                state.timeleap_enabled
                                    && !state.timeleap_lifecycle_policy().configuration_locked,
                                |ui| {
                                    ui.horizontal(|ui| {
                                        let species_name = texts
                                            .species_name(state.timeleap_courier_species_id)
                                            .unwrap_or(texts.ui_label("ポケモン"));
                                        egui::ComboBox::from_id_salt(
                                            "field-timeleap-courier-species",
                                        )
                                        .selected_text(species_name)
                                        .width(150.0 * COURIER_UI_SCALE)
                                        .show_ui(ui, |ui| {
                                            for species_id in 0..texts.species_count() {
                                                let name = texts
                                                    .species_name(species_id as u16)
                                                    .unwrap_or("-");
                                                ui.selectable_value(
                                                    &mut state.timeleap_courier_species_id,
                                                    species_id as u16,
                                                    name,
                                                );
                                            }
                                        });
                                        if ui.button("Import").clicked() {
                                            if let Some(path) = rfd::FileDialog::new()
                                                .add_filter("Wonder Card", &["wc7", "wc7full"])
                                                .pick_file()
                                            {
                                                match std::fs::read(&path)
                                                    .map_err(|error| error.to_string())
                                                    .and_then(|data| {
                                                        crate::domain::wondercard::parse(&data)
                                                    }) {
                                                    Ok(settings) => {
                                                        timeleap_frame1_preview_requested |=
                                                            apply_wondercard_settings(state, settings);
                                                    }
                                                    Err(_error) => {
                                                        courier_import_failed = true;
                                                        ctx.data_mut(|data| {
                                                            data.insert_temp(courier_import_error_id, true)
                                                        });
                                                    }
                                                };
                                            }
                                        }
                                    });
                                    if courier_import_failed {
                                        ui.label(texts.wc7_import_failed());
                                    }
                                    let settings_content_height = ui.available_height();
                                    ui.horizontal(|ui| {
                                        ui.allocate_ui_with_layout(
                                            egui::vec2(116.0 * COURIER_UI_SCALE, settings_content_height),
                                            egui::Layout::top_down(egui::Align::Min),
                                            |ui| {
                                            let label_width = 70.0 * COURIER_UI_SCALE;
                                            let available_height = ui.available_height();
                                            ui.horizontal(|ui| {
                                                ui.allocate_ui_with_layout(
                                                    egui::vec2(label_width, available_height),
                                                    egui::Layout::top_down(egui::Align::Min),
                                                    |ui| {
                                                        for (index, label) in [
                                                            "HP", "攻撃", "防御", "特攻", "特防", "素早さ",
                                                        ]
                                                        .into_iter()
                                                        .enumerate()
                                                        {
                                                            let mut fixed =
                                                                state.timeleap_courier_fixed_ivs[index] >= 0;
                                                            if ui.checkbox(&mut fixed, texts.ui_label(label)).changed() {
                                                                state.timeleap_courier_fixed_ivs[index] =
                                                                    if fixed { 0 } else { -1 };
                                                            }
                                                        }
                                                    },
                                                );
                                                ui.vertical(|ui| {
                                                    for index in 0..6 {
                                                        let fixed =
                                                            state.timeleap_courier_fixed_ivs[index] >= 0;
                                                        let mut value_text = state
                                                            .timeleap_courier_fixed_ivs[index]
                                                            .max(0)
                                                            .to_string();
                                                        ui.push_id(("courier-iv", index), |ui| {
                                                            ui.add_enabled_ui(fixed, |ui| {
                                                                if frame_spinner(
                                                                    ui,
                                                                    &mut value_text,
                                                                    34.0 * COURIER_UI_SCALE,
                                                                    1,
                                                                    true,
                                                                ) {
                                                                    if let Ok(parsed) =
                                                                        value_text.parse::<i16>()
                                                                    {
                                                                        state.timeleap_courier_fixed_ivs[index] =
                                                                            parsed.clamp(0, 31) as i8;
                                                                    }
                                                                }
                                                            });
                                                        });
                                                    }
                                                });
                                            });
                                            },
                                        );
                                        ui.add_space(14.0 * COURIER_UI_SCALE);
                                        ui.allocate_ui_with_layout(
                                            egui::vec2(220.0 * COURIER_UI_SCALE, settings_content_height),
                                            egui::Layout::top_down(egui::Align::Min),
                                            |ui| {
                                            egui::Grid::new("field-timeleap-courier-settings-grid")
                                                .num_columns(2)
                                                .spacing([
                                                    6.0 * COURIER_UI_SCALE,
                                                    4.0 * COURIER_UI_SCALE,
                                                ])
                                                .show(ui, |ui| {
                                                ui.label(texts.ui_label("保証V数"));
                                                iv_count_spinner(
                                                    ui,
                                                    &mut state.timeleap_courier_iv_count,
                                                    42.0 * COURIER_UI_SCALE,
                                                    true,
                                                );
                                                ui.end_row();

                                                ui.horizontal(|ui| {
                                                let egg_changed = ui.checkbox(
                                                    &mut state.timeleap_courier_is_egg,
                                                    texts.ui_label("卵"),
                                                ).changed();
                                                let your_id_changed = ui.checkbox(
                                                    &mut state.timeleap_courier_your_id,
                                                    texts.ui_label("自ID"),
                                                ).changed();
                                                if egg_changed || your_id_changed {
                                                    timeleap_frame1_preview_requested |=
                                                        state.courier_condition_changed();
                                                }
                                                });
                                                ui.label("");
                                                ui.end_row();

                                                ui.label(texts.ui_label("PIDタイプ"));
                                                egui::ComboBox::from_id_salt(
                                                    "field-timeleap-pid-type",
                                                )
                                                .width(110.0 * COURIER_UI_SCALE)
                                                .selected_text(match state.timeleap_courier_pid_type {
                                                    1 => texts.ui_label("通常色"),
                                                    2 => texts.ui_label("色違い"),
                                                    3 => texts.ui_label("指定"),
                                                    _ => texts.ui_label("乱数"),
                                                })
                                                .show_ui(ui, |ui| {
                                                    for (value, label) in [
                                                        (0, texts.ui_label("乱数")),
                                                        (1, texts.ui_label("通常色")),
                                                        (2, texts.ui_label("色違い")),
                                                        (3, texts.ui_label("指定")),
                                                    ] {
                                                        ui.selectable_value(
                                                            &mut state.timeleap_courier_pid_type,
                                                            value,
                                                            label,
                                                        );
                                                    }
                                                });
                                                ui.end_row();

                                                ui.checkbox(
                                                    &mut state.timeleap_courier_ability_locked,
                                                    texts.ui_label("特性固定"),
                                                );
                                                ui.add_enabled_ui(
                                                    state.timeleap_courier_ability_locked,
                                                    |ui| {
                                                        egui::ComboBox::from_id_salt(
                                                            "field-timeleap-ability",
                                                        )
                                                        .width(110.0 * COURIER_UI_SCALE)
                                                        .selected_text(
                                                            match state.timeleap_courier_ability {
                                                                1 => "1",
                                                                2 => "2",
                                                                3 => "H",
                                                                _ => texts.ui_label("乱数"),
                                                            },
                                                        )
                                                        .show_ui(ui, |ui| {
                                                            for (value, label) in
                                                                [(1, "1"), (2, "2"), (3, "H")]
                                                            {
                                                                ui.selectable_value(
                                                                    &mut state
                                                                        .timeleap_courier_ability,
                                                                    value,
                                                                    label,
                                                                );
                                                            }
                                                        })
                                                    },
                                                );
                                                ui.end_row();

                                                ui.checkbox(
                                                    &mut state.timeleap_courier_nature_locked,
                                                    texts.ui_label("性格固定"),
                                                );
                                                ui.add_enabled_ui(
                                                    state.timeleap_courier_nature_locked,
                                                    |ui| {
                                                        egui::ComboBox::from_id_salt(
                                                            "field-timeleap-nature",
                                                        )
                                                        .width(110.0 * COURIER_UI_SCALE)
                                                        .selected_text(
                                                            texts
                                                                .nature_name(state.timeleap_courier_nature)
                                                                .unwrap_or(""),
                                                        )
                                                        .show_ui(ui, |ui| {
                                                            for index in 0..25u8 {
                                                                let label = texts.nature_name(index).unwrap_or("");
                                                                ui.selectable_value(
                                                                    &mut state
                                                                        .timeleap_courier_nature,
                                                                    index as u8,
                                                                    label,
                                                                );
                                                            }
                                                        });
                                                    },
                                                );
                                                ui.end_row();

                                                ui.checkbox(
                                                    &mut state.timeleap_courier_gender_locked,
                                                    texts.ui_label("性別固定"),
                                                );
                                                ui.add_enabled_ui(
                                                    state.timeleap_courier_gender_locked,
                                                    |ui| {
                                                        egui::ComboBox::from_id_salt(
                                                            "field-timeleap-gender",
                                                        )
                                                        .width(110.0 * COURIER_UI_SCALE)
                                                        .selected_text(
                                                            match state
                                                                .timeleap_courier_gender_ratio
                                                            {
                                                                1 => "♂",
                                                                2 => "♀",
                                                                _ => "-",
                                                            },
                                                        )
                                                        .show_ui(ui, |ui| {
                                                            ui.selectable_value(
                                                                &mut state
                                                                    .timeleap_courier_gender_ratio,
                                                                1,
                                                                "♂",
                                                            );
                                                            ui.selectable_value(
                                                                &mut state
                                                                    .timeleap_courier_gender_ratio,
                                                                2,
                                                                "♀",
                                                            );
                                                            ui.selectable_value(
                                                                &mut state
                                                                    .timeleap_courier_gender_ratio,
                                                                0,
                                                                "-",
                                                            );
                                                        })
                                                    },
                                                );
                                                ui.end_row();
                                            });
                                            },
                                        );
                                        ui.add_space(14.0 * COURIER_UI_SCALE);
                                        ui.allocate_ui_with_layout(
                                            egui::vec2(126.0 * COURIER_UI_SCALE, settings_content_height),
                                            egui::Layout::top_down(egui::Align::Min),
                                            |ui| {
                                            ui.checkbox(
                                                &mut state.timeleap_courier_other_info,
                                                texts.ui_label("その他"),
                                            );
                                            ui.add_enabled_ui(
                                                state.timeleap_courier_other_info,
                                                |ui| {
                                                    ui.horizontal(|ui| {
                                                        ui.add_sized(
                                                            [
                                                                28.0 * COURIER_UI_SCALE,
                                                                20.0 * COURIER_UI_SCALE,
                                                            ],
                                                            egui::Label::new("TID"),
                                                        );
                                                        ui.add(
                                                            egui::TextEdit::singleline(
                                                                &mut state.timeleap_courier_tid,
                                                            )
                                                            .desired_width(58.0 * COURIER_UI_SCALE),
                                                        );
                                                    });
                                                    ui.horizontal(|ui| {
                                                        ui.add_sized(
                                                            [
                                                                28.0 * COURIER_UI_SCALE,
                                                                20.0 * COURIER_UI_SCALE,
                                                            ],
                                                            egui::Label::new("SID"),
                                                        );
                                                        ui.add(
                                                            egui::TextEdit::singleline(
                                                                &mut state.timeleap_courier_sid,
                                                            )
                                                            .desired_width(58.0 * COURIER_UI_SCALE),
                                                        );
                                                    });
                                                },
                                            );
                                            ui.add_enabled_ui(
                                                state.timeleap_courier_pid_type == 3,
                                                |ui| {
                                                    ui.horizontal(|ui| {
                                                        ui.add_sized(
                                                            [
                                                                28.0 * COURIER_UI_SCALE,
                                                                20.0 * COURIER_UI_SCALE,
                                                            ],
                                                            egui::Label::new("PID"),
                                                        );
                                                        ui.add(
                                                            egui::TextEdit::singleline(
                                                                &mut state.timeleap_courier_pid,
                                                            )
                                                            .desired_width(90.0 * COURIER_UI_SCALE),
                                                        );
                                                    });
                                                    ui.horizontal(|ui| {
                                                        ui.add_sized(
                                                            [
                                                                28.0 * COURIER_UI_SCALE,
                                                                20.0 * COURIER_UI_SCALE,
                                                            ],
                                                            egui::Label::new("EC"),
                                                        );
                                                        ui.add(
                                                            egui::TextEdit::singleline(
                                                                &mut state.timeleap_courier_ec,
                                                            )
                                                            .desired_width(90.0 * COURIER_UI_SCALE),
                                                        );
                                                    });
                                                },
                                            );
                                            },
                                        );
                                    });
                                },
                            );
                            ui.set_style(original_style);
                            } else if state.timeleap_search_tab == TIMELEAP_TAB_SETTINGS {
                                ui.heading(texts.ui_label("設定"));
                                ui.horizontal(|ui| {
                                    ui.label(texts.ui_label("メニュー閉じ時の主人公"));
                                    egui::ComboBox::from_id_salt(
                                        "timeleap-default-player-gender",
                                    )
                                    .selected_text(if state.timeleap_default_is_boy {
                                        texts.ui_label("男の子")
                                    } else {
                                        texts.ui_label("女の子")
                                    })
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut state.timeleap_default_is_boy,
                                            true,
                                            texts.ui_label("男の子"),
                                        );
                                        ui.selectable_value(
                                            &mut state.timeleap_default_is_boy,
                                            false,
                                            texts.ui_label("女の子"),
                                        );
                                    });
                                });
                                ui.weak(texts.ui_label("ポケモン選択でメニュー閉じが自動選択されたときに使用します。"));
                            } else {
                                let actions =
                                    confirmed_target_search_tab_ui_with_worker_running(
                                        ui,
                                        state,
                                        texts,
                                        calculating,
                                        seed_npc_editable,
                                        fastest_close,
                                        confirmed_target_search_worker_running,
                                    );
                                confirmed_target_search_requested |= actions.search_requested;
                                confirmed_target_search_cancel_requested |= actions.cancel_requested;
                                timeleap_frame1_preview_requested |= actions.preview_requested;
                                confirmed_target_search_row = actions.selected_row;
                                confirmed_target_offset_drift_frames =
                                    actions.offset_drift_frames;
                            }
                            };
                            match viewport_class {
                                egui::ViewportClass::Embedded => {
                                    let mut open = true;
                                    egui::Window::new(texts.ui_label("Timeleap検索"))
                                        .open(&mut open)
                                        .default_size(egui::vec2(TIMELEAP_SEARCH_WINDOW_WIDTH, TIMELEAP_SEARCH_WINDOW_HEIGHT))
                                        .min_size(egui::vec2(TIMELEAP_SEARCH_WINDOW_WIDTH, TIMELEAP_SEARCH_WINDOW_HEIGHT))
                                        .max_size(egui::vec2(TIMELEAP_SEARCH_WINDOW_WIDTH, TIMELEAP_SEARCH_WINDOW_HEIGHT))
                                        .show(viewport_ctx, |ui| render_contents(ui));
                                    !open
                                }
                                _ => {
                                    egui::CentralPanel::default()
                                        .show(viewport_ctx, |ui| render_contents(ui));
                                    viewport_ctx.input(|input| input.viewport().close_requested())
                                }
                            }
                        },
                    );
                    state.timeleap_search_window_open = !should_close;
                }
            }
        });
        columns[1].vertical(|ui| {
            if !viewport_only {
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
                    let target_reachability =
                        state.target_reachability_for_display(result.target_exact);
                    if let Some(target_exact) = target_reachability {
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
                    } else if state.timeleap_target_check_is_pending()
                        || state.target_research_pending
                    {
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), target_row_height),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                ui.weak(texts.target_calculating());
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
            if !calculating && state.results.len() == 1 && state.multiple_npc_search {
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
            }
        });
    });
    if viewport_only {
        return FieldSettingsActions {
            target_committed,
            idx_inference_requested,
            idx_inference_cancel_requested,
            open_offset_manager,
            timeleap_frame1_preview_requested,
            timeleap_candidate_search_requested,
            timeleap_candidate_search_cancel_requested,
            timeleap_reachability_search_requested,
            confirmed_target_search_requested,
            confirmed_target_search_cancel_requested,
            confirmed_target_search_row,
            confirmed_target_offset_drift_frames,
            frame1_offset_to_set,
            #[cfg(test)]
            timeleap_controls_layout: TimeleapMainControlsLayout::default(),
            ..FieldSettingsActions::default()
        };
    }
    let before = (
        state.seed.clone(),
        state.npc_count.clone(),
        state.multiple_npc_search,
        state.encounter_offset.clone(),
        state.use_offset,
    );
    let actual_frame_before = state.actual_frame.clone();
    let _separator_before_timeleap = ui.separator();
    #[allow(unused_mut)] // Test builds attach captured row rectangles to this layout.
    let mut timeleap_controls =
        timeleap_main_controls_ui_with_texts(ui, state, fps, panel_width, color_scheme, texts);
    let _separator_before_correction = ui.separator();
    let _correction_row = ui.horizontal(|ui| {
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
    });
    #[cfg(test)]
    {
        timeleap_controls.layout.npc_row = npc_row_rect;
        timeleap_controls.layout.separator_before_timeleap = _separator_before_timeleap.rect;
        timeleap_controls.layout.separator_before_correction = _separator_before_correction.rect;
        timeleap_controls.layout.correction_row = _correction_row.response.rect;
    }
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
    if seed_npc_editable && before.0 != after.0 {
        state.sync_timeleap_source_seed_from_main();
    }
    if seed_npc_editable && before != after {
        state.invalidate_search_results();
        target_committed = state.intervals.len() >= crate::application::state::MIN_BLINK_INTERVALS;
    }
    let target_offset_changed = before.3 != after.3 || before.4 != after.4;
    let timeleap_enabled_changed = timeleap_controls.enabled_changed;
    timeleap_frame1_preview_requested |= timeleap_controls.frame1_preview_requested;
    let timeleap_session_cancel_requested = timeleap_controls.search_cancel_requested;
    #[cfg(test)]
    let timeleap_controls_layout = timeleap_controls.layout;
    FieldSettingsActions {
        target_committed,
        idx_inference_requested,
        idx_inference_cancel_requested,
        target_offset_changed,
        open_offset_manager,
        timeleap_frame1_preview_requested,
        timeleap_enabled_changed,
        timeleap_candidate_search_requested,
        timeleap_session_cancel_requested,
        timeleap_candidate_search_cancel_requested,
        timeleap_reachability_search_requested,
        confirmed_target_search_requested,
        confirmed_target_search_cancel_requested,
        confirmed_target_search_row,
        confirmed_target_offset_drift_frames,
        frame1_offset_to_set,
        #[cfg(test)]
        timeleap_controls_layout,
    }
}

impl Default for FieldSettingsActions {
    fn default() -> Self {
        Self {
            target_committed: false,
            idx_inference_requested: false,
            idx_inference_cancel_requested: false,
            target_offset_changed: false,
            open_offset_manager: false,
            timeleap_frame1_preview_requested: false,
            timeleap_enabled_changed: false,
            timeleap_candidate_search_requested: false,
            timeleap_session_cancel_requested: false,
            timeleap_candidate_search_cancel_requested: false,
            timeleap_reachability_search_requested: false,
            confirmed_target_search_requested: false,
            confirmed_target_search_cancel_requested: false,
            confirmed_target_search_row: None,
            confirmed_target_offset_drift_frames: None,
            frame1_offset_to_set: None,
            #[cfg(test)]
            timeleap_controls_layout: TimeleapMainControlsLayout::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::courier_import_notice_visible;

    #[test]
    fn courier_import_error_stays_visible_until_the_next_panel_interaction() {
        assert!(courier_import_notice_visible(false, false, true));
        assert!(courier_import_notice_visible(true, false, false));
        assert!(!courier_import_notice_visible(true, true, false));
    }

    #[test]
    fn a_new_import_failure_replaces_a_clear_state_with_the_same_notice_state() {
        assert!(courier_import_notice_visible(false, true, true));
    }
}
