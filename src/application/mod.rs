//! 検索処理と観測セッションを束ねるアプリケーション層。

pub(crate) mod defaults;
pub(crate) mod field;
mod guidance_state;
pub(crate) mod hot_path_metrics;
pub(crate) mod production;
pub(crate) mod state;
pub(crate) mod state_types;
pub(crate) mod timer_state;
