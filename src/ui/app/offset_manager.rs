use super::UiApp;
use crate::application::state::SavedOffset;
use crate::ui::app_types::{OffsetAction, OffsetManagerState, OffsetScope};
use crate::ui::i18n::Texts;
use crate::ui::widgets::frame_spinner;
use eframe::egui;

impl UiApp {
    pub(super) fn open_offset_manager(&mut self, scope: OffsetScope) {
        self.offset_manager = Some(OffsetManagerState {
            scope,
            value: "0".into(),
            memo: String::new(),
            editing: None,
            notice: String::new(),
        });
    }

    pub(super) fn show_offset_manager(&mut self, ctx: &egui::Context, texts: &Texts) {
        let Some(mut manager) = self.offset_manager.take() else {
            return;
        };
        let wheel_offset_enabled = self.wheel_offset_enabled;
        let mut open = true;
        let title = match manager.scope {
            OffsetScope::Hatch => texts.hatch_offset_manager(),
            OffsetScope::Field => texts.field_offset_manager(),
            OffsetScope::Normal => texts.normal_offset_manager(),
        };
        egui::Window::new(title)
            .open(&mut open)
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(texts.offset_frame());
                    frame_spinner(ui, &mut manager.value, 100.0, 2, wheel_offset_enabled);
                });
                ui.horizontal(|ui| {
                    ui.label(texts.memo());
                    ui.add(egui::TextEdit::singleline(&mut manager.memo).desired_width(260.0));
                    let editing = manager.editing.is_some();
                    if ui
                        .button(if editing { texts.update() } else { texts.add() })
                        .clicked()
                    {
                        match manager.value.trim().parse::<i64>() {
                            Ok(value) => {
                                let entry = SavedOffset {
                                    value,
                                    memo: manager.memo.trim().to_string(),
                                };
                                let entries = match manager.scope {
                                    OffsetScope::Hatch => &mut self.state.offset_store.hatch,
                                    OffsetScope::Field => &mut self.state.offset_store.field,
                                    OffsetScope::Normal => &mut self.state.offset_store.normal,
                                };
                                if let Some(index) = manager.editing {
                                    if let Some(existing) = entries.get_mut(index) {
                                        *existing = entry;
                                    }
                                } else {
                                    entries.push(entry);
                                }
                                match self.state.offset_store.save() {
                                    Ok(()) => {
                                        manager.notice.clear();
                                        manager.value = "0".into();
                                        manager.memo.clear();
                                        manager.editing = None;
                                    }
                                    Err(error) => manager.notice = texts.app_error(&error),
                                }
                            }
                            Err(_) => {
                                manager.notice = "オフセットFrameは整数で入力してください。".into()
                            }
                        }
                    }
                    if editing && ui.button(texts.stop_editing()).clicked() {
                        manager.editing = None;
                        manager.value = "0".into();
                        manager.memo.clear();
                    }
                });
                ui.separator();

                let entries = match manager.scope {
                    OffsetScope::Hatch => self.state.offset_store.hatch.clone(),
                    OffsetScope::Field => self.state.offset_store.field.clone(),
                    OffsetScope::Normal => self.state.offset_store.normal.clone(),
                };
                let mut action = None;
                for (index, entry) in entries.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add_sized([70.0, 20.0], egui::Label::new(format!("{}F", entry.value)));
                        ui.add_sized(
                            [260.0, 20.0],
                            egui::Label::new(if entry.memo.is_empty() {
                                texts.no_memo()
                            } else {
                                entry.memo.as_str()
                            }),
                        );
                        if ui.button(texts.apply()).clicked() {
                            action = Some(OffsetAction::Apply(entry.value));
                        }
                        if ui.button(texts.edit()).clicked() {
                            action = Some(OffsetAction::Edit(index));
                        }
                        if ui.button(texts.delete()).clicked() {
                            action = Some(OffsetAction::Delete(index));
                        }
                    });
                }

                if let Some(action) = action {
                    match action {
                        OffsetAction::Apply(value) => match manager.scope {
                            OffsetScope::Hatch => {
                                if self.state.is_timer_phase() || self.state.is_finished() {
                                    manager.notice = "本番中はオフセットを反映できません。".into();
                                } else {
                                    self.state.use_offset = true;
                                    self.state.encounter_offset = value.to_string();
                                    self.state.recalculate_production();
                                    manager.notice.clear();
                                }
                            }
                            OffsetScope::Field => {
                                if self.field.timer_is_running() {
                                    manager.notice = "タイマー停止中のみ反映できます。".into();
                                } else {
                                    self.field.use_offset = true;
                                    self.field.encounter_offset = value.to_string();
                                    self.field
                                        .refresh_target_timer_for_offset(self.state.fps_value());
                                    manager.notice.clear();
                                }
                            }
                            OffsetScope::Normal => {
                                if self.state.normal_timer.is_running() {
                                    manager.notice = "タイマー停止中のみ反映できます。".into();
                                } else {
                                    self.state.normal_timer.offsets.push(value.to_string());
                                    self.state.normal_timer.offset_enabled.push(true);
                                    manager.notice.clear();
                                }
                            }
                        },
                        OffsetAction::Edit(index) => {
                            if let Some(entry) = entries.get(index) {
                                manager.value = entry.value.to_string();
                                manager.memo = entry.memo.clone();
                                manager.editing = Some(index);
                            }
                        }
                        OffsetAction::Delete(index) => {
                            let entries = match manager.scope {
                                OffsetScope::Hatch => &mut self.state.offset_store.hatch,
                                OffsetScope::Field => &mut self.state.offset_store.field,
                                OffsetScope::Normal => &mut self.state.offset_store.normal,
                            };
                            if index < entries.len() {
                                entries.remove(index);
                                manager.editing = None;
                                match self.state.offset_store.save() {
                                    Ok(()) => manager.notice.clear(),
                                    Err(error) => manager.notice = texts.app_error(&error),
                                }
                            }
                        }
                    }
                }
                if !manager.notice.is_empty() {
                    let notice = texts.notice(&manager.notice);
                    ui.colored_label(self.color_scheme.colors_for_ui(ui).error, notice);
                }
            });
        if open {
            self.offset_manager = Some(manager);
        }
    }
}
