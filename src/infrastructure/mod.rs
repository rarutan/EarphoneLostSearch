//! 外部システムとの接続をまとめる層。
//!
//! Domain/Application層がegui、音声デバイス、ファイルシステムへ直接依存しないように、
//! 副作用を伴う実装をここへ置く。

pub(crate) mod audio;
pub(crate) mod cache_store;
pub(crate) mod offset_store;
pub(crate) mod persistence;
pub(crate) mod settings_store;
