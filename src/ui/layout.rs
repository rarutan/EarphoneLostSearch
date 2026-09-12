//! 複数ビューが共有する固定レイアウト寸法。
//!
//! 表示内容や計算用の定数は各ビュー／アプリケーション層に残し、矩形配置に使う
//! 共通寸法だけをここで管理する。

pub(crate) const TOP_PANEL_HEIGHT: f32 = 220.0;
pub(crate) const LEFT_PANEL_WIDTH: f32 = 430.0;
pub(crate) const PANEL_GAP: f32 = 8.0;
pub(crate) const PANEL_PADDING: f32 = 8.0;
pub(crate) const TIMER_PANEL_HEIGHT: f32 = 84.0;
pub(crate) const ACTION_ROW_HEIGHT: f32 = 32.0;
pub(crate) const MIN_WINDOW_WIDTH: f32 = 420.0;
pub(crate) const MIN_WINDOW_HEIGHT: f32 = 120.0;
