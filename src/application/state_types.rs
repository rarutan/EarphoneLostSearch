/// Application層で共有する入力スナップショットと結果型。
///
/// 状態遷移の実装とは分離し、検索・補正・タイマーが受け渡すデータの形を
/// ひとつの場所で確認できるようにする。

#[derive(Clone, Copy)]
pub struct SearchConfig {
    pub seed: u32,
    pub range_start: i64,
    pub range_end: i64,
    pub target: i64,
    pub npc_models: usize,
    pub fps: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HatchGuidance {
    pub current_frame: i64,
    /// 現在位置の○×が計算済みならSome、まだ計算中ならNone。
    pub current_reachability: Option<bool>,
    pub reachable_now: bool,
    /// 現在が○なら現在行を除いた次の○まで、×なら現在位置からの回数。
    pub blinks_until_circle: usize,
    pub next_circle_frame: Option<i64>,
    /// 現在位置から次の○までの実時間。最速閉じの案内表示に使う。
    pub next_circle_seconds: Option<f64>,
    pub grace_seconds: Option<f64>,
    /// 通常待機で現在位置からTargetまでに必要な実時間。
    pub target_wait_seconds: Option<f64>,
    /// 最速閉じの理論上の候補範囲をすべて調べても○がなかった状態。
    pub target_unreachable: bool,
    /// Targetが現在の案内検索範囲の外にあり、到達不能ではなく検索対象外の状態。
    pub search_out_of_range: bool,
}

/// 右上案内の1チャンク分をバックグラウンド計算するための入力。
/// UIスレッドはこの値を組み立てるだけで、SFMT列の生成と到達判定を行わない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuidanceSearchRequest {
    pub session_generation: u64,
    pub observed_start: i64,
    pub observed_end: i64,
    pub range_start: i64,
    pub range_end: i64,
    pub seed: u32,
    pub target: i64,
    pub models: usize,
    pub offset: i64,
    pub consider_rotom: bool,
    pub rotom_threshold: u64,
    pub observed_rotom_talk: Option<bool>,
    pub pre_rotom_consumption: i64,
    pub npc_initial_load: i64,
    pub fastest_ticks: Option<i64>,
}

/// 候補ごとの本番Target到達判定をUIスレッドから切り離すための入力。
/// 選択中候補のタイマー用TimelineはAppState側で保持し、その他の候補は
/// このスナップショットをワーカースレッドでまとめて判定する。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateReachabilityRequest {
    pub session_generation: u64,
    pub seed: u32,
    pub target: i64,
    pub models: usize,
    pub offset: i64,
    pub consider_rotom: bool,
    pub rotom_threshold: u64,
    pub observed_rotom_talk: Option<bool>,
    pub pre_rotom_consumption: i64,
    pub npc_initial_load: i64,
    pub fastest_ticks: Option<i64>,
    pub candidate_positions: Vec<(usize, i64)>,
}

impl GuidanceSearchRequest {
    pub fn same_configuration(self, other: Self) -> bool {
        self.session_generation == other.session_generation
            && self.range_start == other.range_start
            && self.range_end == other.range_end
            && self.seed == other.seed
            && self.target == other.target
            && self.models == other.models
            && self.offset == other.offset
            && self.consider_rotom == other.consider_rotom
            && self.rotom_threshold == other.rotom_threshold
            && self.observed_rotom_talk == other.observed_rotom_talk
            && self.pre_rotom_consumption == other.pre_rotom_consumption
            && self.npc_initial_load == other.npc_initial_load
            && self.fastest_ticks == other.fastest_ticks
    }
}

/// ずれ補正ボタンで確認する内容。計算後のオフセットと、現在値のまま
/// 本番へ進めた場合の到達可否を同じ開始位置に紐づけて保持する。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CorrectionPreview {
    pub suggested_offset: i64,
    /// ロトム判定・NPC初期読み込み後の本番Timeline開始位置。
    /// 最速閉じでは時間オフセットをTimelineのステップへ反映した位置になる。
    pub timeline_start: i64,
    /// 表示した開始位置からTargetFrameへ厳密に着地できるか。
    pub target_on_timeline: bool,
    /// 表示した開始位置から実測Frameへ厳密に着地できるか。
    pub actual_on_timeline: bool,
    pub actual_frame: i64,
    pub target_frame: i64,
    /// 開始位置から実測Frameを照合して得たロトム判定の変更案。
    /// あり／なしのどちらのTimelineだけが実測Frameに一致した場合に設定する。
    pub rotom_correction: Option<RotomCorrection>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RotomCorrection {
    pub roll: u8,
    pub predicted_talk: bool,
    pub observed_talk: bool,
    pub current_threshold: u64,
    pub suggested_threshold: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Idle,
    Observing,
    Ready,
    Production,
    Countdown,
    Encounter,
    Finished,
    Correction,
}

/// 観測セッションの共通した意味を表す表示・操作フェーズ。
///
/// 孵化とフィールドでは内部のタイマー構造が異なるため、既存の詳細な
/// モードは保持する。この型はUIが個別のフラグを組み合わせず、現在の
/// フェーズと操作可否を問い合わせるための共通語彙として使う。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionPhase {
    Input,
    Observing,
    TimelineReady,
    Timer,
    ProductionFinished,
    Correction,
}

/// 観測列と一致した1候補分の情報。
///
/// これを候補の1行として扱うことで、同じ乱数列についての
/// SFMT位置・次の瞬きまでの時間・本番到達可否が分離した配列でずれないようにする。
#[derive(Clone, Copy)]
pub struct TimelineCandidate {
    #[allow(dead_code)]
    pub start_position: i64,
    pub sfmt_position: i64,
    /// 次の瞬きまでの時間チェック回数。TargetFrameとは別の単位。
    pub next_blink_ticks: Option<i64>,
    pub target_reachable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArticleTimerStage {
    Offset,
    ToTarget,
}
