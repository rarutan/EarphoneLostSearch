//! アプリケーション状態に付随するメッセージの表示区分。

/// メッセージを利用者の確認が必要なものとして表示するかどうか。
///
/// メッセージ本文は画面文言として保持するが、表示区分は本文の文字列検索で
/// 推測せず、状態遷移の結果として明示的に更新する。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum StatusKind {
    #[default]
    Informational,
    CandidateNotFound,
    SearchRange,
    Error,
}

impl StatusKind {
    pub(crate) const fn is_field_notice(self) -> bool {
        matches!(self, Self::CandidateNotFound | Self::SearchRange)
    }
}
