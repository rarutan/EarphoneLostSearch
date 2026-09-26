//! アプリケーション全体で共有する、分類可能な失敗理由。
//!
//! `Display`は人に見せる文言への変換専用とし、制御フローでは必ずvariantを照合する。

use std::fmt;

#[derive(Debug)]
pub enum AppError {
    Cancelled,
    InvalidSeed,
    InvalidInteger(InputField),
    NegativeValue(InputField),
    OutOfRange {
        field: InputField,
        min: i64,
        max: i64,
    },
    InvalidNpcRange,
    NpcCountTooLarge,
    NpcNumberTooLarge,
    NpcRangeDoesNotContainTarget,
    MissingObservation,
    InvalidObservation,
    TooManyObservations {
        max: usize,
    },
    InvalidFps,
    InvalidOffset {
        index: usize,
    },
    NegativeOffset {
        index: usize,
    },
    OffsetMustBeEven,
    ArithmeticOverflow(ArithmeticOperation),
    Cache(CacheError),
    InvalidState(StateIssue),
    Persistence(PersistenceError),
    CacheWorkerJoin,
    WorkerPanicked,
    InvalidFieldConfiguration,
    SearchSpanTooWide {
        max: i64,
    },
}

#[derive(Clone, Copy, Debug)]
pub enum InputField {
    SearchStart,
    SearchEnd,
    TargetFrame,
    ActualFrame,
    NpcCount,
    NpcNumber,
    Tolerance,
    WaitFrame,
    TargetConsumption,
    Offset,
    InitialBranchSearchSpan,
}

#[derive(Clone, Copy, Debug)]
pub enum ArithmeticOperation {
    CacheRange,
    CacheLength,
    TimelinePosition,
    TimelineLength,
    FrameSum,
    OffsetCalculation,
}

#[derive(Debug)]
pub enum CacheError {
    Io {
        operation: CacheIoOperation,
        source: std::io::Error,
    },
    OutOfRange,
    InvalidRange,
    EmptyCache,
    StateUnavailable,
    TempNameExhausted,
    SeedMismatch,
}

#[derive(Clone, Copy, Debug)]
pub enum CacheIoOperation {
    CreateDirectory,
    CreateFile,
    Open,
    Seek,
    Read,
    Write,
    SetLength,
    Commit,
    Replace,
}

#[derive(Clone, Copy, Debug)]
pub enum StateIssue {
    CacheUnavailable,
    TimelineUnavailable,
    TimelineCursorUnavailable,
    TargetUnreachable,
    CandidateUnavailable,
    CalculationUnavailable,
    CorrectionModeLocksTimer,
    CandidatesNotUnique,
    BlinkQueuePending,
    BlinkTimerCannotContinue,
    TargetTimerUnavailable,
    ObservationAnchorMissing,
    TimerTargetPassed,
    TimelineLimitReached,
}

#[derive(Debug)]
pub enum PersistenceError {
    PathUnavailable,
    CreateDirectory(std::io::Error),
    Serialize(serde_json::Error),
    Write(std::io::Error),
}

impl AppError {
    pub fn is_cache_io(&self) -> bool {
        matches!(self, Self::Cache(CacheError::Io { .. }))
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use AppError as E;
        match self {
            E::Cancelled => f.write_str("検索をキャンセルしました。"),
            E::InvalidSeed => f.write_str("初期SFMT SEEDは16進数で入力してください。"),
            E::InvalidInteger(field) => write!(f, "{}は整数で入力してください。", field.label()),
            E::NegativeValue(field) => write!(f, "{}は0以上で入力してください。", field.label()),
            E::OutOfRange { field, min, max } => {
                write!(
                    f,
                    "{}は{}〜{}の範囲で入力してください。",
                    field.label(),
                    min,
                    max
                )
            }
            E::InvalidNpcRange => f.write_str("NPC範囲は例: 10~20 の形式で入力してください。"),
            E::NpcCountTooLarge => f.write_str("NPC数が大きすぎます。"),
            E::NpcNumberTooLarge => f.write_str("NPC番号が大きすぎます。"),
            E::NpcRangeDoesNotContainTarget => {
                f.write_str("NPC範囲に観測対象NPC番号を含む設定がありません。")
            }
            E::MissingObservation => f.write_str("瞬き間隔を1つ以上入力してください。"),
            E::InvalidObservation => f.write_str("瞬き間隔は0以上の整数で入力してください。"),
            E::TooManyObservations { max } => write!(f, "瞬き観測は{}回まで入力できます。", max),
            E::InvalidFps => f.write_str("FPSは0より大きい数値で入力してください。"),
            E::InvalidOffset { index } => {
                write!(f, "オフセット{}は整数で入力してください。", index + 1)
            }
            E::NegativeOffset { index } => {
                write!(f, "オフセット{}は0以上で入力してください。", index + 1)
            }
            E::OffsetMustBeEven => f.write_str("オフセットは2F単位で入力してください。"),
            E::ArithmeticOverflow(operation) => write!(f, "{}が大きすぎます。", operation.label()),
            E::Cache(error) => error.fmt(f),
            E::InvalidState(_) => f.write_str("Invalid application state"),
            E::Persistence(error) => error.fmt(f),
            E::CacheWorkerJoin => f.write_str("SFMTプールの計算に失敗しました。"),
            E::WorkerPanicked => f.write_str("バックグラウンド計算に失敗しました。"),
            E::InvalidFieldConfiguration => {
                f.write_str("タイムライン仕様の入力範囲または観測対象を確認してください。")
            }
            E::SearchSpanTooWide { max } => write!(f, "検索範囲は{}F以内で指定してください。", max),
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Cache(CacheError::Io { source, .. }) => Some(source),
            Self::Persistence(
                PersistenceError::CreateDirectory(source) | PersistenceError::Write(source),
            ) => Some(source),
            Self::Persistence(PersistenceError::Serialize(source)) => Some(source),
            _ => None,
        }
    }
}

impl InputField {
    fn label(self) -> &'static str {
        match self {
            Self::SearchStart => "検索範囲の始点Frame",
            Self::SearchEnd => "検索範囲の終点Frame",
            Self::TargetFrame => "Target Frame",
            Self::ActualFrame => "実際に出現したTarget Frame",
            Self::NpcCount => "NPC数",
            Self::NpcNumber => "NPC番号",
            Self::Tolerance => "許容差",
            Self::WaitFrame => "実際の待機Frame",
            Self::TargetConsumption => "Target消費数",
            Self::Offset => "オフセットの表示F",
            Self::InitialBranchSearchSpan => "StartingFrame検索範囲",
        }
    }
}

impl ArithmeticOperation {
    fn label(self) -> &'static str {
        match self {
            Self::CacheRange => "SFMTキャッシュの読み込み範囲",
            Self::CacheLength => "SFMTキャッシュの長さ",
            Self::TimelinePosition => "Timelineの開始位置",
            Self::TimelineLength => "Timeline範囲",
            Self::FrameSum => "待機Frameの合計",
            Self::OffsetCalculation => "計算した補正",
        }
    }
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use CacheError as C;
        match self {
            C::Io { operation, source } => write!(f, "{}: {source}", operation.label()),
            C::OutOfRange => f.write_str("SFMTキャッシュの範囲外を読み込もうとしました。"),
            C::InvalidRange => f.write_str("SFMTキャッシュの範囲が不正です。"),
            C::EmptyCache => f.write_str("空のSFMTキャッシュから状態を復元できません。"),
            C::StateUnavailable => f.write_str("SFMTキャッシュから状態を復元できません。"),
            C::TempNameExhausted => {
                f.write_str("SFMTキャッシュ用の一時ファイル名を確保できません。")
            }
            C::SeedMismatch => f.write_str("SFMTキャッシュが検索条件と一致しません。"),
        }
    }
}

impl std::error::Error for CacheError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl CacheIoOperation {
    fn label(self) -> &'static str {
        match self {
            Self::CreateDirectory => "SFMTキャッシュ用フォルダーを作れません",
            Self::CreateFile => "SFMTキャッシュを作れません",
            Self::Open => "SFMTキャッシュを開けません",
            Self::Seek => "SFMTキャッシュを移動できません",
            Self::Read => "SFMTキャッシュを読み込めません",
            Self::Write => "SFMTキャッシュを書き込めません",
            Self::SetLength => "SFMTキャッシュの長さを更新できません",
            Self::Commit => "SFMTキャッシュを確定できません",
            Self::Replace => "SFMTキャッシュを配置できません",
        }
    }
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PathUnavailable => f.write_str("オフセット保存先を決められません。"),
            Self::CreateDirectory(error) => {
                write!(f, "オフセット保存用フォルダを作成できません: {error}")
            }
            Self::Serialize(error) => write!(f, "オフセットを保存用JSONへ変換できません: {error}"),
            Self::Write(error) => write!(f, "オフセットを保存できません: {error}"),
        }
    }
}

impl std::error::Error for PersistenceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CreateDirectory(error) | Self::Write(error) => Some(error),
            Self::Serialize(error) => Some(error),
            _ => None,
        }
    }
}

pub type AppResult<T> = std::result::Result<T, AppError>;
