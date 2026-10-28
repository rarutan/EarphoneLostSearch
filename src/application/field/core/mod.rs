//! フィールド観測の検索、Timeline生成、到達判定を提供する。
//!
//! 検索範囲と観測列の照合は純粋な計算として実行し、UI層から渡された
//! スナップショットだけを使って候補と本番用Timelineを生成する。

#[cfg(test)]
use crate::domain::rng::generate_u64_values_from;
use crate::domain::rng::{
    ModelConsumption, ModelStatus, ObservationCache, Sfmt, MAX_CONSECUTIVE_IDLE_TICKS,
    MAX_SFMT_FRAME,
};
use crate::domain::timing::{game_frames_to_seconds, seconds_to_game_frames};
use crate::errors::{AppError, AppResult, ArithmeticOperation, CacheError, InputField};
use rayon::prelude::*;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

#[path = "state/mod.rs"]
mod state;
pub(crate) use state::*;

pub(super) const MAX_SEARCH_FRAME: i64 = MAX_SFMT_FRAME;
/// タイムライン仕様の1回の検索で許可するSFMT消費範囲の幅。
pub const MAX_SEARCH_SPAN: i64 = 100_000;
const MIN_TIMELINE_FRAME: i64 = 477;
// 1/30秒tickを表示Frameへ直すと2倍になる。検索の先読みは十分な余裕を持たせる。
pub(super) const MAX_WAIT_TICKS: i64 = 5_000;
/// 一意候補のTimelineを遅延延長する1/30秒単位のチャンクサイズ。
const TIMELINE_CHUNK_TICKS: i64 = 4_096;
/// 画面表示用に保持する瞬き行数。乱数状態と観測要約は別に保持するため、
/// 古い表示行はこの範囲から外れた時点で破棄できる。
pub(crate) const MAX_RETAINED_TIMELINE_ROWS: usize = 256;
// 起動直後のTimelineウォームアップは、全検索範囲を先行走査しない。
// 観測入力までに間に合う小さな先頭チャンクだけを作り、残りは詳細検索へ委ねる。
#[allow(dead_code)]
const TIMELINE_WARMUP_STARTS: i64 = 1_024;
#[allow(dead_code)]
const TIMELINE_WARMUP_TICKS: i64 = 1_024;
/// 未来のTimelineを延長できるSFMT絶対位置の上限。
/// 候補開始位置を検索する範囲の上限とは別に扱う。
pub(super) const MAX_FUTURE_TIMELINE_TICKS: i64 = MAX_SFMT_FRAME;
pub(super) const MAX_MODELS: usize = 256;
/// NPC入力で許可するNPC数の上限（主人公1体は内部で加算）。
pub const MAX_NPC_COUNT: usize = 50;
/// 複数NPC検索で指定できるNPC数の範囲幅（終点−始点）。
const MAX_NPC_RANGE_WIDTH: usize = 20;
const PRECOMPUTED_TIMELINE_EVENTS: usize = 8;
const BLINK_TIMER_PREFETCH_COUNT: usize = 32;
const BLINK_TIMER_REFILL_THRESHOLD: usize = 8;
/// 候補Timelineが保持する観測先読み行数の上限。
/// Target付きのTimelineはTargetで打ち切る場合があり、idx不明の仮候補が
/// すべての先読みtickを走査して計算量を増やさないようにする。
const MAX_CANDIDATE_TIMELINE_EVENTS: usize = 64;
// SSDキャッシュから一度に取り出す開始位置のまとまり。検索範囲とTimelineの
// 事前計算を同じチャンクへ分け、100,000件の範囲でも一括走査でUIを塞がない。
const SEARCH_CHUNK_STARTS: i64 = 4_096;
// 候補確認窓を超えて開始位置を計算しないための検索単位。SFMTキャッシュの
// 読み出し単位とは分け、確認窓（1000F）を跨ぐ過剰計算を小さく抑える。
const SEARCH_CANDIDATE_CHUNK_STARTS: i64 = 512;
/// 最初の一致候補を採用する前に確認する、開始位置からの追加範囲。
/// 窓内に複数候補がある場合は誤採用を避けるため全範囲へ戻る。
const CANDIDATE_CONFIRM_SPAN: i64 = 1_000;
/// 許容差が大きい場合も開始位置ごとの照合窓を分けて保持する。
/// 同じ開始位置から同等の仮候補が無制限に増えないよう、候補数には上限を設ける。
const MAX_MATCHING_WINDOWS_PER_START: usize = 64;
/// 初期読み込み境界を探すために基準位置から走査する最小幅。
/// NPC番号やモデル数とは独立した値で、特殊消費の候補開始位置だけを拾う。
/// 初期分岐探索幅の既定値。設定画面から変更できる。
pub const DEFAULT_INITIAL_BRANCH_SCAN_SPAN: i64 = 30;
#[cfg(test)]
pub const INITIAL_BRANCH_SCAN_SPAN: i64 = DEFAULT_INITIAL_BRANCH_SCAN_SPAN;
/// 初期分岐探索幅として許可する上限。
pub const MAX_INITIAL_BRANCH_SCAN_SPAN: i64 = 1_000;

// ---------------------------------------------------------------------------
// 検索ドメイン型とTimeline索引の構築
// ---------------------------------------------------------------------------

fn check_cancelled(cancel: Option<&AtomicBool>) -> AppResult<()> {
    if cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
        Err(AppError::Cancelled)
    } else {
        Ok(())
    }
}

/// タイムライン仕様の通常タイムライン検索で使う入力値。
///
/// `range_start..=range_end`は、外部ツールのSFMT消費数の候補範囲である。
/// 観測された間隔はゲーム内表示Frame（1/30秒tickの2倍）で照合する。
#[derive(Clone, Debug)]
pub struct FieldConfig {
    pub seed: u32,
    pub range_start: i64,
    pub range_end: i64,
    /// 検索対象として有効にするModelStatusの総数。
    pub models: usize,
    /// 瞬きを観測するModelStatusの番号。0は主人公、1以上はNPC。
    pub target_model: usize,
    pub observed_intervals: Vec<i64>,
    pub tolerance: i64,
    pub target_consumption: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldResult {
    /// タイムライン仕様のResultに相当する開始SFMT消費数。
    pub start_consumption: i64,
    /// 開始消費数から最初／最後の観測瞬きまでの表示Frame。
    pub first_wait_frame: i64,
    pub last_wait_frame: i64,
    /// 各観測瞬き直後のSFMT消費位置。
    pub first_consumption: i64,
    pub last_consumption: i64,
    pub intervals: Vec<i64>,
    /// 観測対象モデルが各瞬きで実際に参照したSFMT絶対位置。
    /// 全モデル処理後の合計カーソルとは異なり、候補Timelineの合流判定に使う。
    pub target_consumptions: Vec<i64>,
    /// 最後に観測した瞬きから、次に同じモデルが瞬きするまでの表示Frame。
    pub next_blink_wait_frame: i64,
    /// Target消費数を指定した場合の、最後の観測瞬きからの表示Frame。
    pub target_wait_frame: Option<i64>,
    /// Targetを正確に踏めたか。falseの場合もtarget_wait_frameは代替時刻を持つ。
    pub target_exact: Option<bool>,
    pub target_frame_before: Option<i64>,
    pub target_frame_after: Option<i64>,
    /// 観測されたモデルのidx。未知モードでは候補ごとに異なる。
    pub observed_model: usize,
    /// この候補のTimelineを生成したModelStatus数（主人公を含む）。
    /// UIでNPC数を表示するときは主人公1体分を除く。
    pub models: usize,
    /// 観測後の現在位置から続く瞬きタイムライン。
    pub timeline: Vec<FieldTimelineRow>,
    /// 候補の追加照合に使う内部イベント。画面表示用Timelineとは分離する。
    candidate_blinks: Vec<CandidateBlink>,
    /// 最後に観測した瞬きからTargetまでに見込まれる瞬き回数。
    /// Target未設定または計算範囲外ならNone。
    pub target_blink_count: Option<usize>,
    /// 表示Timelineの先頭行から最後の観測行までの瞬き行数。
    /// 行履歴が切り詰められてもTargetまでの回数を表示できるよう保持する。
    pub(crate) target_blink_count_offset_from_first_row: i64,
    /// Timelineを遅延延長するための継続状態。結果と同じ候補に保持し、チャンク境界で
    /// ModelStatusとremainを初期化し直さない。
    timeline_cursor: Option<TimelineCursor>,
    /// 最後に観測した瞬き直後の状態。Target判定は先読み済みTimelineの末尾ではなく、
    /// この状態から延長する。
    target_cursor: Option<TimelineCursor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldTimelineRow {
    /// 外部ツールで照合するSFMT消費位置。
    pub frame: i64,
    /// 検索開始からの絶対Timeline表示Frame（1tick=2F）。
    pub timeline_frame: i64,
    /// この位置から次の瞬きまでのゲーム内表示Frame。
    pub duration_frames: i64,
}

/// 候補照合に必要な瞬き情報。表の描画データには渡さない。
#[derive(Clone, Debug, PartialEq, Eq)]
struct CandidateBlink {
    frame: i64,
    timeline_frame: i64,
    /// 実測観測ですでに通過した瞬き行。
    observed: bool,
    /// この行の瞬きで観測対象モデルが参照したSFMT位置。
    target_consumption: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TimelineCursor {
    pub(crate) status: ModelStatus,
    pub(crate) stream_cursor: i64,
    pub(crate) consumption: i64,
    pub(crate) tick: i64,
    pub(crate) timeleap_runtime: Option<crate::domain::timeleap::TimeleapContinuation>,
}

/// フィールド検索のNPC番号指定モード。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldTargetMode {
    Known,
}

/// 実際に取得できたFrameから逆算した、観測対象idxの候補。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldIdxInference {
    pub indices: Vec<usize>,
    pub results: Vec<(usize, Vec<FieldResult>)>,
    /// ずれ補正ダイアログへ渡す、現在オフセットを含めた提案値。
    pub suggested_offset: Option<i64>,
}

/// 指定idxを優先して検索した結果。指定idxに一致しない場合は、
/// 全idxへフォールバックしたことと、その一致idxを呼び出し側へ返す。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldSearchOutcome {
    pub results: Vec<FieldResult>,
    pub fallback_indices: Option<Vec<usize>>,
}

#[derive(Clone)]
struct BlinkEvent {
    wait_frame: i64,
    consumption: i64,
    status: ModelStatus,
    stream_cursor: i64,
    model_consumption: Option<i64>,
}

/// 検索開始消費数を基準に保持するSFMT列。
///
/// `values[0]`は`base`番目の64bit出力に対応する。開始位置より前の列を
/// 保持しないことで、検索範囲が大きい場合のメモリ使用量を抑える。
struct SearchValues {
    base: i64,
    values: Vec<u64>,
}

#[allow(dead_code)]
#[derive(Clone)]
struct PreparedTimelineEntry {
    start_consumption: i64,
    models: usize,
    target_model: usize,
    events: Vec<(i64, i64)>,
}

#[allow(dead_code)]
#[derive(Clone, Default)]
pub(crate) struct PreparedTimelineIndex {
    entries: Vec<PreparedTimelineEntry>,
    range_start: i64,
    range_end: i64,
    configurations: Vec<(usize, usize)>,
}

impl PreparedTimelineIndex {
    #[cfg(test)]
    fn covers(&self, config: &FieldConfig) -> bool {
        self.range_start <= config.range_start
            && self.range_end >= config.range_end
            && self
                .configurations
                .contains(&(config.models, config.target_model))
    }
}

/// タイムライン仕様の検索開始前に生成しておくSFMT出力プール。
///
/// 観測中に作ったプールを検索スレッドへ移動して再利用する。Targetや
/// 範囲が後から変わった場合は`covers`で不足を検出し、検索側で作り直す。
pub(crate) struct FieldSearchPool {
    seed: u32,
    cache: ObservationCache,
    timeline: Option<PreparedTimelineIndex>,
}

impl FieldSearchPool {
    pub(crate) fn covers(&self, config: &FieldConfig) -> bool {
        self.seed == config.seed
            && config.range_start >= 0
            && observation_stream_end(config).min(MAX_SFMT_FRAME) <= self.cache.available_end()
    }

    /// Timeline事前計算ワーカーへ渡す、共有可能なSFMTキャッシュの複製。
    #[allow(dead_code)]
    pub(crate) fn timeline_cache(&self) -> ObservationCache {
        self.cache.clone()
    }

    /// 事前計算が完了した索引をプールへ取り込む。
    pub(crate) fn install_timeline(&mut self, index: PreparedTimelineIndex) {
        self.timeline = Some(index);
    }
}

impl SearchValues {
    fn new(base: i64, values: Vec<u64>) -> Self {
        Self { base, values }
    }

    fn get(&self, absolute_position: i64) -> Option<u64> {
        let relative = absolute_position.checked_sub(self.base)?;
        self.values.get(usize::try_from(relative).ok()?).copied()
    }
}

fn next_state_with_values(
    status: &mut ModelStatus,
    stream_cursor: &mut i64,
    values: &SearchValues,
) -> Option<(usize, Vec<u8>)> {
    let mut exhausted = false;
    let result = status.next_state_with(|| {
        let Some(value) = values.get(*stream_cursor) else {
            exhausted = true;
            return 0;
        };
        *stream_cursor += 1;
        value
    });
    (!exhausted).then_some(result)
}

fn next_state_with_positions_values(
    status: &mut ModelStatus,
    stream_cursor: &mut i64,
    values: &SearchValues,
) -> Option<(usize, Vec<ModelConsumption>)> {
    let mut exhausted = false;
    let result = status.next_state_with_positions(|| {
        let position = *stream_cursor;
        let Some(value) = values.get(position) else {
            exhausted = true;
            return (position, 0);
        };
        *stream_cursor += 1;
        (position, value)
    });
    (!exhausted).then_some(result)
}

fn next_state_target_with_values(
    status: &mut ModelStatus,
    target_model: usize,
    stream_cursor: &mut i64,
    values: &SearchValues,
) -> Option<(usize, bool)> {
    let mut exhausted = false;
    let result = status.next_state_target_with(target_model, || {
        let Some(value) = values.get(*stream_cursor) else {
            exhausted = true;
            return 0;
        };
        *stream_cursor += 1;
        value
    });
    (!exhausted).then_some(result)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TargetTiming {
    absolute_wait_frame: i64,
    exact: bool,
    frame_before: i64,
    frame_after: i64,
}

/// タイムライン仕様の通常検索をRustで独立実装する。
///
/// SFMTの出力列をSSDキャッシュから必要範囲のチャンクだけ読み込み、各開始位置は
/// 同じ列の該当位置から読む。検索結果は最初の一致だけで打ち切らず、範囲内の
/// 全開始消費位置を返す。
#[cfg(test)]
pub fn search(config: &FieldConfig) -> Result<Vec<FieldResult>, AppError> {
    let pool = prepare_search_pool(config)?;
    search_with_cache(config, &pool.cache)
}

/// 観測開始時に先行生成する検索用SFMTプール。
#[cfg(test)]
pub(crate) fn prepare_search_pool(config: &FieldConfig) -> Result<FieldSearchPool, AppError> {
    prepare_search_pool_cancelable(config, None)
}

fn prepare_search_pool_cancelable(
    config: &FieldConfig,
    cancel: Option<&AtomicBool>,
) -> Result<FieldSearchPool, AppError> {
    validate_static_config(config)?;
    check_cancelled(cancel)?;
    // Target到達判定は候補ごとの保存カーソルから行うため、共有観測キャッシュには
    // 瞬き検索に必要な範囲だけを保持する。
    let cache_end = observation_stream_end(config).min(MAX_SFMT_FRAME);
    Ok(FieldSearchPool {
        seed: config.seed,
        cache: crate::domain::rng::generate_observation_cache_cancelable(
            config.seed,
            0..=cache_end,
            0,
            crate::domain::timing::DEFAULT_FPS,
            cancel,
        )?,
        timeline: None,
    })
}

/// 観測開始時に必要なSFMT列だけを先行生成する。
///
/// Timelineは観測間隔が揃ってから検索ワーカーで必要な開始位置だけを計算する。
/// 観測列入力前に全候補を走査せず、最初の瞬き入力を数分間ブロックしない。
#[cfg(test)]
pub(crate) fn prepare_search_pool_with_configs(
    configs: &[FieldConfig],
) -> Result<FieldSearchPool, AppError> {
    prepare_search_pool_with_configs_cancelable(configs, None)
}

pub(crate) fn prepare_search_pool_with_configs_cancelable(
    configs: &[FieldConfig],
    cancel: Option<&AtomicBool>,
) -> Result<FieldSearchPool, AppError> {
    let Some(first) = configs.first() else {
        return Err(AppError::InvalidNpcRange);
    };
    check_cancelled(cancel)?;
    // SFMT列はNPC数に依存せず共有できる。最大モデル数に必要な先読みを一度だけ生成し、
    // 範囲内の小さいモデル数でも再利用する。
    let mut pool_config = first.clone();
    pool_config.models = configs
        .iter()
        .map(|config| config.models)
        .max()
        .unwrap_or(first.models);
    pool_config.target_model = 0;
    prepare_search_pool_cancelable(&pool_config, cancel)
}

#[cfg(test)]
#[allow(dead_code)]
fn prepare_timeline_index(
    cache: &ObservationCache,
    configs: &[FieldConfig],
) -> Result<PreparedTimelineIndex, AppError> {
    prepare_timeline_index_cancelable(cache, configs, None)
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn prepare_timeline_index_cancelable(
    cache: &ObservationCache,
    configs: &[FieldConfig],
    cancel: Option<&AtomicBool>,
) -> Result<PreparedTimelineIndex, AppError> {
    prepare_timeline_index_bounded_cancelable(cache, configs, cancel, MAX_WAIT_TICKS)
}

/// 起動中に作る限定的なTimelineウォームアップ索引。
///
/// 検索範囲全体を先行走査すると、開始位置×NPC設定×5000tickの組合せで
/// 観測入力前に長時間CPUを占有する。先頭チャンクだけを短い先読みで作り、
/// 観測入力が先に来ても数秒以上の待ちを発生させない。
#[allow(dead_code)]
pub(crate) fn prepare_timeline_warmup_cancelable(
    cache: &ObservationCache,
    configs: &[FieldConfig],
    cancel: Option<&AtomicBool>,
) -> Result<PreparedTimelineIndex, AppError> {
    let bounded = configs
        .iter()
        .map(|config| {
            let mut bounded = config.clone();
            bounded.range_end = bounded.range_end.min(
                bounded
                    .range_start
                    .saturating_add(TIMELINE_WARMUP_STARTS - 1),
            );
            bounded
        })
        .collect::<Vec<_>>();
    prepare_timeline_index_bounded_cancelable(cache, &bounded, cancel, TIMELINE_WARMUP_TICKS)
}

fn prepare_timeline_index_bounded_cancelable(
    cache: &ObservationCache,
    configs: &[FieldConfig],
    cancel: Option<&AtomicBool>,
    max_wait_ticks: i64,
) -> Result<PreparedTimelineIndex, AppError> {
    let mut entries = Vec::new();
    for config in configs {
        check_cancelled(cancel)?;
        validate_static_config(config)?;
        let mut start = config.range_start;
        while start <= config.range_end {
            let end = start
                .saturating_add(SEARCH_CHUNK_STARTS.saturating_sub(1))
                .min(config.range_end);
            let stream_end = end
                .saturating_add(MAX_WAIT_TICKS.saturating_mul(config.models as i64))
                .saturating_add(8)
                .min(MAX_SFMT_FRAME);
            let count = stream_end.saturating_sub(start).saturating_add(1).max(0) as usize;
            let values =
                std::sync::Arc::new(SearchValues::new(start, cache.read_values(start, count)?));
            let mut chunk = (start..=end)
                .into_par_iter()
                .filter_map(|start_consumption| {
                    if cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
                        return None;
                    }
                    prepare_timeline_entry_with_limit(
                        start_consumption,
                        config,
                        &values,
                        max_wait_ticks,
                    )
                })
                .collect::<Vec<_>>();
            entries.append(&mut chunk);
            if end == config.range_end {
                break;
            }
            start = end.saturating_add(1);
        }
    }
    check_cancelled(cancel)?;
    entries.sort_by_key(|entry| (entry.models, entry.target_model, entry.start_consumption));
    Ok(PreparedTimelineIndex {
        entries,
        range_start: configs
            .iter()
            .map(|config| config.range_start)
            .min()
            .unwrap_or(0),
        range_end: configs
            .iter()
            .map(|config| config.range_end)
            .max()
            .unwrap_or(0),
        configurations: configs
            .iter()
            .map(|config| (config.models, config.target_model))
            .collect(),
    })
}

fn prepare_timeline_entry_with_limit(
    start_consumption: i64,
    config: &FieldConfig,
    values: &SearchValues,
    max_wait_ticks: i64,
) -> Option<PreparedTimelineEntry> {
    let mut status = ModelStatus::new(config.models);
    let mut stream_cursor = start_consumption;
    let mut tick = 0_i64;
    let scan_end = config.range_end.max(start_consumption.saturating_add(1));
    let mut events = Vec::with_capacity(PRECOMPUTED_TIMELINE_EVENTS);
    while events.len() < PRECOMPUTED_TIMELINE_EVENTS
        && tick < max_wait_ticks
        && stream_cursor <= scan_end
    {
        let (used, target_blinked) = next_state_target_with_values(
            &mut status,
            config.target_model,
            &mut stream_cursor,
            values,
        )?;
        tick += 1;
        if stream_cursor > scan_end {
            break;
        }
        if target_blinked {
            events.push((
                tick * 2,
                start_consumption + (stream_cursor - start_consumption),
            ));
        }
        if used == 0 && tick > max_wait_ticks / 2 {
            break;
        }
    }
    (events.len() >= 2).then_some(PreparedTimelineEntry {
        start_consumption,
        models: config.models,
        target_model: config.target_model,
        events,
    })
}

/// 指定されたidxだけを検索する。
///
/// 指定idxで検索し、別idxへの自動フォールバックは行わない。
/// 呼び出し側が検索対象を切り替える場合は、明示的に別の設定で再検索する。
#[cfg(test)]
#[allow(dead_code)]
pub fn search_with_preferred_idx(config: &FieldConfig) -> Result<FieldSearchOutcome, AppError> {
    let pool = prepare_search_pool(config)?;
    search_with_preferred_idx_with_pool(config, &pool)
}

#[cfg(test)]
pub(crate) fn search_with_preferred_idx_with_pool(
    config: &FieldConfig,
    pool: &FieldSearchPool,
) -> Result<FieldSearchOutcome, AppError> {
    search_with_preferred_idx_with_pool_cancelable(config, pool, None)
}

#[cfg(test)]
fn search_with_preferred_idx_with_pool_cancelable(
    config: &FieldConfig,
    pool: &FieldSearchPool,
    cancel: Option<&AtomicBool>,
) -> Result<FieldSearchOutcome, AppError> {
    validate(config)?;
    check_cancelled(cancel)?;
    let prepared = pool.timeline.as_ref().filter(|index| index.covers(config));
    let preferred = search_with_cache_and_index_cancelable(config, &pool.cache, prepared, cancel)?;
    Ok(FieldSearchOutcome {
        results: preferred,
        fallback_indices: None,
    })
}

/// 複数NPC数を展開した設定を同じSFMTプールへ投入し、結果を一つの候補列へ
/// 統合する。各設定のモデル数はTimeline計算に残るため、同じ開始消費数でも
/// NPC数の違いを候補として保持できる。
#[allow(dead_code)]
pub(crate) fn search_with_npc_range_with_pool(
    configs: &[FieldConfig],
    pool: &FieldSearchPool,
    target_mode: FieldTargetMode,
) -> Result<FieldSearchOutcome, AppError> {
    search_with_npc_range_with_pool_cancelable(configs, pool, target_mode, None)
}

pub(crate) fn search_with_npc_range_with_pool_cancelable(
    configs: &[FieldConfig],
    pool: &FieldSearchPool,
    _target_mode: FieldTargetMode,
    cancel: Option<&AtomicBool>,
) -> Result<FieldSearchOutcome, AppError> {
    search_with_npc_range_with_pool_cancelable_and_span(
        configs,
        pool,
        _target_mode,
        DEFAULT_INITIAL_BRANCH_SCAN_SPAN,
        cancel,
    )
}

/// 初期分岐探索幅を指定して複数NPC検索を実行する。
pub(crate) fn search_with_npc_range_with_pool_cancelable_and_span(
    configs: &[FieldConfig],
    pool: &FieldSearchPool,
    _target_mode: FieldTargetMode,
    initial_branch_scan_span: i64,
    cancel: Option<&AtomicBool>,
) -> Result<FieldSearchOutcome, AppError> {
    check_cancelled(cancel)?;
    // NPC範囲検索では各モデル数のTimeline照合も互いに独立している。
    // idx不明時のモデル並列化と合わせ、範囲内の設定も並列に処理して
    // NPC数の分だけ待ち時間が直列に積み上がらないようにする。
    let outcomes = configs
        .par_iter()
        .map(|config| {
            check_cancelled(cancel)?;
            // NPC番号は検索開始時に固定せず、同じTimelineから全モデルを照合する。
            // 入力された番号の妥当性は候補確定後にUI側で判定する。
            Ok(FieldSearchOutcome {
                results: search_v12_with_cache_cancelable_with_span(
                    config,
                    &pool.cache,
                    true,
                    initial_branch_scan_span,
                    cancel,
                )?,
                fallback_indices: None,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    let mut results = Vec::new();
    let mut fallback_indices = Vec::new();
    for outcome in outcomes {
        results.extend(outcome.results);
        if let Some(indices) = outcome.fallback_indices {
            fallback_indices.extend(indices);
        }
    }
    // 候補確認窓は全NPC設定で共有する。設定ごとに窓を持つと、全体の最初の一致で
    // 除外すべき遠い候補まで残ってしまう。
    if let Some(first) = results.iter().map(|result| result.start_consumption).min() {
        let confirm_end = first.saturating_add(CANDIDATE_CONFIRM_SPAN);
        results.retain(|result| result.start_consumption <= confirm_end);
    }
    results.sort_by_key(|result| {
        (
            result.start_consumption,
            result.observed_model,
            result.first_wait_frame,
            result.last_wait_frame,
        )
    });
    results = merge_observable_candidates(results);
    fallback_indices.sort_unstable();
    fallback_indices.dedup();
    Ok(FieldSearchOutcome {
        results,
        fallback_indices: (!fallback_indices.is_empty()).then_some(fallback_indices),
    })
}

/// 完全に同じ候補だけを検索段階で重複排除する。
///
/// 観測中に`remain`とSFMTカーソルが合流した候補は、過去の列が異なっていても
/// 後段の`FieldState::normalize_search_results`で現在状態をキーに統合する。
/// ここでは候補を早期に捨てず、NPC番号による代表選択に必要な履歴を保持する。
fn merge_observable_candidates(mut results: Vec<FieldResult>) -> Vec<FieldResult> {
    results.sort_by_key(|result| {
        (
            result.start_consumption,
            result.observed_model,
            result.first_wait_frame,
            result.last_wait_frame,
        )
    });
    let mut representatives: Vec<FieldResult> = Vec::with_capacity(results.len());
    for result in results {
        if representatives.iter().any(|representative| {
            representative.start_consumption == result.start_consumption
                && representative.models == result.models
                && representative.observed_model == result.observed_model
                && representative.first_wait_frame == result.first_wait_frame
                && representative.last_wait_frame == result.last_wait_frame
                && representative.intervals == result.intervals
                && representative.target_consumptions == result.target_consumptions
        }) {
            continue;
        }
        representatives.push(result);
    }
    representatives
}

/// タイムライン仕様互換の通常検索。
///
/// `range_start`は基準となるSFMT消費数、`range_end - range_start`は
/// その位置からTimelineを走査するtick数として扱う。参照実装が試す開始位置は
/// 基準消費数から総モデル数個だけで、GUIの範囲全体を開始候補にはしない。
fn search_v12_with_cache_cancelable_with_span(
    config: &FieldConfig,
    cache: &ObservationCache,
    all_models: bool,
    initial_branch_scan_span: i64,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<FieldResult>, AppError> {
    validate_static_config(config)?;
    check_cancelled(cancel)?;
    let raw_config = reference_tick_config(config);
    let base = config.range_start;
    let interval_end = reference_search_stream_end(&raw_config);
    let interval_count = interval_end.saturating_sub(base).saturating_add(1).max(0) as usize;
    let interval_values = SearchValues::new(base, cache.read_values(base, interval_count)?);
    // 初期読み込みの境界候補だけを開始位置として展開する。NPC番号を開始位置へ
    // 加算せず、同じTimelineから全モデルの瞬き列を照合する。
    let candidate_starts = candidate_start_positions_with_span(
        base,
        config.range_end,
        initial_branch_scan_span,
        &interval_values,
    );
    // 候補開始位置ごとの間隔照合は独立しているため並列化する。結果を開始位置順に
    // 並べ直してから、同じ開始位置・同じモデル数の代表候補をUI側で選択する。
    let mut results = candidate_starts
        .into_par_iter()
        .flat_map_iter(|start_consumption| {
            search_v12_start_all_models(
                start_consumption,
                &raw_config,
                config,
                &interval_values,
                all_models,
                // 間隔照合の段階で短い継続Timelineも生成する。Target入力は任意なので、
                // 後段の到達判定がなくても瞬きタイマーを利用できるようにする。
                true,
                cancel,
            )
        })
        .collect::<Vec<_>>();
    results.sort_by_key(|result| {
        (
            result.start_consumption,
            result.observed_model,
            result.first_wait_frame,
            result.last_wait_frame,
        )
    });

    // 観測列に一致したモデル番号が入力番号と異なる場合は、入力番号へ
    // 開始位置を正規化してから同じ条件でTimelineを再生成する。元候補より
    // 前へ戻る変換は、別の初期読み込みや-5分岐を含むため候補にしない。
    results =
        resolve_observed_model_candidates(results, config, &raw_config, &interval_values, cancel);

    let Some(target) = config.target_consumption else {
        results.sort_by_key(|result| {
            (
                result.start_consumption,
                result.observed_model,
                result.first_wait_frame,
                result.last_wait_frame,
            )
        });
        return Ok(results);
    };
    // Target到達判定は保存済みの連続状態から行い、巨大なSFMT後続列を読み込まない。
    // 候補ごとに独立しているため並列化し、最後に開始位置順へ戻す。
    // 全モデル照合時も各モデルの結果へ個別に適用する。UIは候補確定後に
    // 入力NPC番号へ一致する行を選び、未一致なら代表行を仮表示する。
    let mut targeted = results
        .into_par_iter()
        .map(|result| apply_target_from_cursor(result, config.seed, target, Some(cache), cancel))
        .collect::<AppResult<Vec<_>>>()?;
    targeted.sort_by_key(|result| {
        (
            result.start_consumption,
            result.observed_model,
            result.first_wait_frame,
            result.last_wait_frame,
        )
    });
    Ok(targeted)
}

/// 観測列に一致したモデル番号を、入力されたモデル番号へ変換して再検証する。
///
/// 変換後の開始位置が元候補より前になる場合は、候補を破棄せず元の候補を
/// NPC番号不一致のフォールバックとして残す。変換後Timelineが観測したSFMT
/// 位置列まで一致した場合だけ、再生成結果を採用する。
fn resolve_observed_model_candidates(
    results: Vec<FieldResult>,
    display_config: &FieldConfig,
    raw_config: &FieldConfig,
    values: &SearchValues,
    cancel: Option<&AtomicBool>,
) -> Vec<FieldResult> {
    let requested_model = display_config.target_model;
    let mut resolved = Vec::with_capacity(results.len());
    let mut fallbacks = Vec::new();
    for result in results {
        if result.observed_model == requested_model {
            resolved.push(result);
            continue;
        }

        let Some(shifted_start) = shifted_start_for_requested_model(
            result.start_consumption,
            result.observed_model,
            requested_model,
        ) else {
            fallbacks.push(result);
            continue;
        };
        if shifted_start < display_config.range_start || shifted_start > display_config.range_end {
            fallbacks.push(result);
            continue;
        }

        let regenerated = search_v12_start_all_models(
            shifted_start,
            raw_config,
            display_config,
            values,
            true,
            true,
            cancel,
        );
        let matching = regenerated.into_iter().filter(|candidate| {
            candidate.observed_model == requested_model
                && candidate.intervals == result.intervals
                && candidate.target_consumptions == result.target_consumptions
        });
        let mut accepted = false;
        for candidate in matching {
            accepted = true;
            resolved.push(candidate);
        }
        if !accepted {
            // 入力番号へ変換できない候補は、他に有効候補がある場合は破棄する。
            // 全候補が不一致だった場合だけ、後段でNPC番号×を表示する代表として残す。
            fallbacks.push(result);
        }
    }
    if resolved.is_empty() {
        // 1〜6段階では、NPC番号不一致時の代表選択をまだ行わない。
        // 元候補を残して`NPC番号×`または複数候補としてUIへ返す。
        resolved = fallbacks;
    }
    merge_observable_candidates(resolved)
}

/// 観測済みモデルを入力モデルへ変換した開始位置を返す。
///
/// 元候補の開始位置より前へ戻る変換は、候補の因果関係を壊して`-5`を含む
/// 別の初期状態を混ぜるため、候補として扱わない。
fn shifted_start_for_requested_model(
    start_consumption: i64,
    observed_model: usize,
    requested_model: usize,
) -> Option<i64> {
    let shifted = start_consumption.checked_add(observed_model as i64 - requested_model as i64)?;
    (shifted >= start_consumption).then_some(shifted)
}

/// 初期読み込み境界を表す候補開始位置を生成する。
///
/// `base`そのものは常に候補に含め、基準位置から探索幅内で特殊値を見つけた
/// 場合だけ、そのSFMT位置の直後を別のTimeline開始位置として追加する。
/// NPC番号やモデル数は開始位置へ加算しない。
#[cfg(test)]
fn candidate_start_positions(base: i64, range_end: i64, values: &SearchValues) -> Vec<i64> {
    candidate_start_positions_with_span(base, range_end, DEFAULT_INITIAL_BRANCH_SCAN_SPAN, values)
}

fn candidate_start_positions_with_span(
    base: i64,
    range_end: i64,
    scan_span: i64,
    values: &SearchValues,
) -> Vec<i64> {
    let scan_end = base
        .saturating_add(scan_span.max(0))
        .min(range_end)
        .min(MAX_SFMT_FRAME);
    let mut starts = vec![base];
    if scan_end >= base {
        for position in base..=scan_end {
            let Some(value) = values.get(position) else {
                continue;
            };
            if value & 0x7f == 0 {
                let candidate = position.saturating_add(1);
                if candidate <= range_end && candidate <= MAX_SFMT_FRAME {
                    starts.push(candidate);
                }
            }
        }
    }
    starts.sort_unstable();
    starts.dedup();
    starts
}

/// 表示Frameで保持している観測値を、参照実装の1/30秒tickへ変換する。
fn reference_tick_config(config: &FieldConfig) -> FieldConfig {
    let mut converted = config.clone();
    converted.observed_intervals = config
        .observed_intervals
        .iter()
        .map(|frame| ((*frame as f64) / 2.0).round() as i64)
        .collect();
    // GUIの許容差は表示Frame単位なので、参照Timelineのtick単位へ変換する。
    // 奇数Frameは切り上げ、入力した許容範囲を狭めないようにする。
    converted.tolerance = (config.tolerance.saturating_add(1)) / 2;
    converted
}

fn reference_search_stream_end(config: &FieldConfig) -> i64 {
    let elapsed_ticks = config.range_end.saturating_sub(config.range_start).max(0);
    config
        .range_start
        .saturating_add(config.models.saturating_sub(1) as i64)
        .saturating_add(
            elapsed_ticks
                .saturating_add(1)
                .saturating_mul(config.models as i64),
        )
        .saturating_add(8)
        .min(MAX_SFMT_FRAME)
}

fn search_v12_start_all_models(
    start_consumption: i64,
    raw_config: &FieldConfig,
    display_config: &FieldConfig,
    values: &SearchValues,
    all_models: bool,
    materialize_timeline: bool,
    cancel: Option<&AtomicBool>,
) -> Vec<FieldResult> {
    let mut status = ModelStatus::new(raw_config.models);
    let mut stream_cursor = start_consumption;
    let mut events_by_model = vec![Vec::<BlinkEvent>::new(); raw_config.models];
    let max_tick = raw_config
        .range_end
        .saturating_sub(raw_config.range_start)
        .max(0);
    for tick in 1..=max_tick.saturating_add(1) {
        if cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
            return Vec::new();
        }
        let Some((used, consumed)) =
            next_state_with_positions_values(&mut status, &mut stream_cursor, values)
        else {
            break;
        };
        let _ = used;
        for event in consumed.into_iter().filter(|event| event.blink) {
            if let Some(events) = events_by_model.get_mut(event.model) {
                events.push(BlinkEvent {
                    // 照合中は外部仕様と同じ1/30秒tickを保持する。表示Frame（tick×2）へ
                    // の変換は照合窓が一致した後に行う。
                    wait_frame: tick,
                    consumption: stream_cursor,
                    status: status.clone(),
                    stream_cursor,
                    model_consumption: Some(event.position),
                });
            }
        }
    }

    let mut results = Vec::new();
    // Target到達判定は間隔照合と分け、既知idxの一致候補ごとに後段で一度だけ行う。
    // idx不明の検索では、モデル未確定の候補へTargetを適用しない。
    let result_config = {
        let mut config = display_config.clone();
        config.target_consumption = None;
        config
    };
    let models = if all_models {
        0..raw_config.models
    } else {
        raw_config.target_model..raw_config.target_model.saturating_add(1)
    };
    for target_model in models {
        let Some(events) = events_by_model.get(target_model) else {
            continue;
        };
        for window in matching_event_windows(events, raw_config) {
            let display_window = window
                .iter()
                .map(|event| BlinkEvent {
                    wait_frame: event.wait_frame.saturating_mul(2),
                    consumption: event.consumption,
                    status: event.status.clone(),
                    stream_cursor: event.stream_cursor,
                    model_consumption: event.model_consumption,
                })
                .collect::<Vec<_>>();
            if let Some(result) = result_from_window(
                start_consumption,
                target_model,
                &result_config,
                &display_window,
                values,
                cancel,
                materialize_timeline,
            ) {
                results.push(result);
            }
        }
    }
    results
}

#[cfg(test)]
fn search_with_cache(
    config: &FieldConfig,
    cache: &ObservationCache,
) -> Result<Vec<FieldResult>, AppError> {
    search_with_cache_cancelable(config, cache, None)
}

#[cfg(test)]
fn search_with_cache_cancelable(
    config: &FieldConfig,
    cache: &ObservationCache,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<FieldResult>, AppError> {
    search_with_cache_and_index_cancelable(config, cache, None, cancel)
}

#[allow(dead_code)]
fn search_with_cache_and_index(
    config: &FieldConfig,
    cache: &ObservationCache,
    prepared: Option<&PreparedTimelineIndex>,
) -> Result<Vec<FieldResult>, AppError> {
    search_with_cache_and_index_cancelable(config, cache, prepared, None)
}

fn search_with_cache_and_index_cancelable(
    config: &FieldConfig,
    cache: &ObservationCache,
    _prepared: Option<&PreparedTimelineIndex>,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<FieldResult>, AppError> {
    validate(config)?;
    check_cancelled(cancel)?;
    // Timeline生成と間隔照合を開始位置ごとに分け、Target到達判定は一致候補へ
    // 後段で適用する。各チャンクで遠いTargetの後続列まで読むと、広い検索範囲で
    // 同じ大きな読み込みが重複するためである。
    let mut interval_config = config.clone();
    interval_config.target_consumption = None;
    // 事前索引は各Timelineの短い先頭部分だけを含む。通常経路の準備には使えるが、
    // 観測列が候補開始後の任意の瞬き（例えば5回目）から始まるため、除外フィルター
    // としては使わない。
    let mut chunks = Vec::new();
    let mut start = config.range_start;
    while start <= config.range_end {
        let end = start
            .saturating_add(SEARCH_CANDIDATE_CHUNK_STARTS.saturating_sub(1))
            .min(config.range_end);
        chunks.push((start, end));
        if end == config.range_end {
            break;
        }
        start = end.saturating_add(1);
    }

    // 開始位置の小さいチャンクから順番に処理する。チャンク内の開始位置
    // は並列化するが、チャンク自体を並列にすると遠い位置の完了が先に
    // なり、タイムライン仕様の「始点付近から探す」挙動と一致しない。
    // 最初の候補が見つかった位置から1000 SFMT Frameを確認窓として照合し、
    // その窓内だけを候補として返す。複数候補は次の瞬き入力で再照合する。
    let mut results = Vec::new();
    let mut confirm_end = None;
    for (chunk_start, chunk_end) in chunks {
        let chunk_results = {
            if cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
                return Err(AppError::Cancelled);
            }
            // 間隔照合に必要な最大5000 tick分だけをこのチャンクへ読む。
            let chunk_stream_end = chunk_end
                .saturating_add(MAX_WAIT_TICKS.saturating_mul(config.models as i64))
                .saturating_add(8)
                .min(MAX_SFMT_FRAME);
            let count = chunk_stream_end
                .saturating_sub(chunk_start)
                .saturating_add(1)
                .max(0) as usize;
            let values = cache.read_values(chunk_start, count)?;
            let values = SearchValues::new(chunk_start, values);
            // 先に開始位置ごとのTimelineを1回だけ走査し、その結果から
            // 各モデルの瞬き列へ観測間隔を照合する。モデルidxごとに
            // 同じ開始位置ではModelStatusを再初期化せず、1回の走査結果を共有する。
            (chunk_start..=chunk_end)
                .into_par_iter()
                .flat_map_iter(|start_consumption| {
                    search_start_all_models_cancelable(
                        start_consumption,
                        &interval_config,
                        &values,
                        cancel,
                    )
                    .into_iter()
                    .filter(|result| result.observed_model == config.target_model)
                })
                .collect::<Vec<_>>()
        };
        if confirm_end.is_none() && !chunk_results.is_empty() {
            let first = chunk_results
                .iter()
                .map(|result| result.start_consumption)
                .min()
                .expect("non-empty candidate chunk");
            confirm_end = Some(first.saturating_add(CANDIDATE_CONFIRM_SPAN));
        }
        if let Some(end) = confirm_end {
            results.extend(
                chunk_results
                    .into_iter()
                    .filter(|result| result.start_consumption <= end),
            );
            if chunk_end >= end {
                // 確認窓内に複数候補が残っていても、遠い開始位置へ探索を
                // 広げない。追加の瞬き入力で同じ窓を再照合して絞り込む。
                break;
            }
        } else {
            results.extend(chunk_results);
        }
    }
    check_cancelled(cancel)?;
    results.sort_by_key(|result| {
        (
            result.start_consumption,
            result.observed_model,
            result.first_wait_frame,
            result.last_wait_frame,
        )
    });

    let Some(target) = config.target_consumption else {
        return Ok(results);
    };

    // 候補ごとに保存済みの連続状態を再利用する。Target判定は独立して並列化し、
    // 最後の並べ替えで開始位置の早い順を維持する。遠いTargetでも5000万件の
    // SFMT後続列を一括で読み込まない。
    let mut targeted = results
        .into_par_iter()
        .map(|result| apply_target_from_cursor(result, config.seed, target, Some(cache), cancel))
        .collect::<AppResult<Vec<_>>>()?;
    targeted.sort_by_key(|result| {
        (
            result.start_consumption,
            result.observed_model,
            result.first_wait_frame,
            result.last_wait_frame,
        )
    });
    Ok(targeted)
}

#[cfg(test)]
fn generate_search_values(config: &FieldConfig) -> SearchValues {
    let stream_end = search_stream_end(config);
    let base = config.range_start;
    let count = stream_end.saturating_sub(base).saturating_add(1).max(0) as usize;
    SearchValues::new(
        base,
        generate_u64_values_from(config.seed, base as usize, count),
    )
}

fn search_stream_end(config: &FieldConfig) -> i64 {
    let legacy_end = config
        .range_end
        .max(config.target_consumption.unwrap_or(config.range_end))
        .saturating_add(MAX_WAIT_TICKS.saturating_mul(config.models as i64))
        .saturating_add(8);
    legacy_end
        .max(reference_search_stream_end(config))
        .min(MAX_SFMT_FRAME)
}

fn observation_stream_end(config: &FieldConfig) -> i64 {
    let mut observation_only = config.clone();
    observation_only.target_consumption = None;
    search_stream_end(&observation_only)
}

/// 観測対象idxが不明な場合の検索。
///
/// 各idxを全て仮定して間隔だけを照合する。Target到達判定は、観測対象idxが
/// 確定していないため行わない。
#[cfg(test)]
pub fn search_without_target_model(config: &FieldConfig) -> Result<Vec<FieldResult>, AppError> {
    let pool = prepare_search_pool(config)?;
    search_without_target_model_with_pool(config, &pool)
}

#[cfg(test)]
pub(crate) fn search_without_target_model_with_pool(
    config: &FieldConfig,
    pool: &FieldSearchPool,
) -> Result<Vec<FieldResult>, AppError> {
    search_without_target_model_with_pool_cancelable(config, pool, None)
}

#[cfg(test)]
fn search_without_target_model_with_pool_cancelable(
    config: &FieldConfig,
    pool: &FieldSearchPool,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<FieldResult>, AppError> {
    validate(config)?;
    check_cancelled(cancel)?;
    // idx不明時は、同じ開始位置のModelStatusをidxごとに再初期化せず、
    // 1回のTimeline走査で全モデルの瞬き列を同時に記録する。
    let mut chunks = Vec::new();
    let mut start = config.range_start;
    while start <= config.range_end {
        let end = start
            .saturating_add(SEARCH_CANDIDATE_CHUNK_STARTS.saturating_sub(1))
            .min(config.range_end);
        chunks.push((start, end));
        if end == config.range_end {
            break;
        }
        start = end.saturating_add(1);
    }
    let mut results = Vec::new();
    let mut confirm_end = None;
    for (chunk_start, chunk_end) in chunks {
        let chunk_results = {
            if cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
                return Err(AppError::Cancelled);
            }
            let stream_end = chunk_end
                .saturating_add(MAX_WAIT_TICKS.saturating_mul(config.models as i64))
                .saturating_add(8)
                .min(MAX_SFMT_FRAME);
            let count = stream_end
                .saturating_sub(chunk_start)
                .saturating_add(1)
                .max(0) as usize;
            let values = pool.cache.read_values(chunk_start, count)?;
            let values = std::sync::Arc::new(SearchValues::new(chunk_start, values));
            (chunk_start..=chunk_end)
                .into_par_iter()
                .flat_map(|start_consumption| {
                    search_start_all_models_cancelable(start_consumption, config, &values, cancel)
                })
                .collect::<Vec<_>>()
        };
        if confirm_end.is_none() && !chunk_results.is_empty() {
            let first = chunk_results
                .iter()
                .map(|result| result.start_consumption)
                .min()
                .expect("non-empty candidate chunk");
            confirm_end = Some(first.saturating_add(CANDIDATE_CONFIRM_SPAN));
        }
        if let Some(end) = confirm_end {
            results.extend(
                chunk_results
                    .into_iter()
                    .filter(|result| result.start_consumption <= end),
            );
            if chunk_end >= end {
                break;
            }
        } else {
            results.extend(chunk_results);
        }
    }
    results.sort_by_key(|result| {
        (
            result.start_consumption,
            result.observed_model,
            result.first_wait_frame,
            result.last_wait_frame,
        )
    });
    Ok(results)
}

#[cfg(test)]
#[allow(dead_code)]
fn search_with_prefix_filter_cancelable(
    config: &FieldConfig,
    cache: &ObservationCache,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<FieldResult>, AppError> {
    if config.observed_intervals.len() < PRECOMPUTED_TIMELINE_EVENTS {
        return search_with_cache_and_index_cancelable(config, cache, None, cancel);
    }
    let mut prefix = config.clone();
    prefix.target_consumption = None;
    prefix.observed_intervals.truncate(2);
    let mut chunks = Vec::new();
    let mut start = config.range_start;
    while start <= config.range_end {
        let end = start
            .saturating_add(SEARCH_CANDIDATE_CHUNK_STARTS.saturating_sub(1))
            .min(config.range_end);
        chunks.push((start, end));
        if end == config.range_end {
            break;
        }
        start = end.saturating_add(1);
    }

    let mut starts = Vec::new();
    for (chunk_start, chunk_end) in chunks {
        let chunk_starts = {
            if cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
                return Err(AppError::Cancelled);
            }
            let stream_end = chunk_end
                .saturating_add(MAX_WAIT_TICKS.saturating_mul(config.models as i64))
                .saturating_add(8)
                .min(MAX_SFMT_FRAME);
            let count = stream_end
                .saturating_sub(chunk_start)
                .saturating_add(1)
                .max(0) as usize;
            let values = cache.read_values(chunk_start, count)?;
            let values = SearchValues::new(chunk_start, values);
            (chunk_start..=chunk_end)
                .into_par_iter()
                .filter(|start_consumption| {
                    matches_observed_prefix_cancelable(*start_consumption, &prefix, &values, cancel)
                })
                .collect::<Vec<_>>()
        };
        starts.extend(chunk_starts);
    }
    check_cancelled(cancel)?;

    let results = starts
        .into_par_iter()
        .map(|start_consumption| {
            if cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
                return Ok(None);
            }
            let stream_end = start_consumption
                .saturating_add(MAX_WAIT_TICKS.saturating_mul(config.models as i64))
                .saturating_add(8)
                .min(MAX_SFMT_FRAME);
            let count = stream_end
                .saturating_sub(start_consumption)
                .saturating_add(1)
                .max(0) as usize;
            let values = cache.read_values(start_consumption, count)?;
            let values = SearchValues::new(start_consumption, values);
            Ok(search_candidate_cancelable(
                start_consumption,
                config,
                &values,
                cancel,
            ))
        })
        .collect::<Result<Vec<_>, AppError>>()?
        .into_iter()
        .flatten()
        .collect();
    check_cancelled(cancel)?;
    Ok(results)
}

#[cfg(test)]
#[allow(dead_code)]
fn matches_observed_prefix_cancelable(
    start_consumption: i64,
    config: &FieldConfig,
    values: &SearchValues,
    cancel: Option<&AtomicBool>,
) -> bool {
    let mut status = ModelStatus::new(config.models);
    let mut stream_cursor = start_consumption;
    let mut tick = 0_i64;
    let required_events = config.observed_intervals.len().saturating_add(1);
    let mut events = Vec::with_capacity(required_events);
    while tick < MAX_WAIT_TICKS {
        if cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
            return false;
        }
        let Some((_used, blink)) = next_state_with_values(&mut status, &mut stream_cursor, values)
        else {
            return false;
        };
        tick += 1;
        if blink.contains(&(config.target_model as u8)) {
            events.push(tick * 2);
        }
        if events.len() < required_events {
            continue;
        }
        let window_start = events.len().saturating_sub(required_events);
        let window = &events[window_start..];
        if config
            .observed_intervals
            .iter()
            .enumerate()
            .all(|(index, expected)| {
                let actual = window[index + 1] - window[index];
                (actual - expected).abs() <= config.tolerance
            })
        {
            return true;
        }
    }
    false
}

fn search_start_all_models_cancelable(
    start_consumption: i64,
    config: &FieldConfig,
    values: &SearchValues,
    cancel: Option<&AtomicBool>,
) -> Vec<FieldResult> {
    let mut status = ModelStatus::new(config.models);
    let mut stream_cursor = start_consumption;
    let mut consumption = start_consumption;
    let mut tick = 0_i64;
    // 設定範囲を候補の観測窓として使うため、終点に近い開始位置では最大先読みtickを
    // すべて確保する必要がない。幅0の範囲でも終点の候補を確認できるよう、最低1遷移
    // 分は保持する。
    let scan_end = config.range_end.max(start_consumption.saturating_add(1));
    let mut events_by_model = vec![Vec::<BlinkEvent>::new(); config.models];
    while tick < MAX_WAIT_TICKS && consumption <= scan_end {
        if cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
            return Vec::new();
        }
        let Some((used, blink)) = next_state_with_values(&mut status, &mut stream_cursor, values)
        else {
            break;
        };
        consumption += used as i64;
        tick += 1;
        if consumption > scan_end {
            break;
        }
        for model in blink {
            let Some(events) = events_by_model.get_mut(model as usize) else {
                continue;
            };
            events.push(BlinkEvent {
                wait_frame: tick * 2,
                consumption,
                status: status.clone(),
                stream_cursor,
                model_consumption: None,
            });
        }
    }

    let mut results = Vec::new();
    let mut interval_config = config.clone();
    interval_config.target_consumption = None;
    for (target_model, events) in events_by_model.iter().enumerate() {
        for window in matching_event_windows(events, config) {
            if let Some(result) = result_from_window(
                start_consumption,
                target_model,
                &interval_config,
                window,
                values,
                cancel,
                true,
            ) {
                results.push(result);
            }
        }
    }
    results
}

fn matching_event_windows<'a>(
    events: &'a [BlinkEvent],
    config: &FieldConfig,
) -> Vec<&'a [BlinkEvent]> {
    let required_events = config.observed_intervals.len().saturating_add(1);
    events
        .windows(required_events)
        .filter(|window| window_matches_observations(window, config))
        .take(MAX_MATCHING_WINDOWS_PER_START)
        .collect()
}

/// 実際に取得できたTargetFrameを使って、観測対象idxを総当たりする。
///
/// idxは0を主人公、1以上をNPCとして扱う。各idxで通常検索を行い、入力された
/// 実測Frameへ到達できる候補だけを残す。複数idxが残る場合は一意に決めず、全候補を
/// 呼び出し側へ返す。
#[cfg(test)]
#[allow(dead_code)]
pub fn infer_idx(config: &FieldConfig, actual_frame: i64) -> Result<FieldIdxInference, AppError> {
    infer_idx_cancelable(config, actual_frame, None)
}

#[cfg(test)]
#[allow(dead_code)]
fn infer_idx_cancelable(
    config: &FieldConfig,
    actual_frame: i64,
    cancel: Option<&AtomicBool>,
) -> Result<FieldIdxInference, AppError> {
    if actual_frame < 0 {
        return Err(AppError::NegativeValue(InputField::ActualFrame));
    }
    check_cancelled(cancel)?;
    let mut results = Vec::new();
    let mut values_config = config.clone();
    values_config.target_consumption = Some(actual_frame);
    let pool = prepare_search_pool_cancelable(&values_config, cancel)?;
    for idx in 0..config.models {
        check_cancelled(cancel)?;
        let mut indexed = config.clone();
        indexed.target_model = idx;
        indexed.target_consumption = Some(actual_frame);
        let candidates = search_with_cache_cancelable(&indexed, &pool.cache, cancel)?;
        if candidates
            .iter()
            .any(|candidate| candidate.target_exact == Some(true))
        {
            results.push((idx, candidates));
        }
    }
    Ok(FieldIdxInference {
        indices: results.iter().map(|(idx, _)| *idx).collect(),
        results,
        suggested_offset: None,
    })
}

/// NPC範囲検索時のidx推定。各モデル数を独立に調べ、idxだけを統合する。
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn infer_idx_for_configs(
    configs: &[FieldConfig],
    actual_frame: i64,
) -> Result<FieldIdxInference, AppError> {
    infer_idx_for_configs_cancelable(configs, actual_frame, None)
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn infer_idx_for_configs_cancelable(
    configs: &[FieldConfig],
    actual_frame: i64,
    cancel: Option<&AtomicBool>,
) -> Result<FieldIdxInference, AppError> {
    let mut indices = Vec::new();
    let mut results = Vec::new();
    for config in configs {
        check_cancelled(cancel)?;
        let inference = infer_idx_cancelable(config, actual_frame, cancel)?;
        for (idx, candidates) in inference.results {
            if !indices.contains(&idx) {
                indices.push(idx);
            }
            results.push((idx, candidates));
        }
    }
    indices.sort_unstable();
    Ok(FieldIdxInference {
        indices,
        results,
        suggested_offset: None,
    })
}

/// 一致済みTimelineから、実測Frameに対応する観測モデルを求める。
///
/// 各idxに対応する開始位置と初期SFMT値を揃え、観測時と本番時で共通の消費ステップを
/// 使って実測Frameへの到達可否を判定する。
pub(crate) fn infer_idx_from_result_with_cache_cancelable(
    seed: u32,
    result: &FieldResult,
    tolerance: i64,
    actual_frame: i64,
    cache: Option<&ObservationCache>,
    cancel: Option<&AtomicBool>,
) -> Result<FieldIdxInference, AppError> {
    if actual_frame < 0 {
        return Err(AppError::NegativeValue(InputField::ActualFrame));
    }
    check_cancelled(cancel)?;

    let models = result.models.max(1);
    let anchor_position = result
        .start_consumption
        .checked_add(result.observed_model as i64)
        .ok_or(AppError::ArithmeticOverflow(
            ArithmeticOperation::TimelinePosition,
        ))?;
    let anchor_value = sfmt_value_at_cached(seed, anchor_position, cache, cancel)?;
    let observed_intervals = result.intervals.clone();
    let anchor_start = result.start_consumption;
    let anchor_model = result.observed_model;

    // モデルの最初の乱数は`start + idx`で消費される。絶対位置を固定すれば、各idxに
    // 対応する近傍の開始位置（`start + anchor_idx - candidate_idx`）を求められる。
    // 対応するTimelineを観測区間数だけ進め、実測Frameへの到達を同じ消費規則で確認する。
    let indices = (0..models)
        .into_par_iter()
        .filter_map(|candidate_idx| {
            if cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
                return Some(Err(AppError::Cancelled));
            }
            let delta = anchor_model as i64 - candidate_idx as i64;
            let candidate_start = anchor_start.saturating_add(delta);
            // candidate_start + candidate_idxは全候補でanchor_positionと等しい。
            // 同じ値をSeed先頭から候補ごとに再生成しない。
            let candidate_value = anchor_value;
            if candidate_value != anchor_value {
                return None;
            }
            match timeline_reaches_frame(
                seed,
                candidate_start,
                models,
                candidate_idx,
                &observed_intervals,
                tolerance,
                actual_frame,
                cache,
                cancel,
            ) {
                Ok(true) => Some(Ok(candidate_idx)),
                Ok(false) => None,
                Err(message) => Some(Err(message)),
            }
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    let mut indices = indices;
    indices.sort_unstable();
    indices.dedup();
    Ok(FieldIdxInference {
        indices,
        results: Vec::new(),
        suggested_offset: None,
    })
}

/// 保持したTimelineからフィールドTargetタイマーの補正値を求める。
///
/// Targetと実測FrameはSFMT消費位置、補正値は表示Frameで入力する。可変長の消費を
/// 単純な数値差と誤認しないよう、同じ連続ModelStatusカーソル上で両位置を解決する。
/// Timelineの1tickは表示Frameの2Fに相当し、実測がTargetより1tick先なら補正は-2Fとなる。
pub(crate) fn suggest_field_offset_from_result_with_cache_cancelable(
    seed: u32,
    result: &FieldResult,
    target: i64,
    actual: i64,
    current_offset: i64,
    cache: Option<&ObservationCache>,
    cancel: Option<&AtomicBool>,
) -> Result<Option<i64>, AppError> {
    if target < 0 || actual < 0 {
        return Ok(None);
    }
    check_cancelled(cancel)?;
    let Some(target_tick) = find_correction_tick_from_cursor(seed, result, target, cache, cancel)?
    else {
        return Ok(None);
    };
    let Some(actual_tick) = find_correction_tick_from_cursor(seed, result, actual, cache, cancel)?
    else {
        return Ok(None);
    };
    let drift_ticks = actual_tick.saturating_sub(target_tick);
    Ok(Some(
        current_offset.saturating_sub(drift_ticks.saturating_mul(2)),
    ))
}

fn sfmt_value_at_cached(
    seed: u32,
    position: i64,
    cache: Option<&ObservationCache>,
    cancel: Option<&AtomicBool>,
) -> Result<u64, AppError> {
    check_cancelled(cancel)?;
    let value = match cache.filter(|cache| cache.seed == seed) {
        Some(cache) => cache.read_values(position, 1)?.first().copied(),
        None => None,
    };
    value.ok_or(AppError::Cache(CacheError::StateUnavailable))
}

#[allow(clippy::too_many_arguments)]
fn timeline_reaches_frame(
    seed: u32,
    start: i64,
    models: usize,
    target_model: usize,
    observed_intervals: &[i64],
    tolerance: i64,
    actual_frame: i64,
    cache: Option<&ObservationCache>,
    cancel: Option<&AtomicBool>,
) -> Result<bool, AppError> {
    if start < 0 || actual_frame < start || actual_frame > MAX_SFMT_FRAME || target_model >= models
    {
        return Ok(false);
    }
    let Some(mut sfmt) = sfmt_from_cache(seed, start, cache)? else {
        return Err(AppError::Cache(CacheError::StateUnavailable));
    };
    let mut status = ModelStatus::new(models);
    let required_events = observed_intervals.len().saturating_add(1);
    let mut blink_frames = Vec::with_capacity(required_events);
    let mut consumption = start;
    let mut tick = 0_i64;
    let mut matched = false;
    let mut idle_ticks = 0_usize;
    while consumption <= actual_frame {
        check_cancelled(cancel)?;
        let (used, blink) = status.next_state(&mut sfmt);
        if used == 0 {
            idle_ticks += 1;
            if idle_ticks > MAX_CONSECUTIVE_IDLE_TICKS {
                return Ok(false);
            }
        } else {
            idle_ticks = 0;
        }
        consumption = consumption.saturating_add(used as i64);
        tick = tick.saturating_add(1);
        if blink.contains(&(target_model as u8)) {
            blink_frames.push(tick.saturating_mul(2));
            if blink_frames.len() >= required_events {
                matched = blink_frames.windows(required_events).any(|window| {
                    observed_intervals
                        .iter()
                        .enumerate()
                        .all(|(index, expected)| {
                            (window[index + 1] - window[index] - expected).abs() <= tolerance
                        })
                });
            }
        }
        // 実測SFMTFrameは消費された値そのものを特定する。可変長ステップの途中で
        // 通過したFrameまで到達扱いにすると、idxをずらした候補まで有効になるため、
        // 消費境界だけを到達位置として扱う。
        if matched && actual_frame == consumption {
            return Ok(true);
        }
        if consumption > actual_frame {
            break;
        }
    }
    Ok(false)
}

fn sfmt_from_cache(
    seed: u32,
    position: i64,
    cache: Option<&ObservationCache>,
) -> AppResult<Option<Sfmt>> {
    let Some(cache) = cache.filter(|cache| cache.seed == seed) else {
        return Ok(None);
    };
    cache.sfmt_at_checked(position).map_err(AppError::from)
}

impl FieldResult {
    /// Clone only the data read or updated by the display-timeline extender.
    /// Candidate blink history is used by candidate matching, not by extension.
    pub(crate) fn timeline_extension_snapshot(&self) -> Self {
        Self {
            start_consumption: self.start_consumption,
            first_wait_frame: self.first_wait_frame,
            last_wait_frame: self.last_wait_frame,
            first_consumption: self.first_consumption,
            last_consumption: self.last_consumption,
            intervals: self.intervals.clone(),
            target_consumptions: self.target_consumptions.clone(),
            next_blink_wait_frame: self.next_blink_wait_frame,
            target_wait_frame: self.target_wait_frame,
            target_exact: self.target_exact,
            target_frame_before: self.target_frame_before,
            target_frame_after: self.target_frame_after,
            observed_model: self.observed_model,
            models: self.models,
            timeline: self.timeline.clone(),
            candidate_blinks: Vec::new(),
            target_blink_count: self.target_blink_count,
            target_blink_count_offset_from_first_row: self.target_blink_count_offset_from_first_row,
            timeline_cursor: self.timeline_cursor.clone(),
            target_cursor: self.target_cursor.clone(),
        }
    }

    pub(crate) fn timeleap_timeline_start(
        &self,
    ) -> Option<crate::domain::timeleap::TimeleapTimelineStart> {
        let cursor = self.target_cursor.as_ref()?;
        Some(crate::domain::timeleap::TimeleapTimelineStart {
            frame: cursor.consumption,
            sfmt_position: cursor.stream_cursor,
            tick: cursor.tick,
            model: cursor.status.clone(),
        })
    }

    /// 保持中の行番号からTargetまでに残る対象NPCの瞬き数を返す。
    ///
    /// `target_blink_count`は最後に観測した瞬きが基準で、オフセットにより
    /// 表示Timelineの先頭行を基準に直す。表示行を間引いても、読み込み済みの
    /// 行数だけでTargetまでの回数を過少計算しない。
    pub(crate) fn target_blinks_after_row(&self, row_index: usize) -> Option<usize> {
        let remaining = i64::try_from(self.target_blink_count?).ok()?;
        let row_index = i64::try_from(row_index).ok()?;
        usize::try_from(
            remaining
                .checked_add(self.target_blink_count_offset_from_first_row)?
                .checked_sub(row_index)?
                .max(0),
        )
        .ok()
    }

    /// 表示中の瞬き行から、指定SFMT Frameまでにある次の瞬き数を返す。
    /// Frame1が現在行より先にあり、その位置までの表示行が先読み済みの場合に使う。
    #[cfg(test)]
    pub(crate) fn blinks_through_frame_after_row(
        &self,
        row_index: usize,
        frame: i64,
    ) -> Option<usize> {
        let boundary = self.frame_boundary_index(frame)?;
        self.blinks_through_frame_after_row_at_boundary(row_index, frame, boundary)
    }

    pub(crate) fn frame_boundary_index(&self, frame: i64) -> Option<usize> {
        let final_frame = self.timeline.last()?.frame;
        (frame <= final_frame).then(|| self.timeline.partition_point(|row| row.frame <= frame))
    }

    pub(crate) fn blinks_through_frame_after_row_at_boundary(
        &self,
        row_index: usize,
        frame: i64,
        boundary: usize,
    ) -> Option<usize> {
        let current = self.timeline.get(row_index)?;
        Some(if frame <= current.frame {
            0
        } else {
            boundary.saturating_sub(row_index.saturating_add(1))
        })
    }
}

#[cfg(test)]
fn generate_u64_stream(seed: u32, last_index: usize) -> Vec<u64> {
    crate::domain::rng::generate_u64_values(seed, last_index.saturating_add(1))
}

#[cfg(test)]
fn search_candidate(
    start_consumption: i64,
    config: &FieldConfig,
    values: &SearchValues,
) -> Option<FieldResult> {
    search_candidate_cancelable(start_consumption, config, values, None)
}

#[cfg(test)]
fn search_candidate_cancelable(
    start_consumption: i64,
    config: &FieldConfig,
    values: &SearchValues,
    cancel: Option<&AtomicBool>,
) -> Option<FieldResult> {
    search_candidate_all_with_mode_cancelable(start_consumption, config, values, cancel, true)
        .into_iter()
        .next()
}

/// 1つの候補開始位置に対する観測間隔の一致窓をすべて返す。
///
/// 同じ開始位置・モデルに同一の間隔列を持つ窓が複数存在する場合がある。
/// それぞれ別候補として扱い、最初の一致だけに縮約しない。
#[cfg(test)]
#[allow(dead_code)]
fn search_candidate_all_cancelable(
    start_consumption: i64,
    config: &FieldConfig,
    values: &SearchValues,
    cancel: Option<&AtomicBool>,
) -> Vec<FieldResult> {
    search_candidate_all_with_mode_cancelable(start_consumption, config, values, cancel, true)
}

#[cfg(test)]
fn search_candidate_all_with_mode_cancelable(
    start_consumption: i64,
    config: &FieldConfig,
    values: &SearchValues,
    cancel: Option<&AtomicBool>,
    materialize_timeline: bool,
) -> Vec<FieldResult> {
    let mut status = ModelStatus::new(config.models);
    let mut stream_cursor = start_consumption;
    let mut consumption = start_consumption;
    let mut tick = 0_i64;
    // 設定した検索窓の内側だけが観測間隔列を説明できる。終点に近い開始位置では
    // 同じ固定先読みを余分にシミュレーションしない。
    let scan_end = config.range_end.max(start_consumption.saturating_add(1));
    let required_events = config.observed_intervals.len().saturating_add(1);
    let mut events = Vec::new();
    let mut results = Vec::new();

    while tick < MAX_WAIT_TICKS && consumption <= scan_end {
        if cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
            return Vec::new();
        }
        let Some((used, target_blinked)) = next_state_target_with_values(
            &mut status,
            config.target_model,
            &mut stream_cursor,
            values,
        ) else {
            break;
        };
        consumption += used as i64;
        tick += 1;

        if consumption <= scan_end && target_blinked {
            events.push(BlinkEvent {
                // 外部ツール/タイムライン仕様の表示Frameは1/30秒tickの2倍。
                wait_frame: tick * 2,
                consumption,
                status: status.clone(),
                stream_cursor,
                model_consumption: None,
            });
        }

        if events.len() < required_events {
            continue;
        }
        let window_start = events.len().saturating_sub(required_events);
        let window = &events[window_start..];
        if window_matches_observations(window, config)
            && results.len() < MAX_MATCHING_WINDOWS_PER_START
        {
            if let Some(result) = result_from_window(
                start_consumption,
                config.target_model,
                config,
                window,
                values,
                cancel,
                materialize_timeline,
            ) {
                results.push(result);
            }
        }
    }
    results
}

fn window_matches_observations(window: &[BlinkEvent], config: &FieldConfig) -> bool {
    config
        .observed_intervals
        .iter()
        .enumerate()
        .all(|(index, expected)| {
            let actual = window[index + 1].wait_frame - window[index].wait_frame;
            (actual - expected).abs() <= config.tolerance
        })
}

fn result_from_window(
    start_consumption: i64,
    target_model: usize,
    config: &FieldConfig,
    window: &[BlinkEvent],
    values: &SearchValues,
    cancel: Option<&AtomicBool>,
    materialize_timeline: bool,
) -> Option<FieldResult> {
    let first = &window[0];
    let last = window.last().expect("matching window is non-empty");
    let next_blink_wait_frame = if materialize_timeline {
        find_next_blink_cancelable(
            &last.status,
            last.stream_cursor,
            last.wait_frame / 2,
            last.wait_frame,
            target_model,
            values,
            cancel,
        )?
    } else {
        0
    };
    let target_timing = config
        .target_consumption
        .and_then(|target| find_target_timing_cancelable(last, target, values, cancel));
    let target_wait_frame =
        target_timing.map(|timing| timing.absolute_wait_frame - last.wait_frame);
    let (timeline, candidate_blinks, timeline_cursor, target_cursor) = if materialize_timeline {
        let target_cursor = TimelineCursor {
            status: last.status.clone(),
            stream_cursor: last.stream_cursor,
            consumption: last.consumption,
            tick: last.wait_frame / 2,
            timeleap_runtime: None,
        };
        let (timeline, candidate_blinks, timeline_cursor) = build_timeline(
            window,
            target_model,
            target_timing.map(|timing| timing.absolute_wait_frame),
            values,
        );
        (
            timeline,
            candidate_blinks,
            timeline_cursor,
            Some(target_cursor),
        )
    } else {
        let timeline = window
            .iter()
            .map(|event| FieldTimelineRow {
                frame: event.consumption,
                timeline_frame: event.wait_frame,
                duration_frames: 0,
            })
            .collect::<Vec<_>>();
        let candidate_blinks = window
            .iter()
            .map(|event| CandidateBlink {
                frame: event.consumption,
                timeline_frame: event.wait_frame,
                observed: true,
                target_consumption: event.model_consumption,
            })
            .collect::<Vec<_>>();
        let cursor = TimelineCursor {
            status: last.status.clone(),
            stream_cursor: last.stream_cursor,
            consumption: last.consumption,
            tick: last.wait_frame / 2,
            timeleap_runtime: None,
        };
        (timeline, candidate_blinks, cursor.clone(), Some(cursor))
    };
    let target_blink_count = target_timing
        .filter(|_| materialize_timeline)
        .map(|timing| {
            candidate_blinks
                .iter()
                .filter(|row| {
                    row.timeline_frame > last.wait_frame
                        && row.timeline_frame <= timing.absolute_wait_frame
                })
                .count()
        });
    Some(FieldResult {
        start_consumption,
        first_wait_frame: first.wait_frame,
        last_wait_frame: last.wait_frame,
        first_consumption: first.consumption,
        last_consumption: last.consumption,
        intervals: window
            .windows(2)
            .map(|pair| pair[1].wait_frame - pair[0].wait_frame)
            .collect(),
        target_consumptions: window
            .iter()
            .map(|event| event.model_consumption.unwrap_or(event.consumption))
            .collect(),
        next_blink_wait_frame,
        target_wait_frame,
        target_exact: target_timing.map(|timing| timing.exact),
        target_frame_before: target_timing.map(|timing| timing.frame_before),
        target_frame_after: target_timing.map(|timing| timing.frame_after),
        observed_model: target_model,
        models: config.models,
        timeline,
        candidate_blinks,
        target_blink_count,
        target_blink_count_offset_from_first_row: window.len().saturating_sub(1) as i64,
        timeline_cursor: Some(timeline_cursor),
        target_cursor,
    })
}

fn build_timeline(
    observed_events: &[BlinkEvent],
    target_model: usize,
    target_absolute_wait_frame: Option<i64>,
    values: &SearchValues,
) -> (Vec<FieldTimelineRow>, Vec<CandidateBlink>, TimelineCursor) {
    let event = observed_events
        .last()
        .expect("observed events are non-empty");
    let mut status = event.status.clone();
    let mut stream_cursor = event.stream_cursor;
    let mut tick = event.wait_frame / 2;
    let mut consumption = event.consumption;
    let mut next_events = Vec::new();

    // Targetが遠くても初回表示分だけを先読みする。到達時刻の判定は別の
    // カーソルで完了しており、表示TimelineをTargetまで展開する必要はない。
    let display_capacity = MAX_RETAINED_TIMELINE_ROWS.saturating_sub(observed_events.len());
    let event_limit = if target_absolute_wait_frame.is_some() {
        display_capacity
    } else {
        display_capacity.min(MAX_CANDIDATE_TIMELINE_EVENTS)
    };
    while tick < MAX_WAIT_TICKS && next_events.len() < event_limit {
        let Some((used, consumed)) =
            next_state_with_positions_values(&mut status, &mut stream_cursor, values)
        else {
            break;
        };
        consumption += used as i64;
        tick += 1;
        if let Some(target_event) = consumed
            .into_iter()
            .find(|event| event.model == target_model && event.blink)
        {
            let wait_frame = tick * 2;
            next_events.push((wait_frame, consumption, target_event.position));
            if target_absolute_wait_frame.is_some_and(|target| wait_frame >= target) {
                break;
            }
        }
    }

    let mut all_events: Vec<(i64, i64, bool, Option<i64>)> = observed_events
        .iter()
        .map(|event| {
            (
                event.wait_frame,
                event.consumption,
                true,
                event.model_consumption,
            )
        })
        .collect();
    all_events.extend(next_events.into_iter().map(
        |(wait_frame, consumption, target_consumption)| {
            (wait_frame, consumption, false, Some(target_consumption))
        },
    ));
    let first_display_event = all_events.len().saturating_sub(MAX_RETAINED_TIMELINE_ROWS);
    let candidate_blinks = all_events
        .iter()
        .map(
            |(timeline_frame, frame, observed, target_consumption)| CandidateBlink {
                frame: *frame,
                timeline_frame: *timeline_frame,
                observed: *observed,
                target_consumption: *target_consumption,
            },
        )
        .collect();
    let mut rows = Vec::with_capacity(all_events.len().saturating_sub(first_display_event));
    for (index, (wait_frame, consumption, _, _)) in all_events
        .iter()
        .copied()
        .enumerate()
        .skip(first_display_event)
    {
        let duration_frames = all_events
            .get(index + 1)
            .map(|(next_wait, _, _, _)| *next_wait - wait_frame)
            .unwrap_or_default();
        rows.push(FieldTimelineRow {
            frame: consumption,
            timeline_frame: wait_frame,
            duration_frames,
        });
    }
    (
        rows,
        candidate_blinks,
        TimelineCursor {
            status,
            stream_cursor,
            consumption,
            tick,
            timeleap_runtime: None,
        },
    )
}

#[cfg(test)]
#[allow(dead_code)]
fn find_next_blink(
    status: &ModelStatus,
    stream_cursor: i64,
    start_tick: i64,
    last_wait_frame: i64,
    target_model: usize,
    values: &SearchValues,
) -> Option<i64> {
    find_next_blink_cancelable(
        status,
        stream_cursor,
        start_tick,
        last_wait_frame,
        target_model,
        values,
        None,
    )
}

fn find_next_blink_cancelable(
    status: &ModelStatus,
    stream_cursor: i64,
    start_tick: i64,
    last_wait_frame: i64,
    target_model: usize,
    values: &SearchValues,
    cancel: Option<&AtomicBool>,
) -> Option<i64> {
    let mut status = status.clone();
    let mut stream_cursor = stream_cursor;
    let mut tick = start_tick;
    while tick < MAX_WAIT_TICKS {
        if cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
            return None;
        }
        let (used, blink) = next_state_with_values(&mut status, &mut stream_cursor, values)?;
        tick += 1;
        if blink.contains(&(target_model as u8)) {
            return Some(tick * 2 - last_wait_frame);
        }
        if used == 0 && tick == start_tick {
            break;
        }
    }
    None
}

#[cfg(test)]
fn find_target_timing(
    event: &BlinkEvent,
    target: i64,
    values: &SearchValues,
) -> Option<TargetTiming> {
    find_target_timing_cancelable(event, target, values, None)
}

fn find_target_timing_cancelable(
    event: &BlinkEvent,
    target: i64,
    values: &SearchValues,
    cancel: Option<&AtomicBool>,
) -> Option<TargetTiming> {
    if event.consumption > target || target > MAX_SFMT_FRAME {
        return None;
    }
    if event.consumption == target {
        return Some(TargetTiming {
            absolute_wait_frame: event.wait_frame,
            exact: true,
            frame_before: target,
            frame_after: target,
        });
    }

    let mut status = event.status.clone();
    let mut stream_cursor = event.stream_cursor;
    let mut consumption = event.consumption;
    let mut tick = event.wait_frame / 2;
    let mut idle_ticks = 0_usize;
    while consumption <= target {
        if cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
            return None;
        }
        let (used, _) = next_state_with_values(&mut status, &mut stream_cursor, values)?;
        if used == 0 {
            idle_ticks += 1;
            if idle_ticks > MAX_CONSECUTIVE_IDLE_TICKS {
                return None;
            }
        } else {
            idle_ticks = 0;
        }
        let before = consumption;
        consumption += used as i64;
        if before <= target && target < consumption {
            tick += 1;
            return Some(TargetTiming {
                absolute_wait_frame: tick * 2,
                exact: false,
                frame_before: before,
                frame_after: consumption,
            });
        }
        tick += 1;
        if consumption == target {
            return Some(TargetTiming {
                absolute_wait_frame: tick * 2,
                exact: true,
                frame_before: target,
                frame_after: target,
            });
        }
    }
    None
}

/// 一致済み候補の連続Timelineを遠いTargetまで延長する。
/// 観測からTargetまでのSFMT値をすべて展開せず、最後に観測した瞬き時点の
/// ModelStatus、remain、絶対SFMT位置を保持したカーソルから続けて計算する。
fn find_target_timing_from_cursor(
    seed: u32,
    event: &FieldResult,
    target: i64,
    cache: Option<&ObservationCache>,
    cancel: Option<&AtomicBool>,
) -> AppResult<Option<(TargetTiming, usize)>> {
    if event.last_consumption > target || target > MAX_SFMT_FRAME {
        return Ok(None);
    }
    if event.last_consumption == target {
        return Ok(Some((
            TargetTiming {
                absolute_wait_frame: event.last_wait_frame,
                exact: true,
                frame_before: target,
                frame_after: target,
            },
            0,
        )));
    }
    let cursor = event
        .target_cursor
        .as_ref()
        .or(event.timeline_cursor.as_ref())
        .ok_or(AppError::InvalidState(
            crate::errors::StateIssue::TimelineCursorUnavailable,
        ))?;
    let Some(mut sfmt) = sfmt_from_cache(seed, cursor.stream_cursor, cache)? else {
        return Ok(None);
    };
    let mut status = cursor.status.clone();
    let mut consumption = event.last_consumption;
    let mut tick = cursor.tick.max(event.last_wait_frame / 2);
    let mut blink_count = 0;
    let mut idle_ticks = 0_usize;
    while consumption <= target {
        if cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
            return Err(AppError::Cancelled);
        }
        let (used, blink) = status.next_state(&mut sfmt);
        if used == 0 {
            idle_ticks += 1;
            if idle_ticks > MAX_CONSECUTIVE_IDLE_TICKS {
                return Ok(None);
            }
        } else {
            idle_ticks = 0;
        }
        let before = consumption;
        consumption = consumption
            .checked_add(used as i64)
            .ok_or(AppError::ArithmeticOverflow(
                ArithmeticOperation::TimelinePosition,
            ))?;
        tick += 1;
        if before <= target && target < consumption {
            return Ok(Some((
                TargetTiming {
                    absolute_wait_frame: tick * 2,
                    exact: false,
                    frame_before: before,
                    frame_after: consumption,
                },
                blink_count,
            )));
        }
        if consumption == target {
            return Ok(Some((
                TargetTiming {
                    absolute_wait_frame: tick * 2,
                    exact: true,
                    frame_before: target,
                    frame_after: target,
                },
                blink_count,
            )));
        }
        if blink.contains(&(event.observed_model as u8)) {
            blink_count += 1;
        }
    }
    Ok(None)
}

/// 現在のTimelineカーソルから、指定Frameに初めて到達するtickを求める。
fn find_correction_tick_from_cursor(
    seed: u32,
    event: &FieldResult,
    target: i64,
    cache: Option<&ObservationCache>,
    cancel: Option<&AtomicBool>,
) -> AppResult<Option<i64>> {
    if event.last_consumption > target || target > MAX_SFMT_FRAME {
        return Ok(None);
    }
    if event.last_consumption == target {
        return Ok(Some(event.last_wait_frame.div_euclid(2)));
    }
    let cursor = event
        .target_cursor
        .as_ref()
        .or(event.timeline_cursor.as_ref())
        .ok_or(AppError::InvalidState(
            crate::errors::StateIssue::TimelineCursorUnavailable,
        ))?;
    let Some(mut sfmt) = sfmt_from_cache(seed, cursor.stream_cursor, cache)? else {
        return Ok(None);
    };
    let mut status = cursor.status.clone();
    let mut consumption = event.last_consumption;
    let mut tick = cursor.tick.max(event.last_wait_frame / 2);
    let mut idle_ticks = 0_usize;
    while consumption < target {
        check_cancelled(cancel)?;
        let (used, _) = status.next_state(&mut sfmt);
        if used == 0 {
            idle_ticks += 1;
            if idle_ticks > MAX_CONSECUTIVE_IDLE_TICKS {
                return Ok(None);
            }
        } else {
            idle_ticks = 0;
        }
        consumption = consumption
            .checked_add(used as i64)
            .ok_or(AppError::ArithmeticOverflow(
                ArithmeticOperation::TimelinePosition,
            ))?;
        tick += 1;
        if consumption >= target {
            return Ok(Some(tick));
        }
    }
    Ok(Some(tick))
}

/// 間隔一致した候補へTarget到達判定を付加する。
/// Targetへ到達できない場合も通常の瞬きタイマーを継続できるよう候補は残し、
/// Target関連の項目だけを利用不可またはfalseにする。
fn apply_target_from_cursor(
    mut result: FieldResult,
    seed: u32,
    target: i64,
    cache: Option<&ObservationCache>,
    cancel: Option<&AtomicBool>,
) -> AppResult<FieldResult> {
    if target < result.start_consumption || target < result.last_consumption {
        result.target_wait_frame = None;
        result.target_exact = Some(false);
        result.target_frame_before = Some(target);
        result.target_frame_after = Some(result.last_consumption);
        result.target_blink_count = None;
        return Ok(result);
    }
    let Some((timing, blink_count)) =
        find_target_timing_from_cursor(seed, &result, target, cache, cancel)?
    else {
        result.target_wait_frame = None;
        result.target_exact = Some(false);
        result.target_frame_before = None;
        result.target_frame_after = None;
        result.target_blink_count = None;
        return Ok(result);
    };
    result.target_wait_frame = Some(timing.absolute_wait_frame - result.last_wait_frame);
    result.target_exact = Some(timing.exact);
    result.target_frame_before = Some(timing.frame_before);
    result.target_frame_after = Some(timing.frame_after);
    result.target_blink_count = Some(blink_count);
    Ok(result)
}

/// A confirmed candidate can be evaluated for a new Target without repeating blink matching.
#[cfg(test)]
pub(crate) fn retarget_confirmed_result(
    result: FieldResult,
    seed: u32,
    target: i64,
    cache: &ObservationCache,
    cancel: Option<&AtomicBool>,
) -> AppResult<FieldResult> {
    apply_target_from_cursor(result, seed, target, Some(cache), cancel)
}

/// Apply a Target query answered by the resolved Timeline's shared progression cache.
pub(crate) fn apply_cached_target_timing(
    mut result: FieldResult,
    target: i64,
    timing: Option<(i64, bool, i64, i64, usize)>,
) -> FieldResult {
    let Some((absolute_wait_frame, exact, frame_before, frame_after, blink_count)) = timing else {
        result.target_wait_frame = None;
        result.target_exact = Some(false);
        result.target_frame_before = None;
        result.target_frame_after = None;
        result.target_blink_count = None;
        return result;
    };
    result.target_wait_frame = Some(absolute_wait_frame.saturating_sub(result.last_wait_frame));
    result.target_exact = Some(exact);
    result.target_frame_before = Some(frame_before);
    result.target_frame_after = Some(frame_after);
    result.target_blink_count = Some(blink_count);
    if target < result.last_consumption {
        result.target_wait_frame = None;
        result.target_exact = Some(false);
        result.target_frame_before = Some(target);
        result.target_frame_after = Some(result.last_consumption);
        result.target_blink_count = None;
    }
    result
}

fn validate(config: &FieldConfig) -> Result<(), AppError> {
    validate_static_config(config)?;
    if config.observed_intervals.is_empty() {
        return Err(AppError::MissingObservation);
    }
    if config.observed_intervals.len() >= crate::application::state::MAX_BLINK_OBSERVATIONS {
        return Err(AppError::TooManyObservations {
            max: crate::application::state::MAX_BLINK_OBSERVATIONS,
        });
    }
    if config.observed_intervals.iter().any(|value| *value < 0) {
        return Err(AppError::InvalidObservation);
    }
    Ok(())
}

fn validate_static_config(config: &FieldConfig) -> Result<(), AppError> {
    if config.range_start < 0
        || config.range_end < config.range_start
        || config.range_end > MAX_SEARCH_FRAME
        || config.models == 0
        || config.models > MAX_MODELS
        || config.target_model >= config.models
        || config.tolerance < 0
    {
        return Err(AppError::InvalidFieldConfiguration);
    }
    if config.range_end.saturating_sub(config.range_start) > MAX_SEARCH_SPAN {
        return Err(AppError::SearchSpanTooWide {
            max: MAX_SEARCH_SPAN,
        });
    }
    if config
        .target_consumption
        .is_some_and(|target| target > MAX_SEARCH_FRAME)
    {
        return Err(AppError::OutOfRange {
            field: InputField::TargetConsumption,
            min: 0,
            max: MAX_SEARCH_FRAME,
        });
    }
    Ok(())
}

fn parse_npc_counts(input: &str, multiple: bool) -> Result<Vec<usize>, AppError> {
    let value = input.trim();
    if !multiple {
        let count = usize::try_from(state::parse_nonnegative(value, InputField::NpcCount)?)
            .map_err(|_| AppError::NpcCountTooLarge)?;
        if count > MAX_NPC_COUNT {
            return Err(AppError::OutOfRange {
                field: InputField::NpcCount,
                min: 0,
                max: MAX_NPC_COUNT as i64,
            });
        }
        return Ok(vec![count]);
    }
    let mut parts = value.split(['~', '～']);
    let start = parts
        .next()
        .filter(|part| !part.trim().is_empty())
        .ok_or(AppError::InvalidNpcRange)?;
    let end = parts
        .next()
        .filter(|part| !part.trim().is_empty())
        .ok_or(AppError::InvalidNpcRange)?;
    if parts.next().is_some() {
        return Err(AppError::InvalidNpcRange);
    }
    let start = usize::try_from(state::parse_nonnegative(start, InputField::NpcCount)?)
        .map_err(|_| AppError::NpcCountTooLarge)?;
    let end = usize::try_from(state::parse_nonnegative(end, InputField::NpcCount)?)
        .map_err(|_| AppError::NpcCountTooLarge)?;
    if start > MAX_NPC_COUNT || end > MAX_NPC_COUNT {
        return Err(AppError::OutOfRange {
            field: InputField::NpcCount,
            min: 0,
            max: MAX_NPC_COUNT as i64,
        });
    }
    if end < start {
        return Err(AppError::InvalidNpcRange);
    }
    let width = end.saturating_sub(start);
    if width > MAX_NPC_RANGE_WIDTH {
        return Err(AppError::InvalidNpcRange);
    }
    Ok((start..=end).collect())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
