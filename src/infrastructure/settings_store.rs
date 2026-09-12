//! アプリケーション設定の保存先を解決する。
//!
//! OSごとのユーザーデータディレクトリを使い、実行ファイルの位置や作業ディレクトリに
//! 設定ファイルが依存しないようにする。

use std::path::PathBuf;

/// OSごとのユーザー設定ルートを返す。
///
/// 設定の保存先をUIやドメイン状態から切り離し、将来ポータブルモードや
/// 保存先の選択肢を追加しても、呼び出し側の状態モデルから切り離せるようにする。
pub(crate) fn config_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA")
            .or_else(|| std::env::var_os("LOCALAPPDATA"))
            .map(PathBuf::from)
    }

    #[cfg(target_os = "macos")]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join("Library").join("Application Support"))
            })
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".config"))
            })
    }

    #[cfg(not(any(windows, unix)))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

/// アプリ固有の設定ファイルへのパスを返す。実際の作成・読み書きは呼び出し側が行う。
pub(crate) fn app_file(file_name: &str) -> Option<PathBuf> {
    Some(
        config_directory()?
            .join("EarphoneLostSearch")
            .join(file_name),
    )
}
