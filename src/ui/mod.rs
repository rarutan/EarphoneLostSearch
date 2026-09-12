//! eguiを使ったアプリケーション画面と入出力の境界。

mod app;
mod app_types;
mod beep_scheduler;
pub(crate) mod field_view;
pub(crate) mod hatch_view;
pub(crate) mod i18n;
pub(crate) mod layout;
mod search_jobs;
pub(crate) mod settings_view;
pub(crate) mod timer_view;
pub(crate) mod widgets;

pub(crate) use app::UiApp;
