//! 保存対象となる入力設定とセッション項目。
//!
//! このモジュールはディスク上の表現だけを担当する。検索結果、タイマー、ワーカーの
//! ハンドルなど実行中だけ有効な状態は`ui::UiApp`に保持し、このファイルから復元しない。

use crate::application::defaults;
use crate::application::field::{FieldState, FieldTargetMode};
use crate::application::state::{AppState, OffsetStore};
use crate::application::timer_state::NormalTimerState;
use crate::ui::i18n::Language;
use crate::ui::widgets::{ColorScheme, FlashMode};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub(crate) struct PersistedAppState {
    pub(crate) language: Language,
    pub(crate) fps: String,
    pub(crate) color_scheme: ColorScheme,
    pub(crate) beep_enabled: bool,
    pub(crate) keyboard_enabled: bool,
    pub(crate) candidate_search_sound: bool,
    pub(crate) flash_mode: FlashMode,
    pub(crate) wheel_offset_enabled: bool,
    pub(crate) tab: u8,
    pub(crate) hatch: PersistedHatch,
    pub(crate) field: PersistedField,
    pub(crate) normal_timer: PersistedNormalTimer,
    pub(crate) offsets: OffsetStore,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub(crate) struct PersistedHatch {
    pub(crate) seed: String,
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) range_before_target: String,
    pub(crate) target: String,
    pub(crate) actual_target_frame: String,
    pub(crate) npc: String,
    pub(crate) blank_frames: String,
    pub(crate) encounter_offset: String,
    pub(crate) use_offset: bool,
    pub(crate) rotom_threshold: String,
    pub(crate) consider_rotom_talk: bool,
    #[serde(default)]
    pub(crate) consider_pre_rotom_consumption: bool,
    #[serde(default)]
    pub(crate) pre_rotom_consumption: String,
    pub(crate) consider_npc_initial_load: bool,
    pub(crate) npc_initial_load: String,
    pub(crate) fastest_close: bool,
    pub(crate) tolerance: String,
    pub(crate) adjust: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub(crate) struct PersistedField {
    pub(crate) seed: String,
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) range_after_start: String,
    pub(crate) npc_count: String,
    pub(crate) multiple_npc_search: bool,
    pub(crate) target_model: String,
    pub(crate) target_mode_known: bool,
    pub(crate) actual_frame: String,
    pub(crate) tolerance: String,
    pub(crate) target_consumption: String,
    pub(crate) encounter_offset: String,
    pub(crate) use_offset: bool,
    pub(crate) table_follow_current: bool,
    pub(crate) adjust: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub(crate) struct PersistedNormalTimer {
    pub(crate) wait_frames: String,
    pub(crate) offsets: Vec<String>,
    pub(crate) offset_enabled: Vec<bool>,
}

impl Default for PersistedAppState {
    fn default() -> Self {
        Self {
            language: Language::default(),
            fps: defaults::DEFAULT_FPS_TEXT.into(),
            color_scheme: ColorScheme::default(),
            beep_enabled: defaults::DEFAULT_BEEP_ENABLED,
            keyboard_enabled: defaults::DEFAULT_KEYBOARD_ENABLED,
            candidate_search_sound: defaults::DEFAULT_CANDIDATE_SEARCH_SOUND,
            flash_mode: FlashMode::default(),
            wheel_offset_enabled: defaults::DEFAULT_WHEEL_OFFSET_ENABLED,
            tab: defaults::DEFAULT_TAB,
            hatch: PersistedHatch::default(),
            field: PersistedField::default(),
            normal_timer: PersistedNormalTimer::default(),
            offsets: OffsetStore::default(),
        }
    }
}

impl Default for PersistedHatch {
    fn default() -> Self {
        let state = AppState::default();
        Self {
            seed: state.seed,
            range_start: state.range_start,
            range_end: state.range_end,
            range_before_target: state.range_before_target,
            target: state.target,
            actual_target_frame: state.actual_target_frame,
            npc: state.npc,
            blank_frames: state.blank_frames,
            encounter_offset: state.encounter_offset,
            use_offset: state.use_offset,
            rotom_threshold: state.rotom_threshold,
            consider_rotom_talk: state.consider_rotom_talk,
            consider_pre_rotom_consumption: state.consider_pre_rotom_consumption,
            pre_rotom_consumption: state.pre_rotom_consumption,
            consider_npc_initial_load: state.consider_npc_initial_load,
            npc_initial_load: state.npc_initial_load,
            fastest_close: state.fastest_close,
            tolerance: state.tolerance,
            adjust: state.adjust,
        }
    }
}

impl Default for PersistedField {
    fn default() -> Self {
        let state = FieldState::default();
        Self {
            seed: state.seed,
            range_start: state.range_start,
            range_end: state.range_end,
            range_after_start: state.range_after_start,
            npc_count: state.npc_count,
            multiple_npc_search: state.multiple_npc_search,
            target_model: state.target_model,
            target_mode_known: state.target_mode == FieldTargetMode::Known,
            actual_frame: state.actual_frame,
            tolerance: state.tolerance,
            target_consumption: state.target_consumption,
            encounter_offset: state.encounter_offset,
            use_offset: state.use_offset,
            table_follow_current: state.table_follow_current,
            adjust: state.adjust,
        }
    }
}

impl Default for PersistedNormalTimer {
    fn default() -> Self {
        let timer = NormalTimerState::default();
        Self {
            wait_frames: timer.wait_frames,
            offsets: timer.offsets,
            offset_enabled: timer.offset_enabled,
        }
    }
}

impl PersistedAppState {
    pub(crate) fn load() -> Self {
        app_state_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub(crate) fn save(&self) {
        let Some(path) = app_state_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }
}

fn app_state_path() -> Option<std::path::PathBuf> {
    crate::infrastructure::settings_store::app_file("app_state.json")
}

#[cfg(test)]
mod tests {
    use super::PersistedAppState;

    #[test]
    fn persisted_defaults_round_trip_through_json() {
        let original = PersistedAppState::default();
        let encoded = serde_json::to_string(&original).expect("persisted state serializes");
        let decoded: PersistedAppState =
            serde_json::from_str(&encoded).expect("persisted state deserializes");
        assert_eq!(decoded, original);
    }

    #[test]
    fn missing_top_level_fields_use_defaults() {
        let decoded: PersistedAppState = serde_json::from_str("{}").expect("default state");
        assert_eq!(decoded, PersistedAppState::default());
    }
}
