//! 保存対象となる入力設定とセッション項目。
//!
//! このモジュールはディスク上の表現だけを担当する。検索結果、タイマー、ワーカーの
//! ハンドルなど実行中だけ有効な状態は`ui::UiApp`に保持し、このファイルから復元しない。

use crate::application::defaults;
use crate::application::field::{FieldState, FieldTargetMode};
use crate::application::state::{AppState, OffsetStore};
use crate::application::timer_state::NormalTimerState;
use crate::domain::fixed_rng_catalog::GameEdition;
use crate::domain::timeleap::TimeleapMode;
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
    pub(crate) initial_branch_scan_span: String,
    pub(crate) timeleap: PersistedTimeleap,
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
pub(crate) struct PersistedTimeleap {
    pub(crate) enabled: bool,
    pub(crate) source: PersistedTimeleapSource,
    pub(crate) frame1_search: PersistedTimeleapFrame1Search,
    pub(crate) target_search: PersistedTimeleapTargetSearch,
    pub(crate) courier: PersistedTimeleapCourier,
    pub(crate) options: PersistedTimeleapOptions,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub(crate) struct PersistedTimeleapSource {
    pub(crate) inputs_initialized: bool,
    pub(crate) starting_frame: String,
    pub(crate) npc_count: String,
    pub(crate) frame1: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub(crate) struct PersistedTimeleapFrame1Search {
    pub(crate) frame1_input: String,
    pub(crate) search_range: String,
    pub(crate) seconds_min: String,
    pub(crate) seconds_max: String,
    pub(crate) frame1_min: Option<String>,
    pub(crate) frame1_max: Option<String>,
    pub(crate) filter_mode: u8,
    pub(crate) grace_min: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub(crate) struct PersistedTimeleapTargetSearch {
    pub(crate) grace_min: String,
    pub(crate) generation_start_frame: String,
    pub(crate) drift_search_enabled: bool,
    pub(crate) drift_search_generation_start_frame: String,
    pub(crate) drift_range_ticks: String,
    pub(crate) confirmed_grace_min_frames: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub(crate) struct PersistedTimeleapOptions {
    pub(crate) game_edition: GameEdition,
    #[serde(with = "timeleap_mode_code")]
    pub(crate) mode: TimeleapMode,
    pub(crate) is_boy: bool,
    pub(crate) default_is_boy: bool,
    pub(crate) no_dex: bool,
    pub(crate) direct_encounter: bool,
    pub(crate) consider_delay_enabled: bool,
    pub(crate) consider_delay_frames: String,
    pub(crate) search_category: usize,
    pub(crate) search_preset: usize,
    pub(crate) search_tab: u8,
}

mod timeleap_mode_code {
    use crate::domain::timeleap::TimeleapMode;
    use serde::{de::Error, Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S>(mode: &TimeleapMode, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u8(mode.code())
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<TimeleapMode, D::Error>
    where
        D: Deserializer<'de>,
    {
        let code = u8::deserialize(deserializer)?;
        TimeleapMode::try_from(code).map_err(D::Error::custom)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub(crate) struct PersistedTimeleapCourier {
    pub(crate) iv_count: String,
    pub(crate) ability_locked: bool,
    pub(crate) nature_locked: bool,
    pub(crate) gender_locked: bool,
    pub(crate) ec: String,
    pub(crate) pid_type: u8,
    pub(crate) pid: String,
    pub(crate) fixed_ivs: [i8; 6],
    pub(crate) ability: u8,
    pub(crate) nature: u8,
    pub(crate) gender_ratio: u8,
    pub(crate) is_egg: bool,
    pub(crate) your_id: bool,
    pub(crate) other_info: bool,
    pub(crate) tid: String,
    pub(crate) sid: String,
    pub(crate) species_id: u16,
}

fn default_timeleap_search_range() -> String {
    defaults::FIELD_TIMELEAP_SEARCH_RANGE.into()
}
fn default_timeleap_seconds_min() -> String {
    defaults::FIELD_TIMELEAP_SECONDS_MIN.into()
}
fn default_timeleap_seconds_max() -> String {
    defaults::FIELD_TIMELEAP_SECONDS_MAX.into()
}
fn default_timeleap_grace_min() -> String {
    defaults::FIELD_TIMELEAP_GRACE_MIN.into()
}
fn default_timeleap_target_grace_min() -> String {
    "0".into()
}
fn default_timeleap_generation_start_frame() -> String {
    "10000".into()
}
fn default_timeleap_drift_range_ticks() -> String {
    defaults::FIELD_TIMELEAP_DRIFT_RANGE_TICKS.into()
}
fn default_confirmed_target_grace_min_frames() -> String {
    "0".into()
}
fn default_timeleap_courier_iv_count() -> String {
    "0".into()
}
fn default_timeleap_courier_fixed_ivs() -> [i8; 6] {
    [-1; 6]
}
fn default_timeleap_courier_gender_ratio() -> u8 {
    127
}
fn default_true() -> bool {
    true
}

impl Default for PersistedTimeleap {
    fn default() -> Self {
        Self {
            enabled: false,
            source: PersistedTimeleapSource::default(),
            frame1_search: PersistedTimeleapFrame1Search::default(),
            target_search: PersistedTimeleapTargetSearch::default(),
            courier: PersistedTimeleapCourier::default(),
            options: PersistedTimeleapOptions::default(),
        }
    }
}

impl Default for PersistedTimeleapSource {
    fn default() -> Self {
        Self {
            inputs_initialized: false,
            starting_frame: String::new(),
            npc_count: String::new(),
            frame1: String::new(),
        }
    }
}

impl Default for PersistedTimeleapFrame1Search {
    fn default() -> Self {
        let state = FieldState::default();
        Self {
            frame1_input: state.timeleap_frame1_input,
            search_range: default_timeleap_search_range(),
            seconds_min: default_timeleap_seconds_min(),
            seconds_max: default_timeleap_seconds_max(),
            frame1_min: Some(state.timeleap_frame1_min),
            frame1_max: Some(state.timeleap_frame1_max),
            filter_mode: 0,
            grace_min: default_timeleap_grace_min(),
        }
    }
}

impl Default for PersistedTimeleapTargetSearch {
    fn default() -> Self {
        Self {
            grace_min: default_timeleap_target_grace_min(),
            generation_start_frame: default_timeleap_generation_start_frame(),
            drift_search_enabled: false,
            drift_search_generation_start_frame: String::new(),
            drift_range_ticks: default_timeleap_drift_range_ticks(),
            confirmed_grace_min_frames: default_confirmed_target_grace_min_frames(),
        }
    }
}

impl Default for PersistedTimeleapOptions {
    fn default() -> Self {
        Self {
            game_edition: GameEdition::default(),
            mode: TimeleapMode::Menu,
            is_boy: default_true(),
            default_is_boy: default_true(),
            no_dex: false,
            direct_encounter: false,
            consider_delay_enabled: default_true(),
            consider_delay_frames: "0".into(),
            search_category: 0,
            search_preset: 0,
            search_tab: 0,
        }
    }
}

impl Default for PersistedTimeleapCourier {
    fn default() -> Self {
        Self {
            iv_count: default_timeleap_courier_iv_count(),
            ability_locked: false,
            nature_locked: false,
            gender_locked: false,
            ec: String::new(),
            pid_type: 0,
            pid: String::new(),
            fixed_ivs: default_timeleap_courier_fixed_ivs(),
            ability: 0,
            nature: 0,
            gender_ratio: default_timeleap_courier_gender_ratio(),
            is_egg: false,
            your_id: false,
            other_info: default_true(),
            tid: "0".into(),
            sid: "0".into(),
            species_id: 0,
        }
    }
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
            initial_branch_scan_span: state.initial_branch_scan_span,
            timeleap: PersistedTimeleap::default(),
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
    use crate::application::defaults;
    use crate::domain::fixed_rng_catalog::GameEdition;

    use super::PersistedAppState;

    fn timeleap_settings_from_field(field: serde_json::Value) -> super::PersistedTimeleap {
        let state: PersistedAppState = serde_json::from_value(serde_json::json!({
            "field": field
        }))
        .expect("settings fixture deserializes");
        state.field.timeleap
    }

    #[test]
    fn persisted_defaults_round_trip_through_json() {
        let mut original = PersistedAppState::default();
        let timeleap = &mut original.field.timeleap;
        timeleap.enabled = true;
        timeleap.source.inputs_initialized = true;
        timeleap.source.starting_frame = "490".into();
        timeleap.source.npc_count = "15".into();
        timeleap.source.frame1 = "481".into();
        timeleap.options.no_dex = true;
        timeleap.options.direct_encounter = true;
        timeleap.options.consider_delay_enabled = false;
        timeleap.options.consider_delay_frames = "156".into();
        timeleap.options.search_category = 4;
        timeleap.options.search_preset = 2;
        timeleap.options.game_edition = GameEdition::SunMoon;
        timeleap.options.search_tab = 1;
        timeleap.options.default_is_boy = false;
        timeleap.frame1_search.seconds_min = "35".into();
        timeleap.frame1_search.seconds_max = "75".into();
        timeleap.frame1_search.frame1_min = Some("1234".into());
        timeleap.frame1_search.frame1_max = Some("5678".into());
        timeleap.frame1_search.filter_mode = 1;
        timeleap.frame1_search.grace_min = "8".into();
        timeleap.target_search.grace_min = "6".into();
        timeleap.target_search.generation_start_frame = "123456".into();
        timeleap.target_search.drift_search_enabled = true;
        timeleap.target_search.drift_search_generation_start_frame = "123400".into();
        timeleap.target_search.drift_range_ticks = "37".into();
        timeleap.target_search.confirmed_grace_min_frames = "9".into();
        timeleap.courier.your_id = true;
        let encoded = serde_json::to_string(&original).expect("persisted state serializes");
        let decoded: PersistedAppState =
            serde_json::from_str(&encoded).expect("persisted state deserializes");
        assert_eq!(decoded, original);
        assert_eq!(
            decoded.field.timeleap.frame1_search.frame1_input,
            defaults::FIELD_TIMELEAP_FRAME1
        );
        assert!(decoded.field.timeleap.source.inputs_initialized);
        assert_eq!(decoded.field.timeleap.source.starting_frame, "490");
        assert_eq!(decoded.field.timeleap.source.npc_count, "15");
        assert_eq!(decoded.field.timeleap.source.frame1, "481");
        assert!(decoded.field.timeleap.options.direct_encounter);
        assert!(decoded.field.timeleap.target_search.drift_search_enabled);
        assert_eq!(
            decoded
                .field
                .timeleap
                .target_search
                .drift_search_generation_start_frame,
            "123400"
        );
        assert_eq!(decoded.field.timeleap.target_search.drift_range_ticks, "37");
        assert_eq!(
            decoded
                .field
                .timeleap
                .target_search
                .confirmed_grace_min_frames,
            "9"
        );
        assert!(decoded.field.timeleap.courier.your_id);
        assert_eq!(
            decoded.field.timeleap.options.game_edition,
            GameEdition::SunMoon
        );

        let json: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        let field = json["field"].as_object().unwrap();
        assert!(field.contains_key("timeleap"));
        assert!(!field.contains_key("timeleap_enabled"));
        assert!(field.contains_key("seed"));
        assert_eq!(
            json["field"]["timeleap"]["options"]["game_edition"],
            "sun_moon"
        );
        assert!(json["field"]["timeleap"]
            .as_object()
            .unwrap()
            .get("game_edition")
            .is_none());
    }

    #[test]
    fn missing_top_level_fields_use_defaults() {
        let decoded: PersistedAppState = serde_json::from_str("{}").expect("default state");
        assert_eq!(decoded, PersistedAppState::default());
    }

    #[test]
    fn old_nested_timeleap_settings_default_to_usum() {
        let nested = timeleap_settings_from_field(serde_json::json!({
            "timeleap": {"options": {"search_category": 1, "search_preset": 0}}
        }));
        assert_eq!(nested.options.game_edition, GameEdition::UltraSunUltraMoon);
        assert!(!nested.options.direct_encounter);
        assert_eq!(nested.options.search_category, 1);
    }

    #[test]
    fn nested_modes_keep_their_canonical_values() {
        use crate::domain::timeleap::TimeleapMode;

        for mode in [
            TimeleapMode::Menu,
            TimeleapMode::Dialogue,
            TimeleapMode::Courier,
        ] {
            for category in [0, 2] {
                let nested = timeleap_settings_from_field(serde_json::json!({
                    "timeleap": {"options": {
                        "mode": mode.code(),
                        "search_category": category,
                        "search_preset": 1
                    }}
                }));
                assert_eq!(nested.options.mode, mode);
                assert_eq!(nested.options.search_category, category);
                assert_eq!(nested.options.search_preset, 1);
            }
        }
    }

    #[test]
    fn v021_field_settings_without_timeleap_keep_defaults() {
        let timeleap = timeleap_settings_from_field(serde_json::json!({
            "seed": "B248EA3B",
            "range_start": "490",
            "npc_count": "15"
        }));
        assert_eq!(timeleap, super::PersistedTimeleap::default());
    }

    #[test]
    fn missing_branch_scan_setting_uses_the_default() {
        let decoded: PersistedAppState =
            serde_json::from_str(r#"{"field":{}}"#).expect("field defaults");
        assert_eq!(
            decoded.field.initial_branch_scan_span,
            defaults::FIELD_INITIAL_BRANCH_SCAN_SPAN
        );
        let timeleap = &decoded.field.timeleap;
        assert_eq!(
            timeleap.frame1_search.frame1_input,
            defaults::FIELD_TIMELEAP_FRAME1
        );
        assert!(!timeleap.source.inputs_initialized);
        assert!(timeleap.source.starting_frame.is_empty());
        assert!(timeleap.source.npc_count.is_empty());
        assert!(timeleap.source.frame1.is_empty());
        assert!(!timeleap.options.no_dex);
        assert!(!timeleap.options.direct_encounter);
        assert!(timeleap.options.consider_delay_enabled);
        assert_eq!(timeleap.options.consider_delay_frames, "0");
        assert_eq!(timeleap.options.search_category, 0);
        assert_eq!(timeleap.options.search_preset, 0);
        assert_eq!(timeleap.options.search_tab, 0);
        assert!(timeleap.options.default_is_boy);
        assert_eq!(timeleap.target_search.grace_min, "0");
        assert_eq!(timeleap.target_search.generation_start_frame, "10000");
        assert!(!timeleap.target_search.drift_search_enabled);
        assert!(timeleap
            .target_search
            .drift_search_generation_start_frame
            .is_empty());
        assert_eq!(
            timeleap.target_search.drift_range_ticks,
            defaults::FIELD_TIMELEAP_DRIFT_RANGE_TICKS
        );
        assert_eq!(timeleap.target_search.confirmed_grace_min_frames, "0");
        assert_eq!(timeleap.courier.tid, "0");
        assert_eq!(timeleap.courier.sid, "0");
    }

    #[test]
    fn timeleap_default_gender_is_persisted_and_removed_auto_setting_is_ignored() {
        let mut original = PersistedAppState::default();
        assert!(original.field.timeleap.options.default_is_boy);
        original.field.timeleap.options.default_is_boy = false;

        let encoded = serde_json::to_string(&original).expect("persisted state serializes");
        let decoded: PersistedAppState =
            serde_json::from_str(&encoded).expect("persisted state deserializes");
        assert!(!decoded.field.timeleap.options.default_is_boy);

        let without_preference: PersistedAppState =
            serde_json::from_str(r#"{"field":{}}"#).expect("missing field uses default");
        assert!(without_preference.field.timeleap.options.default_is_boy);

        let old_nested: PersistedAppState = serde_json::from_str(
            r#"{"field":{"timeleap":{"options":{"auto_frame1_percent":73}}}}"#,
        )
        .expect("old nested Auto setting is ignored");
        assert!(old_nested.field.timeleap.options.default_is_boy);
        let new_json = serde_json::to_value(old_nested).expect("new settings serialize");
        assert!(new_json["field"]["timeleap"]["options"]
            .get("auto_frame1_percent")
            .is_none());

        let old_flat: PersistedAppState =
            serde_json::from_str(r#"{"field":{"timeleap_auto_frame1_percent":81}}"#)
                .expect("old flat Auto setting is ignored");
        let migrated_json = serde_json::to_value(old_flat).expect("migrated settings serialize");
        assert!(migrated_json["field"]["timeleap"]["options"]
            .get("auto_frame1_percent")
            .is_none());
    }

    #[test]
    fn obsolete_flat_timeleap_keys_are_not_migrated() {
        let decoded: PersistedAppState = serde_json::from_str(
            r#"{
            "field": {
                "seed": "b248ea3b",
                "npc_count": "15",
                "timeleap_enabled": true,
                "timeleap_generation_start_frame": "70000"
            }
        }"#,
        )
        .unwrap();
        assert_eq!(decoded.field.seed, "b248ea3b");
        assert_eq!(decoded.field.npc_count, "15");
        assert_eq!(decoded.field.timeleap, super::PersistedTimeleap::default());
        let saved = serde_json::to_value(decoded).unwrap();
        assert!(saved["field"].get("timeleap_enabled").is_none());
    }

    #[test]
    fn v021_field_settings_load_with_timeleap_defaults() {
        let v021 = r#"{
            "language": "English",
            "fps": "59.94",
            "beep_enabled": false,
            "hatch": { "seed": "76543210", "npc": "10", "adjust": 4 },
            "field": {
                "seed": "01234567",
                "range_start": "10",
                "range_end": "20",
                "range_after_start": "30",
                "npc_count": "15",
                "multiple_npc_search": false,
                "initial_branch_scan_span": "42",
                "target_model": "2",
                "target_mode_known": true,
                "actual_frame": "100",
                "tolerance": "3",
                "target_consumption": "4",
                "encounter_offset": "0",
                "use_offset": false,
                "table_follow_current": true,
                "adjust": 0
            }
        }"#;
        let decoded: PersistedAppState = serde_json::from_str(v021).unwrap();
        assert_eq!(decoded.field.seed, "01234567");
        assert_eq!(decoded.field.range_start, "10");
        assert_eq!(decoded.field.range_end, "20");
        assert_eq!(
            decoded.field.timeleap.frame1_search.frame1_input,
            defaults::FIELD_TIMELEAP_FRAME1
        );
        assert_eq!(
            decoded.field.timeleap.target_search.generation_start_frame,
            "10000"
        );
        assert!(decoded.field.timeleap.options.consider_delay_enabled);
        let original: serde_json::Value = serde_json::from_str(v021).unwrap();
        let saved = serde_json::to_value(&decoded).unwrap();
        for (key, value) in original["field"].as_object().unwrap() {
            assert_eq!(&saved["field"][key], value, "v0.2.1 field: {key}");
        }
        assert_eq!(saved["language"], original["language"]);
        assert_eq!(saved["fps"], original["fps"]);
        assert_eq!(saved["beep_enabled"], original["beep_enabled"]);
        for (key, value) in original["hatch"].as_object().unwrap() {
            assert_eq!(&saved["hatch"][key], value, "v0.2.1 hatch: {key}");
        }
        let restored: PersistedAppState = serde_json::from_value(saved).unwrap();
        assert_eq!(restored, decoded);
    }
}
