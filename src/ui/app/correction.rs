use super::UiApp;
use crate::infrastructure::audio::{fallback_beep, fallback_candidate_not_found, AudioOutput};
use crate::ui::app_types::CorrectionDialog;
use crate::ui::i18n::Texts;
use crate::ui::layout::{MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH};
use eframe::egui;
use std::time::{Duration, Instant};

impl UiApp {
    pub(super) fn start_correction_timeline_calculation(&mut self, ctx: &egui::Context) {
        // 観測間隔は残っているがキャッシュがまだない場合は、補正操作から
        // 観測Timeline生成も開始する。生成中は補正ボタンをスピナーへ置き換え、
        // 完了後に候補照合・本番Timeline計算へ続ける。
        if self.state.candidates.is_empty()
            && self.state.observation_cache.is_none()
            && self.state.intervals.len() >= crate::application::state::MIN_BLINK_INTERVALS
            && !self.jobs.correction_generation.is_running()
        {
            let Ok(config) = self.state.search_config() else {
                return;
            };
            self.jobs.correction_generation_key = Some((
                config.seed,
                config.range_start,
                config.range_end,
                config.fps.to_bits(),
            ));
            let repaint = ctx.clone();
            self.jobs.correction_generation.spawn(move || {
                let cache = crate::domain::rng::generate_observation_cache(
                    config.seed,
                    config.range_start..=config.range_end,
                    config.target,
                    config.fps,
                );
                repaint.request_repaint();
                cache
            });
        }
    }

    /// ずれ検証の入口を一つに集約する。ボタンとEnterのどちらからでも
    /// 同じ条件確認・バックグラウンド計算・ダイアログ表示を行う。
    pub(super) fn request_correction(&mut self, ctx: &egui::Context) {
        let already_in_correction =
            self.state.session_phase() == crate::application::state::SessionPhase::Correction;
        // 最速閉じは実測Frame入力前にReadyからFinishedへ移る。通常タイマーも
        // 終了時にFinishedを使うため、両方の状態から補正を開始できる。
        if (!already_in_correction && !self.state.can_enter_correction())
            || !self.state.correction_inputs_complete()
        {
            return;
        }
        if !already_in_correction {
            self.state.enter_correction_mode();
        }
        if self.state.correction_timeline_needs_calculation() {
            self.correction_calculating = true;
            self.start_correction_timeline_calculation(ctx);
            ctx.request_repaint();
        } else {
            self.open_correction_dialog_if_ready();
        }
    }

    pub(super) fn open_correction_dialog_if_ready(&mut self) {
        // 開始位置は補正欄のドロップダウンで選択する。計算ボタンを押すたびに
        // その時点の選択値を読み込み、古い候補や最初の候補を再利用しない。
        let Some(frame) = self.state.selected_correction_start() else {
            return;
        };
        match self.state.correction_preview_from(frame) {
            Ok(preview) => self.correction_dialog = Some(CorrectionDialog::Confirm { preview }),
            Err(error @ crate::application::state::CorrectionPreviewError::CacheAccess(_)) => {
                self.fail_cache_access(error.to_string());
            }
            Err(_) => {}
        }
    }

    pub(super) fn apply_correction_offset(&mut self, offset: i64) {
        self.state.use_offset = true;
        self.state.encounter_offset = offset.to_string();
        self.state.recalculate_production();
    }

    pub(super) fn apply_correction_rotom(
        &mut self,
        correction: crate::application::state::RotomCorrection,
    ) {
        self.state.rotom_threshold = correction.suggested_threshold.to_string();
        self.state.observed_rotom_talk = Some(correction.observed_talk);
        self.state.recalculate_production();
    }

    pub(super) fn play_beep(&mut self) {
        self.timer_flash_until =
            Some(Instant::now() + Duration::from_millis(self.flash_mode.duration_millis()));
        if !self.beep_enabled {
            return;
        }
        if self.audio.is_none() {
            self.audio = AudioOutput::open();
        }
        if !self.audio.as_ref().is_some_and(AudioOutput::beep) {
            fallback_candidate_not_found();
        }
    }

    pub(super) fn show_correction_dialog(&mut self, ctx: &egui::Context, texts: &Texts) {
        let Some(mut dialog) = self.correction_dialog.take() else {
            return;
        };
        let mut keep = true;
        let title = match &dialog {
            CorrectionDialog::ConfirmFieldCorrection { .. } => texts.correction_confirmation(),
            CorrectionDialog::Confirm { .. } => texts.correction_confirmation(),
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            // 保存されたウィンドウ位置に左右されず、補正ダイアログを画面中央に置く。
            // 最小幅はidxボタン列が狭い画面でも収まる寸法を確保する。
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .min_width(MIN_WINDOW_WIDTH)
            .min_height(MIN_WINDOW_HEIGHT)
            .show(ctx, |ui| match &mut dialog {
                CorrectionDialog::ConfirmFieldCorrection {
                    indices,
                    unknown,
                    suggested_offset,
                } => {
                    ui.label(texts.correction_calculated());
                    let current_offset = if self.field.use_offset {
                        self.field
                            .encounter_offset
                            .trim()
                            .parse::<i64>()
                            .unwrap_or_default()
                    } else {
                        0
                    };
                    if let Some(offset) = suggested_offset {
                        let offset_matches = current_offset == *offset;
                        ui.horizontal(|ui| {
                            ui.label(texts.offset());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if offset_matches {
                                        ui.label("☑");
                                    } else if ui.button(texts.set()).clicked() {
                                        self.field.use_offset = true;
                                        self.field.encounter_offset = offset.to_string();
                                    }
                                    ui.label(format!("{}F", offset));
                                },
                            );
                        });
                    } else {
                        ui.horizontal(|ui| {
                            ui.label(texts.offset());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| ui.label("-"),
                            );
                        });
                    }
                    ui.separator();
                    ui.label(texts.model_number());
                    ui.horizontal(|ui| {
                        if *unknown {
                            ui.label("?");
                        } else {
                            let current_idx = self.field.target_model.trim().parse::<usize>().ok();
                            for idx in indices.iter().copied() {
                                if current_idx == Some(idx) {
                                    ui.add_enabled(false, egui::Button::new(format!("{idx} ☑")));
                                } else if ui.button(texts.idx_set(idx)).clicked() {
                                    self.field.target_model = idx.to_string();
                                    self.field.target_research_pending = true;
                                    self.field.mark_timeline_ready();
                                    self.start_field_search(ctx);
                                    break;
                                }
                            }
                        }
                    });
                    if ui.button(texts.no()).clicked() {
                        keep = false;
                    }
                }
                CorrectionDialog::Confirm { preview } => {
                    ui.label(texts.correction_calculated());

                    let rotom = preview.rotom_correction;
                    let rotom_value = rotom
                        .map(|value| value.suggested_threshold)
                        .unwrap_or_else(|| self.state.rotom_threshold_value());
                    let rotom_matches = rotom.is_none_or(|value| {
                        self.state.rotom_threshold_value() == value.suggested_threshold
                    });
                    ui.horizontal(|ui| {
                        ui.label(texts.rotom_chatter_label());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if rotom_matches {
                                ui.label("☑");
                            } else if let Some(correction) = rotom {
                                if ui.button(texts.set()).clicked() {
                                    self.apply_correction_rotom(correction);
                                }
                            }
                            ui.label(format!("{}%", rotom_value));
                        });
                    });

                    let current_offset = if self.state.use_offset {
                        self.state
                            .encounter_offset
                            .trim()
                            .parse::<i64>()
                            .unwrap_or_default()
                    } else {
                        0
                    };
                    let offset_matches = current_offset == preview.suggested_offset;
                    ui.horizontal(|ui| {
                        ui.label(texts.offset());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if offset_matches {
                                ui.label("☑");
                            } else if ui.button(texts.set()).clicked() {
                                self.apply_correction_offset(preview.suggested_offset);
                            }
                            ui.label(format!("{}F", preview.suggested_offset));
                        });
                    });

                    ui.horizontal(|ui| {
                        ui.label(texts.timeline_reachability());
                        ui.label(format!("(Start:{}F)", preview.timeline_start));
                    });
                    ui.horizontal(|ui| {
                        ui.label(texts.target_actual_reachability(
                            preview.target_on_timeline,
                            preview.actual_on_timeline,
                        ));
                    });

                    if ui.button(texts.no()).clicked() {
                        keep = false;
                    }
                }
            });
        self.correction_dialog = keep.then_some(dialog);
    }

    pub(super) fn show_cache_access_alert(&mut self, ctx: &egui::Context, texts: &Texts) {
        let Some(details) = self.cache_access_alert.clone() else {
            return;
        };
        let mut dismiss = false;
        egui::Window::new(texts.ui_label("キャッシュフォルダにアクセスできません"))
            .id(egui::Id::new("cache_access_alert"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(texts.ui_label("SFMTキャッシュを利用できないため、処理を中断しました。"));
                ui.label(
                    texts
                        .ui_label("キャッシュフォルダへのアクセス権や空き容量を確認してください。"),
                );
                ui.separator();
                ui.label(texts.notice(&details));
                ui.add_space(8.0);
                if ui.button(texts.ui_label("閉じる")).clicked() {
                    dismiss = true;
                }
            });
        if dismiss {
            self.cache_access_alert = None;
        }
    }

    pub(super) fn fail_cache_access(&mut self, details: impl Into<String>) {
        self.cache_access_alert = Some(details.into());
        self.state.cancel_observation();
        self.field.cancel_observation();
        self.jobs.field_timeleap_candidates.cancel_and_discard();
        self.field.cancel_timeleap_candidate_search();
        self.cancel_observation_generation();
        self.cancel_field_background_tasks();
        self.correction_calculating = false;
        self.correction_dialog = None;
        self.field_timer_retry = false;
        self.beeps.article.clear();
        self.beeps.article_start = None;
    }

    pub(super) fn play_candidate_confirmed(&mut self) {
        if !self.beep_enabled || !self.candidate_search_sound {
            return;
        }
        if self.audio.is_none() {
            self.audio = AudioOutput::open();
        }
        if !self
            .audio
            .as_ref()
            .is_some_and(AudioOutput::candidate_confirmed)
        {
            fallback_beep();
        }
    }

    pub(super) fn play_candidate_not_found(&mut self) {
        if !self.beep_enabled || !self.candidate_search_sound {
            return;
        }
        if self.audio.is_none() {
            self.audio = AudioOutput::open();
        }
        if !self
            .audio
            .as_ref()
            .is_some_and(AudioOutput::candidate_not_found)
        {
            fallback_candidate_not_found();
        }
    }
}
