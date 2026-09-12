//! ユーザーが再利用するオフセット値の永続化を担当する。
//!
//! 保存先とシリアライズ形式を管理し、補正値の解釈や丸めは呼び出し側へ委ねる。

use crate::infrastructure::settings_store::app_file;
use serde::{Deserialize, Serialize};

/// ユーザーが再利用するオフセット1件分。
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SavedOffset {
    pub value: i64,
    pub memo: String,
}

/// 孵化・通常タイマーで共有する保存済みオフセット。
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct OffsetStore {
    /// 孵化時瞬きタブ用。B終了後〜エンカウントのFrameオフセット。
    pub hatch: Vec<SavedOffset>,
    /// フィールドタブ用。Targetタイマーへ加えるFrameオフセット。
    #[serde(default)]
    pub field: Vec<SavedOffset>,
    /// 通常タイマー用。待機Frameへ加えるオフセット。
    pub normal: Vec<SavedOffset>,
}

impl OffsetStore {
    pub fn load() -> Self {
        let Some(path) = app_file("offsets.json") else {
            return Self::default();
        };
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = app_file("offsets.json")
            .ok_or_else(|| "オフセット保存先を決められません。".to_string())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|_| "オフセット保存用フォルダを作成できません。".to_string())?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|_| "オフセットを保存用JSONへ変換できません。".to_string())?;
        std::fs::write(path, text).map_err(|_| "オフセットを保存できません。".to_string())
    }
}
