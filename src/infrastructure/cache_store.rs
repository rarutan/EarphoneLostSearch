//! SFMTキャッシュのファイル配置と読み書きを担当する。
//!
//! 乱数列や検索条件の意味はドメイン層が管理し、このモジュールではファイル形式の
//! 検証、一時ファイル、保存先の解決だけを行う。

use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const CACHE_MAGIC: &[u8; 8] = b"HBSFMT\0\0";

/// SFMT出力キャッシュの保存先と形式検証を担当するI/Oアダプター。
///
/// SFMT列の生成やTimeline計算は行わず、ファイルの場所・一時ファイル・ヘッダーだけを
/// 管理する。キャッシュが削除されても、呼び出し側が再生成できる前提を維持する。
pub(crate) struct CacheStore {
    directory: PathBuf,
}

impl Default for CacheStore {
    fn default() -> Self {
        Self {
            directory: std::env::temp_dir().join("hatch-blink-search-cache"),
        }
    }
}

impl CacheStore {
    pub(crate) fn ensure_directory(&self) -> Result<(), String> {
        fs::create_dir_all(&self.directory)
            .map_err(|error| format!("SFMTキャッシュ用フォルダーを作れません: {error}"))
    }

    pub(crate) fn path(&self, seed: u32, last_position: i64) -> PathBuf {
        self.directory
            .join(format!("sfmt_{seed:08X}_{last_position}.bin"))
    }

    pub(crate) fn temporary_path(&self, seed: u32, last_position: i64) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        self.directory.join(format!(
            "sfmt_{seed:08X}_{last_position}_{stamp}_{}.tmp",
            std::process::id()
        ))
    }

    pub(crate) fn is_valid(
        &self,
        path: &Path,
        seed: u32,
        value_count: u64,
        version: u32,
        header_size: u64,
    ) -> Result<bool, String> {
        let Ok(metadata) = fs::metadata(path) else {
            return Ok(false);
        };
        if metadata.len() != header_size + value_count.saturating_mul(8) {
            return Ok(false);
        }
        let mut reader = BufReader::new(
            File::open(path).map_err(|error| format!("SFMTキャッシュを確認できません: {error}"))?,
        );
        let mut magic = [0_u8; 8];
        let mut actual_version = [0_u8; 4];
        let mut actual_seed = [0_u8; 4];
        let mut actual_count = [0_u8; 8];
        reader
            .read_exact(&mut magic)
            .and_then(|_| reader.read_exact(&mut actual_version))
            .and_then(|_| reader.read_exact(&mut actual_seed))
            .and_then(|_| reader.read_exact(&mut actual_count))
            .map_err(|error| format!("SFMTキャッシュのヘッダーを確認できません: {error}"))?;
        Ok(&magic == CACHE_MAGIC
            && u32::from_le_bytes(actual_version) == version
            && u32::from_le_bytes(actual_seed) == seed
            && u64::from_le_bytes(actual_count) == value_count)
    }
}
