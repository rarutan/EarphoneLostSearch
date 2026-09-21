//! フィールド観測の検索、Timeline生成、到達判定を提供する。
//!
//! 検索範囲と観測列の照合は純粋な計算として実行し、UI層から渡された
//! スナップショットだけを使って候補と本番用Timelineを生成する。

#[cfg(test)]
use crate::domain::rng::generate_u64_values_from;
use crate::domain::rng::{ModelConsumption, ModelStatus, ObservationCache, Sfmt, MAX_SFMT_FRAME};
use crate::domain::timing::{game_frames_to_seconds, seconds_to_game_frames};
use rayon::prelude::*;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

#[path = "field_state.rs"]
mod field_state;
pub(crate) use field_state::*;

const MAX_SEARCH_FRAME: i64 = MAX_SFMT_FRAME;
/// タイムライン仕様の1回の検索で許可するSFMT消費範囲の幅。
pub const MAX_SEARCH_SPAN: i64 = 100_000;
const MIN_TIMELINE_FRAME: i64 = 477;
// 1/30秒tickを表示Frameへ直すと2倍になる。検索の先読みは十分な余裕を持たせる。
const MAX_WAIT_TICKS: i64 = 5_000;
const MAX_TARGET_TICKS: i64 = 10_000_000;
/// 一意候補のTimelineを遅延延長する1/30秒単位のチャンクサイズ。
const TIMELINE_CHUNK_TICKS: i64 = 4_096;
// 起動直後のTimelineウォームアップは、全検索範囲を先行走査しない。
// 観測入力までに間に合う小さな先頭チャンクだけを作り、残りは詳細検索へ委ねる。
#[allow(dead_code)]
const TIMELINE_WARMUP_STARTS: i64 = 1_024;
#[allow(dead_code)]
const TIMELINE_WARMUP_TICKS: i64 = 1_024;
/// 未来のTimelineを延長できるSFMT絶対位置の上限。
/// 候補開始位置を検索する範囲の上限とは別に扱う。
const MAX_FUTURE_TIMELINE_TICKS: i64 = MAX_SFMT_FRAME;
const MAX_MODELS: usize = 256;
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
const SEARCH_CANCELLED: &str = "検索をキャンセルしました。";

// ---------------------------------------------------------------------------
// 検索ドメイン型とTimeline索引の構築
// ---------------------------------------------------------------------------

fn check_cancelled(cancel: Option<&AtomicBool>) -> Result<(), String> {
    if cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
        Err(SEARCH_CANCELLED.into())
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
    /// Targetまでに見込まれる瞬き回数。Target未設定または到達不能ならNone。
    pub target_blink_count: Option<usize>,
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
    /// 実測観測ですでに通過した瞬き行。
    pub observed: bool,
    /// この行の瞬きで観測対象モデルが参照したSFMT位置。
    pub target_consumption: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TimelineCursor {
    status: ModelStatus,
    stream_cursor: i64,
    consumption: i64,
    tick: i64,
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
pub fn search(config: &FieldConfig) -> Result<Vec<FieldResult>, String> {
    let pool = prepare_search_pool(config)?;
    search_with_cache(config, &pool.cache)
}

/// 観測開始時に先行生成する検索用SFMTプール。
#[cfg(test)]
pub(crate) fn prepare_search_pool(config: &FieldConfig) -> Result<FieldSearchPool, String> {
    prepare_search_pool_cancelable(config, None)
}

fn prepare_search_pool_cancelable(
    config: &FieldConfig,
    cancel: Option<&AtomicBool>,
) -> Result<FieldSearchPool, String> {
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
) -> Result<FieldSearchPool, String> {
    prepare_search_pool_with_configs_cancelable(configs, None)
}

pub(crate) fn prepare_search_pool_with_configs_cancelable(
    configs: &[FieldConfig],
    cancel: Option<&AtomicBool>,
) -> Result<FieldSearchPool, String> {
    let Some(first) = configs.first() else {
        return Err("NPC範囲を確認してください。".into());
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
) -> Result<PreparedTimelineIndex, String> {
    prepare_timeline_index_cancelable(cache, configs, None)
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn prepare_timeline_index_cancelable(
    cache: &ObservationCache,
    configs: &[FieldConfig],
    cancel: Option<&AtomicBool>,
) -> Result<PreparedTimelineIndex, String> {
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
) -> Result<PreparedTimelineIndex, String> {
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
) -> Result<PreparedTimelineIndex, String> {
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
pub fn search_with_preferred_idx(config: &FieldConfig) -> Result<FieldSearchOutcome, String> {
    let pool = prepare_search_pool(config)?;
    search_with_preferred_idx_with_pool(config, &pool)
}

#[cfg(test)]
pub(crate) fn search_with_preferred_idx_with_pool(
    config: &FieldConfig,
    pool: &FieldSearchPool,
) -> Result<FieldSearchOutcome, String> {
    search_with_preferred_idx_with_pool_cancelable(config, pool, None)
}

#[cfg(test)]
fn search_with_preferred_idx_with_pool_cancelable(
    config: &FieldConfig,
    pool: &FieldSearchPool,
    cancel: Option<&AtomicBool>,
) -> Result<FieldSearchOutcome, String> {
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
) -> Result<FieldSearchOutcome, String> {
    search_with_npc_range_with_pool_cancelable(configs, pool, target_mode, None)
}

pub(crate) fn search_with_npc_range_with_pool_cancelable(
    configs: &[FieldConfig],
    pool: &FieldSearchPool,
    _target_mode: FieldTargetMode,
    cancel: Option<&AtomicBool>,
) -> Result<FieldSearchOutcome, String> {
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
) -> Result<FieldSearchOutcome, String> {
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
        .collect::<Result<Vec<_>, String>>()?;
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
) -> Result<Vec<FieldResult>, String> {
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
        .map(|result| apply_target_from_cursor(result, config.seed, target, cancel))
        .collect::<Vec<_>>();
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
) -> Result<Vec<FieldResult>, String> {
    search_with_cache_cancelable(config, cache, None)
}

#[cfg(test)]
fn search_with_cache_cancelable(
    config: &FieldConfig,
    cache: &ObservationCache,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<FieldResult>, String> {
    search_with_cache_and_index_cancelable(config, cache, None, cancel)
}

#[allow(dead_code)]
fn search_with_cache_and_index(
    config: &FieldConfig,
    cache: &ObservationCache,
    prepared: Option<&PreparedTimelineIndex>,
) -> Result<Vec<FieldResult>, String> {
    search_with_cache_and_index_cancelable(config, cache, prepared, None)
}

fn search_with_cache_and_index_cancelable(
    config: &FieldConfig,
    cache: &ObservationCache,
    _prepared: Option<&PreparedTimelineIndex>,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<FieldResult>, String> {
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
                return Err(SEARCH_CANCELLED.into());
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
            let Ok(values) = cache.read_values(chunk_start, count) else {
                continue;
            };
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
        .map(|result| apply_target_from_cursor(result, config.seed, target, cancel))
        .collect::<Vec<_>>();
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
pub fn search_without_target_model(config: &FieldConfig) -> Result<Vec<FieldResult>, String> {
    let pool = prepare_search_pool(config)?;
    search_without_target_model_with_pool(config, &pool)
}

#[cfg(test)]
pub(crate) fn search_without_target_model_with_pool(
    config: &FieldConfig,
    pool: &FieldSearchPool,
) -> Result<Vec<FieldResult>, String> {
    search_without_target_model_with_pool_cancelable(config, pool, None)
}

#[cfg(test)]
fn search_without_target_model_with_pool_cancelable(
    config: &FieldConfig,
    pool: &FieldSearchPool,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<FieldResult>, String> {
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
                return Err(SEARCH_CANCELLED.into());
            }
            let stream_end = chunk_end
                .saturating_add(MAX_WAIT_TICKS.saturating_mul(config.models as i64))
                .saturating_add(8)
                .min(MAX_SFMT_FRAME);
            let count = stream_end
                .saturating_sub(chunk_start)
                .saturating_add(1)
                .max(0) as usize;
            let Ok(values) = pool.cache.read_values(chunk_start, count) else {
                continue;
            };
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
) -> Result<Vec<FieldResult>, String> {
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
                return Err(SEARCH_CANCELLED.into());
            }
            let stream_end = chunk_end
                .saturating_add(MAX_WAIT_TICKS.saturating_mul(config.models as i64))
                .saturating_add(8)
                .min(MAX_SFMT_FRAME);
            let count = stream_end
                .saturating_sub(chunk_start)
                .saturating_add(1)
                .max(0) as usize;
            let Ok(values) = cache.read_values(chunk_start, count) else {
                continue;
            };
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
        .filter_map(|start_consumption| {
            if cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
                return None;
            }
            let stream_end = start_consumption
                .saturating_add(MAX_WAIT_TICKS.saturating_mul(config.models as i64))
                .saturating_add(8)
                .min(MAX_SFMT_FRAME);
            let count = stream_end
                .saturating_sub(start_consumption)
                .saturating_add(1)
                .max(0) as usize;
            let values = cache.read_values(start_consumption, count).ok()?;
            let values = SearchValues::new(start_consumption, values);
            search_candidate_cancelable(start_consumption, config, &values, cancel)
        })
        .collect::<Vec<_>>();
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
pub fn infer_idx(config: &FieldConfig, actual_frame: i64) -> Result<FieldIdxInference, String> {
    infer_idx_cancelable(config, actual_frame, None)
}

#[cfg(test)]
#[allow(dead_code)]
fn infer_idx_cancelable(
    config: &FieldConfig,
    actual_frame: i64,
    cancel: Option<&AtomicBool>,
) -> Result<FieldIdxInference, String> {
    if actual_frame < 0 {
        return Err("実際に出たFrameは0以上で入力してください。".into());
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
) -> Result<FieldIdxInference, String> {
    infer_idx_for_configs_cancelable(configs, actual_frame, None)
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn infer_idx_for_configs_cancelable(
    configs: &[FieldConfig],
    actual_frame: i64,
    cancel: Option<&AtomicBool>,
) -> Result<FieldIdxInference, String> {
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
pub(crate) fn infer_idx_from_result_cancelable(
    seed: u32,
    result: &FieldResult,
    tolerance: i64,
    actual_frame: i64,
    cancel: Option<&AtomicBool>,
) -> Result<FieldIdxInference, String> {
    if actual_frame < 0 {
        return Err("実際に出たFrameは0以上で入力してください。".into());
    }
    check_cancelled(cancel)?;

    let models = result.models.max(1);
    let anchor_position = result
        .start_consumption
        .checked_add(result.observed_model as i64)
        .ok_or_else(|| "Timelineの開始位置が大きすぎます。".to_string())?;
    let anchor_value = sfmt_value_at(seed, anchor_position, cancel)?;
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
                return Some(Err(SEARCH_CANCELLED.into()));
            }
            let delta = anchor_model as i64 - candidate_idx as i64;
            let candidate_start = anchor_start.saturating_add(delta);
            let candidate_value = match sfmt_value_at(
                seed,
                candidate_start.saturating_add(candidate_idx as i64),
                cancel,
            ) {
                Ok(value) => value,
                Err(message) => return Some(Err(message)),
            };
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
                cancel,
            ) {
                Ok(true) => Some(Ok(candidate_idx)),
                Ok(false) => None,
                Err(message) => Some(Err(message)),
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
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
pub(crate) fn suggest_field_offset_from_result_cancelable(
    seed: u32,
    result: &FieldResult,
    target: i64,
    actual: i64,
    current_offset: i64,
    cancel: Option<&AtomicBool>,
) -> Result<Option<i64>, String> {
    if target < 0 || actual < 0 {
        return Ok(None);
    }
    check_cancelled(cancel)?;
    let Some(target_tick) = find_correction_tick_from_cursor(seed, result, target, cancel) else {
        return Ok(None);
    };
    let Some(actual_tick) = find_correction_tick_from_cursor(seed, result, actual, cancel) else {
        return Ok(None);
    };
    let drift_ticks = actual_tick.saturating_sub(target_tick);
    Ok(Some(
        current_offset.saturating_sub(drift_ticks.saturating_mul(2)),
    ))
}

fn sfmt_value_at(seed: u32, position: i64, cancel: Option<&AtomicBool>) -> Result<u64, String> {
    if !(0..=MAX_SFMT_FRAME).contains(&position) {
        return Err("Timelineの開始位置が範囲外です。".into());
    }
    let mut sfmt = Sfmt::new(seed);
    for _ in 0..position {
        check_cancelled(cancel)?;
        sfmt.next_u64();
    }
    Ok(sfmt.next_u64())
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
    cancel: Option<&AtomicBool>,
) -> Result<bool, String> {
    if start < 0 || actual_frame < start || target_model >= models {
        return Ok(false);
    }
    let mut sfmt = Sfmt::new(seed);
    for _ in 0..start {
        check_cancelled(cancel)?;
        sfmt.next_u64();
    }
    let mut status = ModelStatus::new(models);
    let required_events = observed_intervals.len().saturating_add(1);
    let mut blink_frames = Vec::with_capacity(required_events);
    let mut consumption = start;
    let mut tick = 0_i64;
    let mut matched = false;
    while tick < MAX_TARGET_TICKS && consumption <= actual_frame {
        check_cancelled(cancel)?;
        let (used, blink) = status.next_state(&mut sfmt);
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
    let (timeline, timeline_cursor, target_cursor) = if materialize_timeline {
        let target_cursor = TimelineCursor {
            status: last.status.clone(),
            stream_cursor: last.stream_cursor,
            consumption: last.consumption,
            tick: last.wait_frame / 2,
        };
        let (timeline, timeline_cursor) = build_timeline(
            window,
            target_model,
            target_timing.map(|timing| timing.absolute_wait_frame),
            values,
        );
        (timeline, timeline_cursor, Some(target_cursor))
    } else {
        let timeline = window
            .iter()
            .map(|event| FieldTimelineRow {
                frame: event.consumption,
                timeline_frame: event.wait_frame,
                duration_frames: 0,
                observed: true,
                target_consumption: event.model_consumption,
            })
            .collect::<Vec<_>>();
        let cursor = TimelineCursor {
            status: last.status.clone(),
            stream_cursor: last.stream_cursor,
            consumption: last.consumption,
            tick: last.wait_frame / 2,
        };
        (timeline, cursor.clone(), Some(cursor))
    };
    let target_blink_count = target_timing
        .filter(|_| materialize_timeline)
        .map(|timing| {
            timeline
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
        target_blink_count,
        timeline_cursor: Some(timeline_cursor),
        target_cursor,
    })
}

fn build_timeline(
    observed_events: &[BlinkEvent],
    target_model: usize,
    target_absolute_wait_frame: Option<i64>,
    values: &SearchValues,
) -> (Vec<FieldTimelineRow>, TimelineCursor) {
    let event = observed_events
        .last()
        .expect("observed events are non-empty");
    let mut status = event.status.clone();
    let mut stream_cursor = event.stream_cursor;
    let mut tick = event.wait_frame / 2;
    let mut consumption = event.consumption;
    let mut next_events = Vec::new();

    let event_limit = target_absolute_wait_frame
        .map(|_| usize::MAX)
        .unwrap_or(MAX_CANDIDATE_TIMELINE_EVENTS);
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
    let mut rows = Vec::with_capacity(all_events.len());
    for (index, (wait_frame, consumption, observed, target_consumption)) in
        all_events.iter().copied().enumerate()
    {
        let duration_frames = all_events
            .get(index + 1)
            .map(|(next_wait, _, _, _)| *next_wait - wait_frame)
            .unwrap_or_default();
        rows.push(FieldTimelineRow {
            frame: consumption,
            timeline_frame: wait_frame,
            duration_frames,
            observed,
            target_consumption,
        });
    }
    (
        rows,
        TimelineCursor {
            status,
            stream_cursor,
            consumption,
            tick,
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
    if event.consumption > target {
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
    while tick < MAX_TARGET_TICKS {
        if cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
            return None;
        }
        let (used, _) = next_state_with_values(&mut status, &mut stream_cursor, values)?;
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
    cancel: Option<&AtomicBool>,
) -> Option<(TargetTiming, usize)> {
    if event.last_consumption > target {
        return None;
    }
    if event.last_consumption == target {
        return Some((
            TargetTiming {
                absolute_wait_frame: event.last_wait_frame,
                exact: true,
                frame_before: target,
                frame_after: target,
            },
            0,
        ));
    }
    let cursor = event
        .target_cursor
        .as_ref()
        .or(event.timeline_cursor.as_ref())?;
    let mut sfmt = Sfmt::new(seed);
    for _ in 0..cursor.stream_cursor.max(0) {
        sfmt.next_u64();
    }
    let mut status = cursor.status.clone();
    let mut consumption = event.last_consumption;
    let mut tick = cursor.tick.max(event.last_wait_frame / 2);
    let mut blink_count = 0;
    while tick < MAX_TARGET_TICKS {
        if cancel.is_some_and(|token| token.load(Ordering::Acquire)) {
            return None;
        }
        let (used, blink) = status.next_state(&mut sfmt);
        let before = consumption;
        consumption = consumption.checked_add(used as i64)?;
        tick += 1;
        if blink.contains(&(event.observed_model as u8)) {
            blink_count += 1;
        }
        if before <= target && target < consumption {
            return Some((
                TargetTiming {
                    absolute_wait_frame: tick * 2,
                    exact: false,
                    frame_before: before,
                    frame_after: consumption,
                },
                blink_count,
            ));
        }
        if consumption == target {
            return Some((
                TargetTiming {
                    absolute_wait_frame: tick * 2,
                    exact: true,
                    frame_before: target,
                    frame_after: target,
                },
                blink_count,
            ));
        }
    }
    None
}

/// 参照ツールと同じ「消費境界を次の遷移時刻で扱う」規則で、補正用のtickを求める。
/// Targetタイマー本体の厳密着地判定とは独立させ、補正時だけ境界の表示時刻を揃える。
fn find_correction_tick_from_cursor(
    seed: u32,
    event: &FieldResult,
    target: i64,
    cancel: Option<&AtomicBool>,
) -> Option<i64> {
    if event.last_consumption > target {
        return None;
    }
    if event.last_consumption == target {
        return Some(event.last_wait_frame.div_euclid(2));
    }
    let cursor = event
        .target_cursor
        .as_ref()
        .or(event.timeline_cursor.as_ref())?;
    let mut sfmt = Sfmt::new(seed);
    for _ in 0..cursor.stream_cursor.max(0) {
        sfmt.next_u64();
    }
    let mut status = cursor.status.clone();
    let mut consumption = event.last_consumption;
    let mut tick = cursor.tick.max(event.last_wait_frame / 2);
    while tick < MAX_TARGET_TICKS {
        check_cancelled(cancel).ok()?;
        let before = consumption;
        let (used, _) = status.next_state(&mut sfmt);
        consumption = consumption.checked_add(used as i64)?;
        tick += 1;
        if before <= target && target < consumption {
            return Some(tick);
        }
    }
    None
}

/// 間隔一致した候補へTarget到達判定を付加する。
/// Targetへ到達できない場合も通常の瞬きタイマーを継続できるよう候補は残し、
/// Target関連の項目だけを利用不可またはfalseにする。
fn apply_target_from_cursor(
    mut result: FieldResult,
    seed: u32,
    target: i64,
    cancel: Option<&AtomicBool>,
) -> FieldResult {
    if target < result.start_consumption || target < result.last_consumption {
        result.target_wait_frame = None;
        result.target_exact = Some(false);
        result.target_frame_before = Some(target);
        result.target_frame_after = Some(result.last_consumption);
        result.target_blink_count = None;
        return result;
    }
    let Some((timing, blink_count)) = find_target_timing_from_cursor(seed, &result, target, cancel)
    else {
        result.target_wait_frame = None;
        result.target_exact = Some(false);
        result.target_frame_before = None;
        result.target_frame_after = None;
        result.target_blink_count = None;
        return result;
    };
    result.target_wait_frame = Some(timing.absolute_wait_frame - result.last_wait_frame);
    result.target_exact = Some(timing.exact);
    result.target_frame_before = Some(timing.frame_before);
    result.target_frame_after = Some(timing.frame_after);
    result.target_blink_count = Some(blink_count);
    result
}

fn validate(config: &FieldConfig) -> Result<(), String> {
    validate_static_config(config)?;
    if config.observed_intervals.is_empty() {
        return Err("瞬き間隔を1つ以上入力してください。".into());
    }
    if config.observed_intervals.len() >= crate::application::state::MAX_BLINK_OBSERVATIONS {
        return Err(format!(
            "瞬き観測は{}回まで入力できます。",
            crate::application::state::MAX_BLINK_OBSERVATIONS
        ));
    }
    if config.observed_intervals.iter().any(|value| *value < 0) {
        return Err("瞬き間隔は0以上の整数で入力してください。".into());
    }
    Ok(())
}

fn validate_static_config(config: &FieldConfig) -> Result<(), String> {
    if config.range_start < 0
        || config.range_end < config.range_start
        || config.range_end > MAX_SEARCH_FRAME
        || config.models == 0
        || config.models > MAX_MODELS
        || config.target_model >= config.models
        || config.tolerance < 0
    {
        return Err(format!(
            "タイムライン仕様の入力範囲または観測対象を確認してください（消費数範囲は0～{}、モデル数は1～{}）。",
            MAX_SEARCH_FRAME,
            MAX_MODELS
        ));
    }
    if config.range_end.saturating_sub(config.range_start) > MAX_SEARCH_SPAN {
        return Err(format!(
            "検索範囲は{}F以内で指定してください。",
            MAX_SEARCH_SPAN
        ));
    }
    if config
        .target_consumption
        .is_some_and(|target| target > MAX_SEARCH_FRAME)
    {
        return Err(format!(
            "目標消費数は{}以下で入力してください。",
            MAX_SEARCH_FRAME
        ));
    }
    Ok(())
}

fn parse_npc_counts(input: &str, multiple: bool) -> Result<Vec<usize>, String> {
    let value = input.trim();
    if !multiple {
        let count = usize::try_from(field_state::parse_nonnegative(value, "NPC数")?)
            .map_err(|_| "NPC数が大きすぎます。".to_string())?;
        if count > MAX_NPC_COUNT {
            return Err(format!(
                "NPC数は0～{}の範囲で入力してください。",
                MAX_NPC_COUNT
            ));
        }
        return Ok(vec![count]);
    }
    let mut parts = value.split(['~', '～']);
    let start = parts
        .next()
        .filter(|part| !part.trim().is_empty())
        .ok_or_else(|| "NPC範囲は例: 10~20 の形式で入力してください。".to_string())?;
    let end = parts
        .next()
        .filter(|part| !part.trim().is_empty())
        .ok_or_else(|| "NPC範囲は例: 10~20 の形式で入力してください。".to_string())?;
    if parts.next().is_some() {
        return Err("NPC範囲は例: 10~20 の形式で入力してください。".to_string());
    }
    let start = usize::try_from(field_state::parse_nonnegative(start, "NPC数")?)
        .map_err(|_| "NPC数が大きすぎます。".to_string())?;
    let end = usize::try_from(field_state::parse_nonnegative(end, "NPC数")?)
        .map_err(|_| "NPC数が大きすぎます。".to_string())?;
    if start > MAX_NPC_COUNT || end > MAX_NPC_COUNT {
        return Err(format!(
            "NPC数は0～{}の範囲で入力してください。",
            MAX_NPC_COUNT
        ));
    }
    if end < start {
        return Err("NPC範囲の終点は始点以上にしてください。".to_string());
    }
    let width = end.saturating_sub(start);
    if width > MAX_NPC_RANGE_WIDTH {
        return Err(format!(
            "NPC範囲は幅{}以内で指定してください。",
            MAX_NPC_RANGE_WIDTH
        ));
    }
    Ok((start..=end).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correction_counts_boundary_from_following_tick() {
        let cursor = TimelineCursor {
            status: ModelStatus::new(6),
            stream_cursor: 0,
            consumption: 0,
            tick: 0,
        };
        let result = FieldResult {
            start_consumption: 0,
            first_wait_frame: 0,
            last_wait_frame: 0,
            first_consumption: 0,
            last_consumption: 0,
            intervals: Vec::new(),
            target_consumptions: Vec::new(),
            next_blink_wait_frame: 0,
            target_wait_frame: None,
            target_exact: None,
            target_frame_before: None,
            target_frame_after: None,
            observed_model: 0,
            models: 6,
            timeline: Vec::new(),
            target_blink_count: None,
            timeline_cursor: Some(cursor.clone()),
            target_cursor: Some(cursor),
        };
        let corrected =
            suggest_field_offset_from_result_cancelable(0, &result, 6, 12, 4, None).unwrap();
        assert_eq!(corrected, Some(2));
    }

    #[test]
    fn candidate_starts_include_only_special_boundaries_near_the_base() {
        let mut values = vec![1_u64; 64];
        values[5] = 128;
        values[10] = 256;
        values[40] = 384;
        let search_values = SearchValues::new(100, values);

        assert_eq!(
            candidate_start_positions(100, 200, &search_values),
            vec![100, 106, 111]
        );
    }

    #[test]
    fn configured_initial_branch_scan_span_controls_special_boundaries() {
        let mut values = vec![1_u64; 64];
        values[40] = 128;
        let search_values = SearchValues::new(100, values);

        assert_eq!(
            candidate_start_positions_with_span(100, 200, 10, &search_values),
            vec![100]
        );
        assert_eq!(
            candidate_start_positions_with_span(100, 200, 40, &search_values),
            vec![100, 141]
        );
    }

    #[test]
    fn reference_tick_config_uses_gui_tolerance_in_display_frames() {
        let mut config = FieldConfig {
            seed: 1,
            range_start: 0,
            range_end: 100,
            models: 2,
            target_model: 0,
            observed_intervals: vec![15, 20],
            tolerance: 5,
            target_consumption: None,
        };
        assert_eq!(reference_tick_config(&config).tolerance, 3);

        config.tolerance = 20;
        assert_eq!(reference_tick_config(&config).tolerance, 10);
    }

    #[test]
    fn npc_model_conversion_only_moves_forward_from_candidate_start() {
        assert_eq!(shifted_start_for_requested_model(480, 2, 0), Some(482));
        assert_eq!(shifted_start_for_requested_model(480, 9, 6), Some(483));
        assert_eq!(shifted_start_for_requested_model(485, 4, 6), None);
        assert_eq!(shifted_start_for_requested_model(485, 7, 9), None);
    }

    #[test]
    fn npc_model_conversion_rebuilds_a_matching_timeline() {
        let base = 480_i64;
        let models = 13_usize;
        let mut converted = None;

        for seed in 0_u32..500 {
            let traces = generated_target_streams(seed, base, models, 240);
            let (positions, waits) = &traces[2];
            if positions.len() < 4 {
                continue;
            }
            let positions = positions[..4].to_vec();
            let intervals = waits[..4]
                .windows(2)
                .map(|pair| pair[1] - pair[0])
                .collect::<Vec<_>>();
            let config = FieldConfig {
                seed,
                range_start: base,
                range_end: base + 240,
                models,
                target_model: 0,
                observed_intervals: intervals.clone(),
                tolerance: 0,
                target_consumption: None,
            };
            let raw_config = reference_tick_config(&config);
            let stream_end = reference_search_stream_end(&raw_config);
            let values = SearchValues::new(
                base,
                generate_u64_values_from(
                    seed,
                    base as usize,
                    stream_end.saturating_sub(base).saturating_add(1) as usize,
                ),
            );
            let raw =
                search_v12_start_all_models(base, &raw_config, &config, &values, true, true, None);
            if !raw.iter().any(|result| {
                result.observed_model == 2
                    && result.intervals == intervals
                    && result.target_consumptions == positions
            }) {
                continue;
            }
            let resolved =
                resolve_observed_model_candidates(raw, &config, &raw_config, &values, None);
            if resolved.iter().any(|result| {
                result.start_consumption == base + 2
                    && result.observed_model == 0
                    && result.intervals == intervals
                    && result.target_consumptions == positions
            }) {
                converted = Some(seed);
                break;
            }
        }

        assert!(
            converted.is_some(),
            "NPC番号変換後に再生成したTimelineが観測列へ一致しませんでした"
        );
        println!("NPC番号変換の実Timeline検証Seed: {:?}", converted);
    }

    #[test]
    #[ignore = "1〜6段階後に複数候補が残る実データを探す診断"]
    fn diagnostic_unresolved_candidates_after_model_resolution() {
        let base = 480_i64;
        let mut checked = 0_usize;
        let mut unresolved = Vec::new();

        'seeds: for seed in 0_u32..500 {
            for &models in &[5_usize, 10, 13, 20, 22] {
                let stream_len = models.saturating_mul(280).saturating_add(128);
                let all_values = generate_u64_values_from(seed, base as usize, stream_len);
                let starts = candidate_start_positions(
                    base,
                    base + 240,
                    &SearchValues::new(base, all_values[..31].to_vec()),
                );
                if starts.len() < 2 {
                    continue;
                }
                let raw_config = FieldConfig {
                    seed,
                    range_start: base,
                    range_end: base + 240,
                    models,
                    target_model: 0,
                    observed_intervals: Vec::new(),
                    tolerance: 0,
                    target_consumption: None,
                };
                let stream_end = reference_search_stream_end(&raw_config);
                let values = SearchValues::new(
                    base,
                    generate_u64_values_from(
                        seed,
                        base as usize,
                        stream_end.saturating_sub(base).saturating_add(1) as usize,
                    ),
                );
                for target_model in 0..models {
                    for &source_start in &starts {
                        let trace = generated_target_trace_with_state(
                            seed,
                            source_start,
                            models,
                            target_model,
                            240,
                        );
                        if trace.len() < 4 {
                            continue;
                        }
                        let intervals = trace[..4]
                            .windows(2)
                            .map(|pair| pair[1].1 - pair[0].1)
                            .collect::<Vec<_>>();
                        let mut config = raw_config.clone();
                        config.target_model = target_model;
                        config.observed_intervals = intervals.clone();
                        let converted_config = reference_tick_config(&config);
                        let mut raw_results = Vec::new();
                        for &start in &starts {
                            raw_results.extend(search_v12_start_all_models(
                                start,
                                &converted_config,
                                &config,
                                &values,
                                true,
                                true,
                                None,
                            ));
                        }
                        let resolved = resolve_observed_model_candidates(
                            raw_results,
                            &config,
                            &converted_config,
                            &values,
                            None,
                        );
                        let mut keys = resolved
                            .into_iter()
                            .filter(|result| result.observed_model == target_model)
                            .map(|result| {
                                (
                                    result.start_consumption,
                                    result.observed_model,
                                    result.target_consumptions,
                                )
                            })
                            .collect::<Vec<_>>();
                        keys.sort_by_key(|key| key.0);
                        keys.dedup();
                        checked += 1;
                        if keys.len() > 1 {
                            unresolved.push((
                                seed,
                                models,
                                target_model,
                                source_start,
                                starts.clone(),
                                keys,
                            ));
                            if unresolved.len() >= 5 {
                                break 'seeds;
                            }
                        }
                    }
                }
            }
        }

        println!("1〜6段階後の未解決候補: checked={checked}, examples={unresolved:#?}");
        assert!(checked > 0);
    }

    #[test]
    #[ignore = "Seed 00000067の分岐候補周辺を比較する診断"]
    fn diagnostic_seed_00000067_compare_branch_neighbors() {
        let seed = 103_u32;
        let models = 5_usize;
        let target_model = 4_usize;
        let base = 480_i64;
        let values = generate_u64_values_from(seed, base as usize, 40);
        let special = values
            .iter()
            .enumerate()
            .filter(|(_, value)| **value & 0x7f == 0)
            .map(|(index, value)| (base + index as i64, *value))
            .collect::<Vec<_>>();
        println!("seed=00000067 special SFMT positions={special:#?}");
        for start in [480_i64, 486_i64] {
            let trace = generated_target_trace_with_state(seed, start, models, target_model, 600);
            println!(
                "seed=00000067 start={start} first_events={:#?}",
                trace.iter().take(8).collect::<Vec<_>>()
            );
        }
        let left = generated_target_trace_with_state(seed, 480, models, target_model, 100_000);
        let right = generated_target_trace_with_state(seed, 486, models, target_model, 100_000);
        let first_difference = left
            .iter()
            .zip(right.iter())
            .position(|(left, right)| left.0 != right.0 || left.1 != right.1);
        println!(
            "seed=00000067 target4 future: left_events={} right_events={} first_difference={first_difference:?}",
            left.len(),
            right.len()
        );
        let left_positions = left.iter().map(|event| event.0).collect::<Vec<_>>();
        let right_positions = right.iter().map(|event| event.0).collect::<Vec<_>>();
        let first_position_difference_after_two = left_positions
            .iter()
            .skip(2)
            .zip(right_positions.iter().skip(2))
            .position(|(left, right)| left != right);
        println!(
            "seed=00000067 target4 position difference after first 2: {first_position_difference_after_two:?}"
        );
    }

    #[test]
    #[ignore = "正規化後も複数の内部状態候補が残る反例を走査する診断"]
    fn diagnostic_counterexamples_after_full_state_merge() {
        let base = 480_i64;
        let mut checked = 0_usize;
        let mut unresolved = Vec::new();

        'seeds: for seed in 0_u32..500 {
            for &models in &[5_usize, 10, 13, 20, 22] {
                let values = generate_u64_values_from(
                    seed,
                    base as usize,
                    (INITIAL_BRANCH_SCAN_SPAN + 1) as usize,
                );
                let starts =
                    candidate_start_positions(base, base + 240, &SearchValues::new(base, values));
                if starts.len() < 2 {
                    continue;
                }
                let raw_config = FieldConfig {
                    seed,
                    range_start: base,
                    range_end: base + 240,
                    models,
                    target_model: 0,
                    observed_intervals: Vec::new(),
                    tolerance: 0,
                    target_consumption: None,
                };
                let stream_end = reference_search_stream_end(&raw_config);
                let search_values = SearchValues::new(
                    base,
                    generate_u64_values_from(
                        seed,
                        base as usize,
                        stream_end.saturating_sub(base).saturating_add(1) as usize,
                    ),
                );

                for target_model in 0..models {
                    for &source_start in &starts {
                        let trace = generated_target_trace_with_state(
                            seed,
                            source_start,
                            models,
                            target_model,
                            240,
                        );
                        if trace.len() < 4 {
                            continue;
                        }
                        let intervals = trace[..4]
                            .windows(2)
                            .map(|pair| pair[1].1 - pair[0].1)
                            .collect::<Vec<_>>();
                        let mut config = raw_config.clone();
                        config.target_model = target_model;
                        config.observed_intervals = intervals;
                        let converted_config = reference_tick_config(&config);
                        let mut raw_results = Vec::new();
                        for &start in &starts {
                            raw_results.extend(search_v12_start_all_models(
                                start,
                                &converted_config,
                                &config,
                                &search_values,
                                true,
                                true,
                                None,
                            ));
                        }
                        let resolved = resolve_observed_model_candidates(
                            raw_results,
                            &config,
                            &converted_config,
                            &search_values,
                            None,
                        );
                        let mut state = FieldState::default();
                        state.target_model = target_model.to_string();
                        let normalized = state.normalize_search_results(resolved);
                        checked += 1;
                        if normalized.len() > 1 {
                            if seed == 103 && models == 5 && target_model == 4 {
                                for result in &normalized {
                                    println!(
                                        "seed=00000067 normalized start={} cursor={:?}",
                                        result.start_consumption,
                                        result
                                            .target_cursor
                                            .as_ref()
                                            .or(result.timeline_cursor.as_ref())
                                            .map(|cursor| {
                                                (
                                                    cursor.stream_cursor,
                                                    cursor.consumption,
                                                    cursor.tick,
                                                    cursor.status.remain.clone(),
                                                )
                                            })
                                    );
                                }
                            }
                            unresolved.push((
                                seed,
                                models,
                                target_model,
                                source_start,
                                normalized
                                    .iter()
                                    .map(|result| {
                                        let cursor = result
                                            .target_cursor
                                            .as_ref()
                                            .or(result.timeline_cursor.as_ref())
                                            .map(|cursor| {
                                                (
                                                    cursor.stream_cursor,
                                                    cursor.consumption,
                                                    cursor.tick,
                                                    cursor.status.remain.clone(),
                                                )
                                            });
                                        (
                                            result.start_consumption,
                                            result.observed_model,
                                            result.target_consumptions.clone(),
                                            cursor,
                                        )
                                    })
                                    .collect::<Vec<_>>(),
                            ));
                            if unresolved.len() >= 10 {
                                break 'seeds;
                            }
                        }
                    }
                }
            }
        }

        println!("正規化後の未解決候補: checked={checked}, examples={unresolved:#?}");
        assert!(checked > 0);
    }

    #[test]
    #[ignore = "同一Start480の別観測窓を追加瞬きで区別できるかの診断"]
    fn diagnostic_seed_0000019f_same_start_windows_future_disambiguation() {
        let seed = 415_u32;
        let models = 22_usize;
        let target_model = 4_usize;
        let trace = generated_target_trace_with_state(seed, 480, models, target_model, 20_000);
        let windows = [[688_i64, 1430, 2269, 3098], [1430_i64, 2269, 3098, 3947]]
            .into_iter()
            .map(|positions| {
                trace
                    .windows(4)
                    .find(|window| window.iter().map(|event| event.0).eq(positions.into_iter()))
                    .map(|window| window.to_vec())
                    .expect("diagnostic observation window exists")
            })
            .collect::<Vec<_>>();

        let futures = windows
            .iter()
            .map(|window| continue_target_trace(seed, window.last().unwrap(), target_model, 2_000))
            .collect::<Vec<_>>();
        let first_difference = futures[0]
            .iter()
            .zip(futures[1].iter())
            .position(|(left, right)| left != right);
        println!(
            "seed=0000019F same-start windows={:?}",
            windows
                .iter()
                .map(|window| {
                    (
                        window.iter().map(|event| event.0).collect::<Vec<_>>(),
                        window.iter().map(|event| event.1).collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>()
        );
        println!(
            "seed=0000019F future first_difference={first_difference:?} left_len={} right_len={} left_head={:?} right_head={:?}",
            futures[0].len(),
            futures[1].len(),
            futures[0].iter().take(8).collect::<Vec<_>>(),
            futures[1].iter().take(8).collect::<Vec<_>>(),
        );
        assert!(first_difference.is_some());
    }

    #[test]
    #[ignore = "正規化後に残る3系列を追加瞬きで区別できるかの診断"]
    fn diagnostic_remaining_counterexamples_future_disambiguation() {
        let cases = [
            (
                104_u32,
                10_usize,
                0_usize,
                [480_i64, 517_i64],
                [647_i64, 1434, 1751, 2238],
                [649_i64, 1434, 1751, 2238],
            ),
            (
                269_u32,
                20_usize,
                9_usize,
                [482_i64, 501_i64],
                [937_i64, 1741, 2607, 3635],
                [984_i64, 1741, 2607, 3635],
            ),
            (
                415_u32,
                13_usize,
                0_usize,
                [483_i64, 508_i64],
                [644_i64, 1215, 1843, 2304],
                [688_i64, 1215, 1843, 2304],
            ),
        ];

        for (seed, models, target_model, starts, left_positions, right_positions) in cases {
            let traces = starts
                .into_iter()
                .map(|start| {
                    generated_target_trace_with_state(seed, start, models, target_model, 20_000)
                })
                .collect::<Vec<_>>();
            let expected = [left_positions, right_positions];
            let windows = traces
                .iter()
                .zip(expected)
                .map(|(trace, positions)| {
                    trace
                        .windows(4)
                        .find(|window| window.iter().map(|event| event.0).eq(positions.into_iter()))
                        .map(|window| window.to_vec())
                        .expect("diagnostic observation window exists")
                })
                .collect::<Vec<_>>();
            let futures = windows
                .iter()
                .map(|window| {
                    continue_target_trace(seed, window.last().unwrap(), target_model, 100_000)
                })
                .collect::<Vec<_>>();
            let first_difference = futures[0]
                .iter()
                .zip(futures[1].iter())
                .position(|(left, right)| left != right);
            println!(
                "seed={seed:08X} models={models} target_model={target_model} windows={:?} future_first_difference={first_difference:?} left_len={} right_len={} left_head={:?} right_head={:?}",
                windows
                    .iter()
                    .map(|window| {
                        (
                            window.iter().map(|event| event.0).collect::<Vec<_>>(),
                            window.iter().map(|event| event.1).collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>(),
                futures[0].len(),
                futures[1].len(),
                futures[0].iter().take(6).collect::<Vec<_>>(),
                futures[1].iter().take(6).collect::<Vec<_>>(),
            );
        }
    }

    #[test]
    fn multiple_initial_special_boundaries_keep_all_branch_starts() {
        let base = 480_i64;
        let scan_end = base + INITIAL_BRANCH_SCAN_SPAN;
        let mut examples = Vec::new();

        // 実際のSFMT列から、基準位置の近傍に特殊値が2つ以上あるSeedを探す。
        // ここで見つけた開始位置は、通常の基準Timelineと各-5分岐Timelineの候補になる。
        for seed in 0_u32..50_000 {
            let values =
                generate_u64_values_from(seed, base as usize, (scan_end - base + 1) as usize);
            let search_values = SearchValues::new(base, values.clone());
            let starts = candidate_start_positions(base, base + 100, &search_values);
            if starts.len() < 3 {
                continue;
            }
            let special_starts = (base..=scan_end)
                .filter(|position| {
                    search_values
                        .get(*position)
                        .is_some_and(|value| value & 0x7f == 0)
                })
                .map(|position| position + 1)
                .collect::<Vec<_>>();
            assert_eq!(starts, {
                let mut expected = vec![base];
                expected.extend(special_starts.iter().copied());
                expected.sort_unstable();
                expected.dedup();
                expected
            });
            examples.push((seed, special_starts, starts));
            if examples.len() == 3 {
                break;
            }
        }

        assert!(
            examples.len() == 3,
            "近傍に-5分岐が2つ以上あるSeedを見つけられませんでした"
        );
        println!("複数-5分岐の検証Seed: {examples:#?}");
    }

    #[test]
    #[ignore = "複数-5分岐で同じNPC番号の観測列衝突を探す診断。--ignored で実行"]
    fn diagnostic_same_idx_distinct_sfmt_sequences() {
        let base = 480_i64;
        let models = 13_usize;
        let required_events = 4_usize;
        let mut checked_seeds = 0_usize;
        let mut branch_seed_count = 0_usize;
        let mut counterexamples = Vec::new();

        'seeds: for seed in 0_u32..50_000 {
            let values = generate_u64_values_from(
                seed,
                base as usize,
                (INITIAL_BRANCH_SCAN_SPAN + 1) as usize,
            );
            let starts =
                candidate_start_positions(base, base + 100, &SearchValues::new(base, values));
            if starts.len() < 3 {
                continue;
            }
            branch_seed_count += 1;
            let traces = starts
                .iter()
                .map(|start| generated_target_streams(seed, *start, models, 600))
                .collect::<Vec<_>>();
            for left_start in 0..traces.len() {
                for right_start in (left_start + 1)..traces.len() {
                    for model in 0..models {
                        let left = &traces[left_start][model];
                        let right = &traces[right_start][model];
                        if left.0.len() < required_events || right.0.len() < required_events {
                            continue;
                        }
                        for left_window in 0..=left.0.len() - required_events {
                            let left_positions =
                                &left.0[left_window..left_window + required_events];
                            let left_waits = &left.1[left_window..left_window + required_events];
                            let left_intervals = left_waits
                                .windows(2)
                                .map(|pair| pair[1] - pair[0])
                                .collect::<Vec<_>>();
                            for right_window in 0..=right.0.len() - required_events {
                                let right_positions =
                                    &right.0[right_window..right_window + required_events];
                                let right_waits =
                                    &right.1[right_window..right_window + required_events];
                                let right_intervals = right_waits
                                    .windows(2)
                                    .map(|pair| pair[1] - pair[0])
                                    .collect::<Vec<_>>();
                                if left_intervals == right_intervals
                                    && left_positions != right_positions
                                {
                                    counterexamples.push((
                                        seed,
                                        starts[left_start],
                                        starts[right_start],
                                        model,
                                        left_positions.to_vec(),
                                        right_positions.to_vec(),
                                        left_intervals.clone(),
                                    ));
                                    if counterexamples.len() >= 5 {
                                        break 'seeds;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            checked_seeds += 1;
        }

        println!(
            "同一NPC番号・異なるSFMT列の間隔一致: checked_branch_seeds={branch_seed_count}, counterexamples={counterexamples:#?}"
        );
        assert!(checked_seeds > 0 || branch_seed_count > 0);
        // これは「衝突が存在しない」ことを前提にするテストではない。
        // 実機入力だけでは区別できない衝突を検出した記録として残す。
    }

    #[test]
    #[ignore = "同一NPC番号の衝突候補についてremainとSFMTカーソルを確認する診断"]
    fn diagnostic_seed_0000123a_compare_remain_after_matching_blinks() {
        let seed = 0x0000_123a;
        let models = 13_usize;
        let target_model = 10_usize;
        let left = generated_target_trace_with_state(seed, 482, models, target_model, 600);
        let right = generated_target_trace_with_state(seed, 508, models, target_model, 600);
        println!("seed=0000123A left_start=482 trace={left:#?}");
        println!("seed=0000123A right_start=508 trace={right:#?}");
        assert!(left.len() >= 4 && right.len() >= 4);
    }

    #[test]
    #[ignore = "合流前に同じNPC番号の観測列が衝突するケースを走査する診断"]
    fn diagnostic_post_merge_unresolved_same_model_cases() {
        let base = 480_i64;
        let required_events = 4_usize;
        let model_counts = [5_usize, 10, 13, 20, 22];
        let mut checked = 0_usize;
        let mut unresolved = Vec::new();

        'seeds: for seed in 0_u32..10_000 {
            let values = generate_u64_values_from(
                seed,
                base as usize,
                (INITIAL_BRANCH_SCAN_SPAN + 1) as usize,
            );
            let starts =
                candidate_start_positions(base, base + 100, &SearchValues::new(base, values));
            if starts.len() < 2 {
                continue;
            }
            checked += 1;
            for &models in &model_counts {
                for target_model in 0..models {
                    let model_traces = starts
                        .iter()
                        .enumerate()
                        .map(|(index, start)| {
                            (
                                *start,
                                generated_target_trace_with_state(
                                    seed,
                                    *start,
                                    models,
                                    target_model,
                                    600,
                                ),
                                index,
                            )
                        })
                        .collect::<Vec<_>>();
                    for left in &model_traces {
                        if left.1.len() < required_events {
                            continue;
                        }
                        for left_window in left.1.windows(required_events) {
                            let left_intervals = left_window
                                .windows(2)
                                .map(|pair| pair[1].1 - pair[0].1)
                                .collect::<Vec<_>>();
                            for right in &model_traces {
                                if right.0 <= left.0 || right.1.len() < required_events {
                                    continue;
                                }
                                if right.1.windows(required_events).any(|right_window| {
                                    let right_intervals = right_window
                                        .windows(2)
                                        .map(|pair| pair[1].1 - pair[0].1)
                                        .collect::<Vec<_>>();
                                    if left_intervals != right_intervals {
                                        return false;
                                    }
                                    let left_state = left_window.last().unwrap();
                                    let right_state = right_window.last().unwrap();
                                    left_state.2 != right_state.2 || left_state.3 != right_state.3
                                }) {
                                    unresolved.push((
                                        seed,
                                        models,
                                        target_model,
                                        left.0,
                                        right.0,
                                        left_intervals.clone(),
                                    ));
                                    if unresolved.len() >= 5 {
                                        break 'seeds;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        println!(
            "合流前の同一NPC番号・間隔列衝突: checked_branch_seeds={checked}, examples={unresolved:#?}"
        );
        assert!(checked > 0);
    }

    #[test]
    #[ignore = "同じNPC番号・同じ間隔列で状態が分かれる具体例の出力"]
    fn diagnostic_seed_00000024_unresolved_same_model_case() {
        let seed = 36_u32;
        let models = 22_usize;
        let target_model = 8_usize;
        let base = 480_i64;
        let values =
            generate_u64_values_from(seed, base as usize, (INITIAL_BRANCH_SCAN_SPAN + 1) as usize);
        let starts = candidate_start_positions(base, base + 100, &SearchValues::new(base, values));
        for start in starts {
            let trace = generated_target_trace_with_state(seed, start, models, target_model, 600);
            for window in trace.windows(4) {
                let intervals = window
                    .windows(2)
                    .map(|pair| pair[1].1 - pair[0].1)
                    .collect::<Vec<_>>();
                if intervals == [98, 114, 76] {
                    let last = window.last().unwrap();
                    println!(
                        "seed=00000024 models=22 idx=8 start={start} positions={:?} waits={:?} cursor={} remain={:?}",
                        window.iter().map(|event| event.0).collect::<Vec<_>>(),
                        window.iter().map(|event| event.1).collect::<Vec<_>>(),
                        last.2,
                        last.3,
                    );
                }
            }
        }

        let left = generated_target_trace_with_state(seed, 480, models, target_model, 600);
        let right = generated_target_trace_with_state(seed, 491, models, target_model, 600);
        let left_window = left
            .windows(4)
            .find(|window| {
                window
                    .windows(2)
                    .map(|pair| pair[1].1 - pair[0].1)
                    .collect::<Vec<_>>()
                    == [98, 114, 76]
            })
            .unwrap();
        let right_window = right
            .windows(4)
            .find(|window| {
                window
                    .windows(2)
                    .map(|pair| pair[1].1 - pair[0].1)
                    .collect::<Vec<_>>()
                    == [98, 114, 76]
            })
            .unwrap();
        let left_future =
            continue_target_trace(seed, left_window.last().unwrap(), target_model, 100_000);
        let right_future =
            continue_target_trace(seed, right_window.last().unwrap(), target_model, 100_000);
        let first_difference = left_future
            .iter()
            .zip(right_future.iter())
            .position(|(left, right)| left != right);
        println!(
            "seed=00000024 future target events: left={} right={} first_difference={first_difference:?}",
            left_future.len(),
            right_future.len(),
        );
    }

    #[test]
    #[ignore = "開始480/491からSFMT5122までの消費経路を確認する診断"]
    fn diagnostic_seed_00000024_flow_to_sfmt_5122() {
        for start in [480_i64, 491_i64] {
            let values = generate_u64_values_from(36, start as usize, 8_000);
            let mut status = ModelStatus::new(22);
            let mut stream_cursor = start;
            let mut value_index = 0;
            let mut special_count = 0;
            let mut target_blink_count = 0;
            println!("start={start}");
            for tick in 1..=1_000 {
                let before_cursor = stream_cursor;
                let (_used, consumed) = status.next_state_with_positions(|| {
                    let position = stream_cursor;
                    let value = values[value_index];
                    value_index += 1;
                    stream_cursor += 1;
                    (position, value)
                });
                let special = consumed
                    .iter()
                    .filter(|event| !event.blink && event.value & 0x7f == 0)
                    .map(|event| (event.model, event.position))
                    .collect::<Vec<_>>();
                let target_blinks = consumed
                    .iter()
                    .filter(|event| event.model == 8 && event.blink)
                    .map(|event| event.position)
                    .collect::<Vec<_>>();
                if !special.is_empty() {
                    special_count += special.len();
                }
                if !target_blinks.is_empty() {
                    target_blink_count += target_blinks.len();
                }
                if !special.is_empty() || !target_blinks.is_empty() {
                    println!(
                        "  tick={tick:>3} cursor={before_cursor}->{stream_cursor} special_models={special:?} target8_blinks={target_blinks:?} remain8={} total_special={special_count} target8_count={target_blink_count}",
                        status.remain[8]
                    );
                }
                if target_blinks.iter().any(|position| *position >= 5_122) {
                    println!("  reached target SFMT 5122 at tick={tick}");
                    break;
                }
            }
        }
    }

    #[test]
    #[ignore = "同じNPC番号・同じSFMTカーソル・対象remain一致でも将来分岐するかの診断"]
    fn diagnostic_same_target_state_but_other_models_diverge() {
        let base = 480_i64;
        let required_events = 4_usize;
        let model_counts = [5_usize, 10, 13, 20, 22];
        let mut checked = 0_usize;
        let mut examples = Vec::new();

        'seeds: for seed in 0_u32..500 {
            let values = generate_u64_values_from(
                seed,
                base as usize,
                (INITIAL_BRANCH_SCAN_SPAN + 1) as usize,
            );
            let starts =
                candidate_start_positions(base, base + 100, &SearchValues::new(base, values));
            if starts.len() < 2 {
                continue;
            }
            checked += 1;
            for &models in &model_counts {
                for target_model in 0..models {
                    let traces = starts
                        .iter()
                        .map(|start| {
                            generated_target_trace_with_state(
                                seed,
                                *start,
                                models,
                                target_model,
                                600,
                            )
                        })
                        .collect::<Vec<_>>();
                    for left_index in 0..traces.len() {
                        for right_index in (left_index + 1)..traces.len() {
                            for left_window in traces[left_index].windows(required_events) {
                                let left_intervals = left_window
                                    .windows(2)
                                    .map(|pair| pair[1].1 - pair[0].1)
                                    .collect::<Vec<_>>();
                                let left_last = left_window.last().unwrap();
                                for right_window in traces[right_index].windows(required_events) {
                                    let right_intervals = right_window
                                        .windows(2)
                                        .map(|pair| pair[1].1 - pair[0].1)
                                        .collect::<Vec<_>>();
                                    let right_last = right_window.last().unwrap();
                                    if left_intervals != right_intervals
                                        || left_last.2 != right_last.2
                                        || left_last.3[target_model] != right_last.3[target_model]
                                        || left_last.3 == right_last.3
                                    {
                                        continue;
                                    }
                                    let left_future =
                                        continue_target_trace(seed, left_last, target_model, 5_000);
                                    let right_future = continue_target_trace(
                                        seed,
                                        right_last,
                                        target_model,
                                        5_000,
                                    );
                                    if left_future != right_future {
                                        examples.push((
                                            seed,
                                            models,
                                            target_model,
                                            starts[left_index],
                                            starts[right_index],
                                            left_intervals.clone(),
                                            left_future.len(),
                                            right_future.len(),
                                        ));
                                        if examples.len() >= 5 {
                                            break 'seeds;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        println!(
            "同一対象モデル状態からの将来分岐: checked_branch_seeds={checked}, examples={examples:#?}"
        );
        assert!(checked > 0);
    }

    fn synthetic_result(
        start_consumption: i64,
        observed_model: usize,
        intervals: Vec<i64>,
        target_consumptions: Vec<i64>,
    ) -> FieldResult {
        FieldResult {
            start_consumption,
            first_wait_frame: 50,
            last_wait_frame: 50 + intervals.iter().sum::<i64>(),
            first_consumption: target_consumptions[0],
            last_consumption: *target_consumptions.last().unwrap(),
            intervals,
            target_consumptions,
            next_blink_wait_frame: 0,
            target_wait_frame: None,
            target_exact: None,
            target_frame_before: None,
            target_frame_after: None,
            observed_model,
            models: 13,
            timeline: Vec::new(),
            target_blink_count: None,
            timeline_cursor: None,
            target_cursor: None,
        }
    }

    #[test]
    fn observable_candidates_keep_different_sfmt_positions() {
        let candidates = vec![
            synthetic_result(1_000, 5, vec![506, 362], vec![1288, 3739, 5522]),
            synthetic_result(1_020, 12, vec![502, 362], vec![1284, 3739, 5522]),
        ];
        let merged = merge_observable_candidates(candidates);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].start_consumption, 1_000);
    }

    #[test]
    fn observable_candidates_stay_separate_when_target_stream_diverges() {
        let candidates = vec![
            synthetic_result(1_000, 5, vec![506, 362], vec![1288, 3739, 5522]),
            synthetic_result(1_020, 12, vec![502, 362], vec![1284, 3739, 5523]),
        ];
        let merged = merge_observable_candidates(candidates);
        assert_eq!(merged.len(), 2);
    }

    fn generated_target_streams(
        seed: u32,
        start_consumption: i64,
        models: usize,
        ticks: usize,
    ) -> Vec<(Vec<i64>, Vec<i64>)> {
        let values = generate_u64_values_from(
            seed,
            start_consumption as usize,
            models.saturating_mul(ticks).saturating_add(64),
        );
        let mut status = ModelStatus::new(models);
        let mut stream_cursor = start_consumption;
        let mut value_index = 0;
        let mut streams = vec![(Vec::new(), Vec::new()); models];
        for tick in 1..=ticks {
            let (_used, consumed) = status.next_state_with_positions(|| {
                let position = stream_cursor;
                let value = values[value_index];
                value_index += 1;
                stream_cursor += 1;
                (position, value)
            });
            for event in consumed.into_iter().filter(|event| event.blink) {
                streams[event.model].0.push(event.position);
                streams[event.model].1.push(tick as i64 * 2);
            }
        }
        streams
    }

    fn generated_target_trace_with_state(
        seed: u32,
        start_consumption: i64,
        models: usize,
        target_model: usize,
        ticks: usize,
    ) -> Vec<(i64, i64, i64, Vec<i32>)> {
        let values = generate_u64_values_from(
            seed,
            start_consumption as usize,
            models.saturating_mul(ticks).saturating_add(64),
        );
        let mut status = ModelStatus::new(models);
        let mut stream_cursor = start_consumption;
        let mut value_index = 0;
        let mut trace = Vec::new();
        for tick in 1..=ticks {
            let (_used, consumed) = status.next_state_with_positions(|| {
                let position = stream_cursor;
                let value = values[value_index];
                value_index += 1;
                stream_cursor += 1;
                (position, value)
            });
            if consumed
                .iter()
                .any(|event| event.model == target_model && event.blink)
            {
                trace.push((
                    consumed
                        .iter()
                        .find(|event| event.model == target_model && event.blink)
                        .map(|event| event.position)
                        .unwrap(),
                    tick as i64 * 2,
                    stream_cursor,
                    status.remain.clone(),
                ));
            }
        }
        trace
    }

    fn continue_target_trace(
        seed: u32,
        last_event: &(i64, i64, i64, Vec<i32>),
        target_model: usize,
        ticks: usize,
    ) -> Vec<(i64, i64)> {
        let models = last_event.3.len();
        let values = generate_u64_values_from(
            seed,
            last_event.2 as usize,
            models.saturating_mul(ticks).saturating_add(64),
        );
        let mut status = ModelStatus {
            remain: last_event.3.clone(),
        };
        let mut stream_cursor = last_event.2;
        let mut value_index = 0;
        let mut trace = Vec::new();
        for tick in 1..=ticks {
            let (_used, consumed) = status.next_state_with_positions(|| {
                let position = stream_cursor;
                let value = values[value_index];
                value_index += 1;
                stream_cursor += 1;
                (position, value)
            });
            if let Some(event) = consumed
                .into_iter()
                .find(|event| event.model == target_model && event.blink)
            {
                trace.push((event.position, tick as i64 * 2));
            }
        }
        trace
    }

    #[test]
    fn generated_sfmt_patterns_merge_converged_candidates_and_keep_divergence() {
        let mut tested_converged_pairs = 0;
        let mut pseudo_seed = 0x1357_9bdf_u32;
        for _ in 0..24 {
            pseudo_seed = pseudo_seed
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            let seed = pseudo_seed;
            for &models in &[5_usize, 10, 13, 20] {
                let starts = [480_i64, 481, 482, 483, 484];
                let traces = starts
                    .iter()
                    .map(|start| generated_target_streams(seed, *start, models, 400))
                    .collect::<Vec<_>>();
                for left_index in 0..starts.len() {
                    for right_index in (left_index + 1)..starts.len() {
                        for left_model in 0..models {
                            let (left_positions, left_waits) = &traces[left_index][left_model];
                            let left_intervals = left_waits
                                .windows(2)
                                .map(|pair| pair[1] - pair[0])
                                .collect::<Vec<_>>();
                            if left_intervals.len() < 2 {
                                continue;
                            }
                            for right_model in 0..models {
                                let (right_positions, right_waits) =
                                    &traces[right_index][right_model];
                                let right_intervals = right_waits
                                    .windows(2)
                                    .map(|pair| pair[1] - pair[0])
                                    .collect::<Vec<_>>();
                                if left_intervals.get(1..) != right_intervals.get(1..)
                                    || left_positions.get(1..) != right_positions.get(1..)
                                    || (left_intervals.first() == right_intervals.first()
                                        && left_positions.first() == right_positions.first())
                                {
                                    continue;
                                }
                                let candidates = vec![
                                    synthetic_result(
                                        starts[left_index],
                                        left_model,
                                        left_intervals.clone(),
                                        left_positions.clone(),
                                    ),
                                    synthetic_result(
                                        starts[right_index],
                                        right_model,
                                        right_intervals,
                                        right_positions.clone(),
                                    ),
                                ];
                                assert_eq!(merge_observable_candidates(candidates).len(), 2);
                                tested_converged_pairs += 1;
                            }
                        }
                    }
                }
            }
        }

        assert!(
            tested_converged_pairs > 0,
            "ランダムに生成した範囲で収束する候補を見つけられませんでした"
        );

        let streams_a = generated_target_streams(1, 1_000, 13, 400);
        let streams_b = generated_target_streams(1, 1_020, 13, 400);
        let left = &streams_a[5];
        let right = &streams_b[12];
        let left_intervals = left
            .1
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .collect::<Vec<_>>();
        let right_intervals = right
            .1
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .collect::<Vec<_>>();
        assert_eq!(left_intervals.get(1..), right_intervals.get(1..));
        assert_eq!(left.0.get(1..), right.0.get(1..));
        assert_ne!(left.0.first(), right.0.first());
        assert_eq!(
            merge_observable_candidates(vec![
                synthetic_result(1_000, 5, left_intervals.clone(), left.0.clone()),
                synthetic_result(1_020, 12, right_intervals.clone(), right.0.clone()),
            ])
            .len(),
            2
        );

        let mut divergent_positions = right.0.clone();
        let last = divergent_positions
            .last_mut()
            .expect("収束ケースには複数の瞬きが必要です");
        *last += 1;
        assert_eq!(
            merge_observable_candidates(vec![
                synthetic_result(1_000, 5, left_intervals.clone(), left.0.clone()),
                synthetic_result(1_020, 12, right_intervals, divergent_positions),
            ])
            .len(),
            2
        );
    }
}
