//! 実行状態と保存設定で共有する初期値。
//!
//! UIと保存層が編集可能な文字列を扱うため、値は文字列として保持する。解析と検証は
//! 各入力欄を所有するアプリケーション状態が担当する。

pub(crate) const SEED: &str = "00000001";

pub(crate) const HATCH_RANGE_START: &str = "0";
pub(crate) const HATCH_RANGE_END: &str = "1000";
pub(crate) const HATCH_RANGE_BEFORE_TARGET: &str = "10000";
pub(crate) const HATCH_TARGET: &str = "1000";
pub(crate) const HATCH_NPC: &str = "0";
pub(crate) const HATCH_BLANK_FRAMES: &str = "120";
pub(crate) const HATCH_ENCOUNTER_OFFSET: &str = "0";
pub(crate) const HATCH_ROTOM_THRESHOLD: &str = "79";
pub(crate) const HATCH_NPC_INITIAL_LOAD: &str = "1";
pub(crate) const HATCH_PRE_ROTOM_CONSUMPTION: &str = "0";
pub(crate) const HATCH_TOLERANCE: &str = "5";

pub(crate) const FIELD_RANGE_START: &str = "0";
pub(crate) const FIELD_RANGE_END: &str = "10000";
pub(crate) const FIELD_RANGE_AFTER_START: &str = "10000";
pub(crate) const FIELD_NPC_COUNT: &str = "1";
pub(crate) const FIELD_TARGET_MODEL: &str = "0";
pub(crate) const FIELD_ENCOUNTER_OFFSET: &str = "0";
pub(crate) const FIELD_TOLERANCE: &str = "5";

pub(crate) const DEFAULT_WAIT_FRAMES: &str = "0";
pub(crate) const DEFAULT_FPS_TEXT: &str = "59.8621";
pub(crate) const DEFAULT_BEEP_INTERVAL: &str = "0.5";
pub(crate) const DEFAULT_BEEP_COUNT: &str = "6";
pub(crate) const DEFAULT_ADJUST: i64 = 0;

pub(crate) const DEFAULT_CONSIDER_ROTOM_TALK: bool = true;
pub(crate) const DEFAULT_CONSIDER_NPC_INITIAL_LOAD: bool = true;
pub(crate) const DEFAULT_FASTEST_CLOSE: bool = false;
pub(crate) const DEFAULT_USE_OFFSET: bool = false;
pub(crate) const DEFAULT_TABLE_FOLLOW_CURRENT: bool = true;
pub(crate) const DEFAULT_BEEP_ENABLED: bool = true;
pub(crate) const DEFAULT_KEYBOARD_ENABLED: bool = true;
pub(crate) const DEFAULT_CANDIDATE_SEARCH_SOUND: bool = true;
pub(crate) const DEFAULT_WHEEL_OFFSET_ENABLED: bool = false;
pub(crate) const DEFAULT_TAB: u8 = 0;

pub(crate) const INITIAL_APP_STATUS: &str =
    "初期状態です。条件を入力してタイムラインを生成してください。";
pub(crate) const INITIAL_FIELD_STATUS: &str = "条件を入力して観測を開始してください。";
pub(crate) const INITIAL_TIMER_STATUS: &str = "待機Frameと必要なオフセットを入力してください。";
pub(crate) const CORRECTION_TIMER_LOCKED: &str =
    "ずれ検証中はタイマーを再開できません。リセットしてください。";
