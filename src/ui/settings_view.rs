//! 設定画面の描画と入力受付を担当する。
//!
//! 値の永続化と適用はアプリケーション状態へ返し、画面側では入力欄と
//! コントロールの配置だけを扱う。

use crate::application::state::AppState;
use crate::ui::i18n::{Language, Texts};
use crate::ui::widgets::{
    apply_color_scheme, consumption_spinner, numeric_text_edit, ColorScheme, FlashMode,
};
use eframe::egui;

const SETTING_INPUT_WIDTH: f32 = 120.0;
const PRE_ROTOM_CONTROL_WIDTH: f32 = 310.0;
const LANGUAGE_JAPANESE: &str = "日本語";
const LANGUAGE_ENGLISH: &str = "English";

/// 設定タブの描画と入力を担当する。保存やタイマー制御は呼び出し側へ返す。
#[allow(clippy::too_many_arguments)]
pub(crate) fn settings_ui(
    ui: &mut egui::Ui,
    state: &mut AppState,
    language: &mut Language,
    color_scheme: &mut ColorScheme,
    beep_enabled: &mut bool,
    keyboard_enabled: &mut bool,
    candidate_search_sound: &mut bool,
    flash_mode: &mut FlashMode,
    wheel_offset_enabled: &mut bool,
    ctx: &egui::Context,
    texts: &Texts,
) -> bool {
    let before = (
        *language,
        state.fps.clone(),
        state.consider_pre_rotom_consumption,
        state.pre_rotom_consumption.clone(),
        *color_scheme,
        *beep_enabled,
        *keyboard_enabled,
        *candidate_search_sound,
        *flash_mode,
        *wheel_offset_enabled,
    );
    ui.heading(texts.settings());
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.label(texts.fps());
            numeric_text_edit(ui, &mut state.fps, SETTING_INPUT_WIDTH, true, false);
            // FPS入力と重ならないように、右端の設定領域を固定幅で確保する。
            let available = ui.available_width();
            let control_width = PRE_ROTOM_CONTROL_WIDTH.min(available.max(0.0));
            ui.add_space((available - control_width).max(0.0));
            ui.allocate_ui_with_layout(
                egui::vec2(control_width, ui.spacing().interact_size.y),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    ui.add_enabled_ui(state.consider_pre_rotom_consumption, |ui| {
                        consumption_spinner(
                            ui,
                            &mut state.pre_rotom_consumption,
                            SETTING_INPUT_WIDTH,
                            *wheel_offset_enabled,
                        );
                    });
                    ui.checkbox(
                        &mut state.consider_pre_rotom_consumption,
                        texts.pre_rotom_consumption(),
                    );
                },
            );
        });
        ui.horizontal(|ui| {
            ui.label(texts.beep_interval());
            numeric_text_edit(
                ui,
                &mut state.beep_interval,
                SETTING_INPUT_WIDTH,
                true,
                false,
            );
        });
        ui.horizontal(|ui| {
            ui.label(texts.beep_count());
            numeric_text_edit(ui, &mut state.beep_count, SETTING_INPUT_WIDTH, false, false);
        });
    });
    ui.checkbox(beep_enabled, texts.beep_enabled());
    ui.checkbox(keyboard_enabled, texts.keyboard_enabled());
    ui.checkbox(candidate_search_sound, texts.candidate_search_sound());
    ui.horizontal(|ui| {
        ui.label(texts.flash_mode());
        egui::ComboBox::from_id_salt("flash-mode")
            .selected_text(flash_mode.label(texts.language))
            .show_ui(ui, |ui| {
                for mode in [FlashMode::Short, FlashMode::Long, FlashMode::Static] {
                    ui.selectable_value(flash_mode, mode, mode.label(texts.language));
                }
            });
    });
    ui.checkbox(wheel_offset_enabled, texts.wheel_offset());
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(texts.language());
        egui::ComboBox::from_id_salt("language")
            .selected_text(Texts::new(*language).language_name())
            .show_ui(ui, |ui| {
                ui.selectable_value(language, Language::Japanese, LANGUAGE_JAPANESE);
                ui.selectable_value(language, Language::English, LANGUAGE_ENGLISH);
            });
    });
    let mut palette_changed = false;
    ui.horizontal(|ui| {
        ui.label(texts.color_scheme());
        egui::ComboBox::from_id_salt("color-scheme")
            .selected_text(color_scheme.label(texts.language))
            .show_ui(ui, |ui| {
                for scheme in [
                    ColorScheme::Standard,
                    ColorScheme::HighContrast,
                    ColorScheme::Monochrome,
                ] {
                    palette_changed |= ui
                        .selectable_value(color_scheme, scheme, scheme.label(texts.language))
                        .changed();
                }
            });
    });
    if palette_changed {
        apply_color_scheme(ctx, *color_scheme);
    }
    before
        != (
            *language,
            state.fps.clone(),
            state.consider_pre_rotom_consumption,
            state.pre_rotom_consumption.clone(),
            *color_scheme,
            *beep_enabled,
            *keyboard_enabled,
            *candidate_search_sound,
            *flash_mode,
            *wheel_offset_enabled,
        )
}
