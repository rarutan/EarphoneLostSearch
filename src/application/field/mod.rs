//! フィールド検索と実行時状態を束ねる公開窓口。
//!
//! 検索計算と状態所有者は内部moduleに分け、既存の`application::field::*` APIを維持する。

mod core;

pub(crate) use core::*;
