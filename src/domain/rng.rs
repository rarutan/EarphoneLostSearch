use rayon::prelude::*;
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

#[path = "encounter.rs"]
#[allow(dead_code)]
mod encounter;
pub(crate) use encounter::*;
#[path = "observation_cache.rs"]
mod observation_cache;
pub(crate) use observation_cache::{
    blink_interval_seconds, blink_interval_ticks, generate_observation_cache,
    generate_observation_cache_cancelable, BlinkTableRow, ObservationCache, ObservationMatch,
};

use crate::domain::timing::blink_ticks_to_seconds;
pub(crate) use crate::domain::timing::GAME_FRAMES_PER_BLINK_TICK;

const N: usize = 156;
const N32: usize = 624;
// 孵化後の瞬きは64bit乱数を1個使い、(乱数値 % 120) + 130回の
// 約1/30秒チェックで次に発生する。これはSFMTの消費位置とは別の軸である。
const BLINK_INTERVAL_MODULUS: u64 = 120;
const BLINK_INTERVAL_BASE_TICKS: u64 = 130;
// 観測候補の末尾でも追加の瞬き列を照合できるように先読みする。
const OBSERVATION_LOOKAHEAD: i64 = 4096;
const OBSERVATION_CACHE_VERSION: u32 = 1;
const OBSERVATION_CACHE_HEADER_SIZE: u64 = 24;

/// 全タブで共有するSFMTキャッシュの最大Frame位置。
/// 値は0番目から格納するため、ファイル上の要素数はこの値+1になる。
pub const MAX_SFMT_FRAME: i64 = 50_000_000;

/// Target値に依存せず保持できる連続本番Timelineの先読み上限。
/// Targetは生成済み経路の検索位置にすぎないため、更新しても経路を作り直さない。
pub const PRODUCTION_TIMELINE_CACHE_FRAMES: i64 = 100_000;
const PRODUCTION_TIMELINE_CACHE_CHUNK_FRAMES: i64 = 4_096;

// ---------------------------------------------------------------------------
// SFMTの生成と参照
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Sfmt {
    pub state: [u32; N32],
    pub index: usize,
}
impl Sfmt {
    pub fn new(seed: u32) -> Self {
        let mut s = [0; N32];
        s[0] = seed;
        for i in 1..N32 {
            s[i] = 1812433253u32
                .wrapping_mul(s[i - 1] ^ (s[i - 1] >> 30))
                .wrapping_add(i as u32);
        }
        let mut r = Self {
            state: s,
            index: N32,
        };
        r.certify();
        r
    }
    fn certify(&mut self) {
        let p = [1, 0, 0, 0x13c9e684];
        let mut x = 0;
        for (i, mask) in p.iter().enumerate() {
            x ^= self.state[i] & mask;
        }
        x ^= x >> 16;
        x ^= x >> 8;
        x ^= x >> 4;
        x ^= x >> 2;
        x ^= x >> 1;
        if x & 1 == 0 {
            for (i, mask) in p.iter().enumerate() {
                let mut b = 1;
                for _ in 0..32 {
                    if mask & b != 0 {
                        self.state[i] ^= b;
                        return;
                    }
                    b <<= 1;
                }
            }
        }
    }
    pub fn refresh(&mut self) {
        let (mut a, mut b, mut c, mut d) = (0, 122 * 4, (N - 2) * 4, (N - 1) * 4);
        while a < N32 {
            self.state[a + 3] ^= self.state[a + 3] << 8
                ^ self.state[a + 2] >> 24
                ^ self.state[c + 3] >> 8
                ^ (self.state[b + 3] >> 11 & 0xbfff_fff6)
                ^ self.state[d + 3] << 18;
            self.state[a + 2] ^= self.state[a + 2] << 8
                ^ self.state[a + 1] >> 24
                ^ self.state[c + 3] << 24
                ^ self.state[c + 2] >> 8
                ^ (self.state[b + 2] >> 11 & 0xbffa_ffff)
                ^ self.state[d + 2] << 18;
            self.state[a + 1] ^= self.state[a + 1] << 8
                ^ self.state[a] >> 24
                ^ self.state[c + 2] << 24
                ^ self.state[c + 1] >> 8
                ^ (self.state[b + 1] >> 11 & 0xddfe_cb7f)
                ^ self.state[d + 1] << 18;
            self.state[a] ^= self.state[a] << 8
                ^ self.state[c + 1] << 24
                ^ self.state[c] >> 8
                ^ (self.state[b] >> 11 & 0xdfff_ffef)
                ^ self.state[d] << 18;
            c = d;
            d = a;
            a += 4;
            b += 4;
            if b >= N32 {
                b = 0;
            }
        }
        self.index = 0;
    }
    pub fn next_u32(&mut self) -> u32 {
        if self.index >= N32 {
            self.refresh();
        }
        let v = self.state[self.index];
        self.index += 1;
        v
    }
    pub fn next_u64(&mut self) -> u64 {
        self.next_u32() as u64 | ((self.next_u32() as u64) << 32)
    }
}

/// SeedからSFMTの64bit出力列を一度だけ生成する共通ヘルパー。
///
/// 通常タイムライン検索と参照仕様検索で同じSFMT列を使うため、
/// 各モジュールが個別に生成処理を持たないようにする。
pub(crate) fn generate_u64_values(seed: u32, count: usize) -> Vec<u64> {
    let mut sfmt = Sfmt::new(seed);
    (0..count).map(|_| sfmt.next_u64()).collect()
}

/// Seedから指定した64bit出力位置まで進め、そこから必要な列だけを生成する。
///
/// 検索開始消費数より前の列を保持する必要はないため、絶対位置の大きな
/// Targetを扱う場合も、検索窓だけをメモリへ置く。
pub(crate) fn generate_u64_values_from(seed: u32, start: usize, count: usize) -> Vec<u64> {
    let mut sfmt = Sfmt::new(seed);
    for _ in 0..start {
        sfmt.next_u64();
    }
    (0..count).map(|_| sfmt.next_u64()).collect()
}

// ---------------------------------------------------------------------------
// ModelStatusとTimelineの状態遷移
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelStatus {
    pub remain: Vec<i32>,
}
impl ModelStatus {
    pub fn new(models: usize) -> Self {
        Self {
            remain: vec![0; models.max(1)],
        }
    }
    #[allow(dead_code)]
    pub fn next_state(&mut self, rng: &mut Sfmt) -> (usize, Vec<u8>) {
        self.next_state_with(|| rng.next_u64())
    }

    pub(crate) fn next_state_with<F>(&mut self, mut next: F) -> (usize, Vec<u8>)
    where
        F: FnMut() -> u64,
    {
        let mut used = 0;
        let mut blink = Vec::new();
        for (n, s) in self.remain.iter_mut().enumerate() {
            if *s > 1 {
                *s -= 1;
                continue;
            }
            if *s < 0 {
                *s += 1;
                if *s == 0 {
                    *s = if next() % 3 == 0 { 36 } else { 30 };
                    used += 1;
                    // 直前の乱数は準備状態を完了させるだけで、表示上の瞬き記号には使わない。
                    // low-7-bit判定は準備状態を開始する条件であり、判定成立自体は瞬きではない。
                    blink.push(n as u8);
                }
                continue;
            }
            if next() & 0x7f == 0 {
                *s = -5;
            }
            used += 1;
        }
        (used, blink)
    }

    /// 指定モデルの瞬き有無だけを返す軽量版。全モデルの状態更新と
    /// SFMT消費は維持しつつ、毎tickのVec確保を避けるためTimeline検索で使う。
    pub(crate) fn next_state_target_with<F>(
        &mut self,
        target_model: usize,
        mut next: F,
    ) -> (usize, bool)
    where
        F: FnMut() -> u64,
    {
        let mut used = 0;
        let mut target_blinked = false;
        for (n, s) in self.remain.iter_mut().enumerate() {
            if *s > 1 {
                *s -= 1;
                continue;
            }
            if *s < 0 {
                *s += 1;
                if *s == 0 {
                    *s = if next() % 3 == 0 { 36 } else { 30 };
                    used += 1;
                    target_blinked |= n == target_model;
                }
                continue;
            }
            if next() & 0x7f == 0 {
                *s = -5;
            }
            used += 1;
        }
        (used, target_blinked)
    }
}

#[derive(Clone)]
pub struct Timeline {
    #[allow(dead_code)]
    pub start: i64,
    #[allow(dead_code)]
    pub blink_frames: Vec<i64>,
    pub target_tick: Option<i64>,
}

/// 1回の瞬きに対応する、観測用と本番用の情報。
///
/// `elapsed_frames` はゲーム内の経過時間（実機Frame）であり、
/// `sfmt_position` は本番区間の開始位置に使うSFMT消費位置である。
/// この2つは単位が違うため、同じ値として扱ってはいけない。
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub struct BlinkPoint {
    pub elapsed_frames: i64,
    pub sfmt_position: i64,
    /// この瞬きから次の瞬きまでの時間チェック回数。末尾の瞬きには存在しない。
    pub next_blink_ticks: Option<i64>,
}

/// 孵化後の瞬き観測専用タイムライン。NPCによる到達判定を含まない。
#[derive(Clone)]
#[allow(dead_code)]
pub struct BlinkTimeline {
    pub start: i64,
    /// 乱数列上の各瞬き。時間、SFMT位置、次の瞬きまでの間隔を同じ行で保持する。
    pub blinks: Vec<BlinkPoint>,
}

/// 孵化演出終了後からエンカウントまでの本番区間の結果。
#[derive(Clone, Copy)]
pub struct ProductionTimeline {
    pub target: i64,
    pub target_tick: Option<i64>,
}

/// 本番Timelineキャッシュに保持する1遷移分の行。
///
/// `frame_before`と`frame_after`はSFMT位置、`tick`は遷移終了時の経過1/30秒tickを
/// 表す。ModelStatusとSFMTカーソルはキャッシュ末尾だけに保持し、再初期化せずに
/// 延長できる大きさへ抑える。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProductionTimelinePoint {
    pub frame_before: i64,
    pub frame_after: i64,
    pub tick: i64,
}

/// TargetFrameから独立した、連続する本番Timeline。
///
/// handoff位置で一度だけ初期化し、延長時もModelStatusの状態を保持する。
/// Targetの更新は生成済みの行を検索するだけで行う。
#[derive(Clone)]
// ---------------------------------------------------------------------------
// 本番Timelineと観測キャッシュのモデル
// ---------------------------------------------------------------------------

pub struct ProductionTimelineCache {
    pub seed: u32,
    pub start: i64,
    pub models: usize,
    pub points: Vec<ProductionTimelinePoint>,
    frame_limit: i64,
    frame: i64,
    tick: i64,
    sfmt: Sfmt,
    status: ModelStatus,
}

impl ProductionTimelineCache {
    pub fn new(seed: u32, start: i64, models: usize, frame_limit: i64) -> Option<Self> {
        if start < 0 || models == 0 || frame_limit <= 0 {
            return None;
        }
        let mut sfmt = Sfmt::new(seed);
        for _ in 0..start {
            sfmt.next_u64();
        }
        Some(Self {
            seed,
            start,
            models,
            points: Vec::new(),
            frame_limit,
            frame: start,
            tick: 0,
            sfmt,
            status: ModelStatus::new(models),
        })
    }

    /// 開始位置から`frame_limit`分のSFMT位置を覆うまでキャッシュを延長する。
    /// Target指定時も小さなチャンクで延長し、短いTargetで全先読み領域を確保しない。
    pub fn extend_to(&mut self, target_frame: Option<i64>) {
        let limit_end = self.start.saturating_add(self.frame_limit);
        let requested_end = target_frame
            .unwrap_or(limit_end)
            .max(self.frame)
            .min(limit_end);
        let end = if target_frame.is_some() {
            self.frame
                .saturating_add(PRODUCTION_TIMELINE_CACHE_CHUNK_FRAMES)
                .min(requested_end)
        } else {
            requested_end
        };
        while self.frame < end && self.tick < 10_000_000 {
            let before = self.frame;
            let (used, _) = self.status.next_state(&mut self.sfmt);
            self.frame = self.frame.saturating_add(used as i64);
            self.tick = self.tick.saturating_add(1);
            self.points.push(ProductionTimelinePoint {
                frame_before: before,
                frame_after: self.frame,
                tick: self.tick,
            });
        }
    }

    /// Targetを表現するまで、または先読み上限に達するまで4096Frame単位で延長する。
    /// Target更新を遅延処理しつつ、連続したModelStatus状態を保持する。
    pub fn extend_until(&mut self, target: i64) {
        let target = target.min(self.start.saturating_add(self.frame_limit));
        while self.frame < target && self.tick < 10_000_000 {
            let previous_frame = self.frame;
            let previous_tick = self.tick;
            self.extend_to(Some(target));
            if self.frame == previous_frame && self.tick == previous_tick {
                break;
            }
        }
        // 正確に着地した後の遷移を1つ保持し、キャッシュ検索でも
        // 次のtick開始時にTarget境界を確認できるようにする。
        let has_boundary = self
            .points
            .last()
            .is_some_and(|point| point.frame_before == target);
        if self.frame == target && !has_boundary && self.tick < 10_000_000 {
            self.extend_to(Some(target.saturating_add(1)));
        }
    }

    pub fn target(&self, target: i64) -> Option<TimelineTarget> {
        if target < self.start {
            return None;
        }
        if target == self.start {
            return Some(TimelineTarget {
                exact_tick: Some(0),
                timeline_tick: 0,
                frame_before: self.start,
                frame_after: self.start,
            });
        }
        // 現在Frameを次の遷移前に確認するため、遷移がTargetへ正確に着地した場合は
        // 直前の遷移ではなく次のtick開始位置を優先する。
        let before_index = self
            .points
            .partition_point(|point| point.frame_before < target);
        if let Some(point) = self.points.get(before_index) {
            if point.frame_before == target {
                return Some(TimelineTarget {
                    exact_tick: Some(point.tick.saturating_sub(1)),
                    timeline_tick: point.tick.saturating_sub(1),
                    frame_before: target,
                    frame_after: target,
                });
            }
        }
        let after_index = self
            .points
            .partition_point(|point| point.frame_after < target);
        let point = self.points.get(after_index)?;
        if point.frame_after == target {
            return Some(TimelineTarget {
                exact_tick: Some(point.tick),
                timeline_tick: point.tick,
                frame_before: point.frame_before,
                frame_after: point.frame_after,
            });
        }
        (point.frame_before <= target && target < point.frame_after).then_some(TimelineTarget {
            exact_tick: None,
            timeline_tick: point.tick,
            frame_before: point.frame_before,
            frame_after: point.frame_after,
        })
    }

    /// 同じ連続Timeline上の後続位置を基準にTargetを検索する。
    /// 現在位置が進んでもModelStatusを再初期化せず状態を再利用する。
    pub fn target_from(&self, start_frame: i64, target: i64) -> Option<TimelineTarget> {
        let start_tick = self.tick_at_frame(start_frame)?;
        let absolute = self.target(target)?;
        if absolute.timeline_tick < start_tick {
            return None;
        }
        Some(TimelineTarget {
            exact_tick: absolute.exact_tick.map(|tick| tick - start_tick),
            timeline_tick: absolute.timeline_tick - start_tick,
            frame_before: absolute.frame_before,
            frame_after: absolute.frame_after,
        })
    }

    fn tick_at_frame(&self, frame: i64) -> Option<i64> {
        if frame == self.start {
            return Some(0);
        }
        let index = self
            .points
            .partition_point(|point| point.frame_after < frame);
        self.points
            .get(index)
            .filter(|point| point.frame_after == frame)
            .map(|point| point.tick)
    }
}

/// 通常Timeline上でTargetに対応する時刻を検索した結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimelineTarget {
    /// Targetと正確に一致するステップ。飛び越した場合はNone。
    pub exact_tick: Option<i64>,
    /// `frame_before <= target < frame_after`となる最初のステップ。
    /// 正確に一致した場合は`exact_tick`と同じ値になる。
    pub timeline_tick: i64,
    pub frame_before: i64,
    pub frame_after: i64,
}

// ---------------------------------------------------------------------------
// Target reachability and encounter/Rotom rules
// ---------------------------------------------------------------------------

/// 孵化後の瞬き位置から、記事の手順でエンカウント開始位置を求めた結果。
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub struct EncounterPlan {
    /// 瞬き観測で得た、Bボタン直後のSFMT位置。
    pub observed_position: i64,
    /// 追加消費後にロトム判定を行うSFMT位置。
    pub b_position: i64,
    /// ロトムの総消費（無言なら+1、お喋りなら+2）。ロトムを考慮しない場合は0。
    /// NPC初期読み込みとは分けて保持する。
    pub rotom_consumption: i64,
    /// ユーザーが測定して入力するB終了後のオフセット。
    pub offset: i64,
    /// 本番タイムラインの開始位置。
    pub encounter_start: i64,
}

#[allow(dead_code)]
pub fn simulate(seed: u32, start: i64, target: i64, models: usize) -> Timeline {
    let last = start.max(target).max(0) as usize;
    let values = generate_u64_stream(seed, last, models);
    simulate_with_stream(&values, 0, start, target, models)
}

fn generate_u64_stream(seed: u32, last_index: usize, extra: usize) -> Vec<u64> {
    let count = last_index.saturating_add(extra).saturating_add(1);
    generate_u64_values(seed, count)
}

fn simulate_with_stream(
    values: &[u64],
    values_start: i64,
    start: i64,
    target: i64,
    models: usize,
) -> Timeline {
    let mut cursor = start
        .checked_sub(values_start)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0);
    let mut status = ModelStatus::new(models);
    let mut frame = start;
    // 検索結果として必要なのは瞬き列と到達可否。全フレームの履歴を
    let mut blink_frames = Vec::new();
    let mut tick = 0;
    let mut target_tick = None;
    while frame <= target && tick < 10_000_000 {
        let before = frame;
        if before == target {
            target_tick = Some(tick);
        }
        let (used, blink) = status.next_state_with(|| {
            let value = *values
                .get(cursor)
                .expect("SFMT列が不足しています。検索範囲またはTarget Frameを確認してください。");
            cursor += 1;
            value
        });
        if !blink.is_empty() {
            blink_frames.push(before + used as i64);
        }
        frame += used as i64;
        tick += 1;
    }
    Timeline {
        start,
        blink_frames,
        target_tick,
    }
}

#[allow(dead_code)]
pub fn search(seed: u32, range: RangeInclusive<i64>, target: i64, models: usize) -> Vec<Timeline> {
    let last = (*range.end()).max(target).max(0) as usize;
    // SFMTは一度だけ624個単位で更新し、全開始Frameから同じ出力列を参照する。
    // 検索完了後にvaluesがスコープから外れるため、検索結果だけが残る。
    let values = generate_u64_stream(seed, last, models);
    // 各開始Frameは独立しているため並列化できる。IndexedParallelIteratorのcollect
    // により、結果の順番は従来の開始Frame順のまま維持される。
    range
        .into_par_iter()
        .map(|s| simulate_with_stream(&values, 0, s, target, models))
        .collect()
}

/// NPCを使わず、孵化後の瞬き観測だけを検索する。
///
/// `target` は観測列の先読み終点を決める値として参照するが、
/// NPCによる到達判定や観測候補の除外には使わない。
#[allow(dead_code)]
pub fn search_observation(
    seed: u32,
    range: RangeInclusive<i64>,
    target: i64,
    fps: f64,
) -> Vec<BlinkTimeline> {
    // Targetは本番での到達可否に使う値であり、観測候補を除外する条件ではない。
    // 検索範囲の終点までの各候補について、追加の瞬き列を照合できる長さを用意する。
    // Targetが終点より遠い場合だけ、先読み終点をTarget側へ延長する。
    let observation_limit = (*range.end())
        .max(target)
        .max(0)
        .saturating_add(OBSERVATION_LOOKAHEAD);
    let last = observation_limit as usize;
    let values = generate_u64_stream(seed, last, 1);
    range
        .into_par_iter()
        .map(|start| simulate_observation_with_stream(&values, start, observation_limit, fps))
        .collect()
}

/// NPCなしの瞬き観測を、実機の経過フレーム単位でシミュレーションする。
///
/// `ModelStatus::next_state_with` の戻り値はSFMT消費数であり、ゲーム内の
/// 経過フレーム数ではない。外部ツールのタイムライン処理と同様に、両者を
/// 別々に進めて保存する。
#[allow(dead_code)]
fn simulate_observation_with_stream(
    values: &[u64],
    start: i64,
    target: i64,
    _fps: f64,
) -> BlinkTimeline {
    let mut cursor = start.max(0) as usize;
    let mut elapsed_frames = 0;
    let mut blinks = Vec::new();

    // 最初の瞬きは基準点として扱う。最初の瞬きまでの時間は検索に含めない。
    blinks.push(BlinkPoint {
        elapsed_frames: 0,
        sfmt_position: start.max(0),
        next_blink_ticks: None,
    });

    while cursor <= target.max(0) as usize && blinks.len() < 10_000_000 {
        let value = *values
            .get(cursor)
            .expect("SFMT列が不足しています。検索範囲またはTarget Frameを確認してください。");
        let interval_ticks = blink_interval_ticks(value) as i64;
        let interval_game_frames = interval_ticks * GAME_FRAMES_PER_BLINK_TICK;
        elapsed_frames += interval_game_frames;
        cursor += 1;
        blinks.push(BlinkPoint {
            elapsed_frames,
            sfmt_position: cursor as i64,
            next_blink_ticks: Some(interval_ticks),
        });
    }

    for index in 0..blinks.len().saturating_sub(1) {
        blinks[index].next_blink_ticks = Some(
            (blinks[index + 1].elapsed_frames - blinks[index].elapsed_frames)
                / GAME_FRAMES_PER_BLINK_TICK,
        );
    }
    if let Some(last) = blinks.last_mut() {
        last.next_blink_ticks = None;
    }

    BlinkTimeline { start, blinks }
}

/// 観測で特定した孵化演出終了時点から、本番用NPCタイムラインを計算する。
pub fn production_timeline(
    seed: u32,
    start: i64,
    target: i64,
    models: usize,
) -> ProductionTimeline {
    let target_result = timeline_target(seed, start, target, models);
    ProductionTimeline {
        target,
        target_tick: target_result.and_then(|result| result.exact_tick),
    }
}

/// `start`から通常のModelStatusを`ticks`ステップ進めたSFMT位置を返す。
/// 1ステップは1/30秒で、外部ツールの表示上は2Fに相当する。
pub fn advance_timeline(seed: u32, start: i64, models: usize, ticks: i64) -> Option<i64> {
    if start < 0 || ticks < 0 || models == 0 {
        return None;
    }
    let mut sfmt = Sfmt::new(seed);
    for _ in 0..start {
        sfmt.next_u64();
    }
    let mut status = ModelStatus::new(models);
    let mut frame = start;
    for _ in 0..ticks {
        let (used, _) = status.next_state(&mut sfmt);
        frame = frame.checked_add(used as i64)?;
    }
    Some(frame)
}

fn advance_timeline_with_stream(
    values: &[u64],
    values_start: i64,
    start: i64,
    models: usize,
    ticks: i64,
) -> Option<i64> {
    if start < 0 || ticks < 0 || models == 0 {
        return None;
    }
    let mut cursor = start
        .checked_sub(values_start)
        .and_then(|value| usize::try_from(value).ok())?;
    let mut status = ModelStatus::new(models);
    let mut frame = start;
    for _ in 0..ticks {
        let (used, _) = status.next_state_with(|| {
            let value = values[cursor];
            cursor += 1;
            value
        });
        frame = frame.checked_add(used as i64)?;
    }
    Some(frame)
}

/// 連続する孵化瞬き位置の○×を、共有SFMT列から一括計算する。
/// `fastest_ticks`がSomeなら指定ステップ後の位置がTargetと一致するか、
/// Noneなら通常Timeline上でTargetを正確に踏むかを判定する。
pub fn target_reachability_range(
    seed: u32,
    range: RangeInclusive<i64>,
    target: i64,
    models: usize,
    fastest_ticks: Option<i64>,
) -> Vec<(i64, bool)> {
    if range.is_empty() || *range.start() < 0 || models == 0 {
        return Vec::new();
    }
    let end = *range.end();
    let extra = fastest_ticks
        .unwrap_or(1)
        .max(1)
        .saturating_mul(models as i64)
        .saturating_add(8) as usize;
    let values_start = *range.start();
    let last = end.max(target).max(values_start).max(0);
    let span = usize::try_from(last.saturating_sub(values_start)).unwrap_or(usize::MAX);
    let count = span.saturating_add(extra).saturating_add(1);
    let values = generate_u64_values_from(seed, usize::try_from(values_start).unwrap(), count);
    range
        .into_par_iter()
        .map(|start| {
            let reachable = match fastest_ticks {
                Some(ticks) => {
                    advance_timeline_with_stream(&values, values_start, start, models, ticks)
                        == Some(target)
                }
                None => simulate_with_stream(&values, values_start, start, target, models)
                    .target_tick
                    .is_some(),
            };
            (start, reachable)
        })
        .collect()
}

/// 外部ツールの`CalcFrame`／通常Timelineと同じModelStatus経路でTarget時刻を探す。
/// 正確に一致しない場合も、Targetを初めてまたぐステップを返す。
pub fn timeline_target(
    seed: u32,
    start: i64,
    target: i64,
    models: usize,
) -> Option<TimelineTarget> {
    if start < 0 || target < start || models == 0 {
        return None;
    }
    let mut sfmt = Sfmt::new(seed);
    for _ in 0..start {
        sfmt.next_u64();
    }
    let mut status = ModelStatus::new(models);
    let mut frame = start;
    for tick in 0..10_000_000_i64 {
        if frame == target {
            return Some(TimelineTarget {
                exact_tick: Some(tick),
                timeline_tick: tick,
                frame_before: frame,
                frame_after: frame,
            });
        }
        let before = frame;
        let (used, _) = status.next_state(&mut sfmt);
        frame = frame.checked_add(used as i64)?;
        if before <= target && target < frame {
            return Some(TimelineTarget {
                exact_tick: None,
                // Targetをまたぐのはこの遷移の終了時点なので、経過tickはループ添字より1多い。
                timeline_tick: tick + 1,
                frame_before: before,
                frame_after: frame,
            });
        }
    }
    None
}

/// 指定したSFMT範囲を、観測後の確認用表へ変換する。
#[allow(dead_code)]
pub fn blink_table(seed: u32, range: RangeInclusive<i64>, fps: f64) -> Vec<BlinkTableRow> {
    if *range.end() < 0 {
        return Vec::new();
    }
    let start = (*range.start()).max(0) as usize;
    let end = (*range.end()).max(0) as usize;
    if end < start {
        return Vec::new();
    }
    let values = generate_u64_stream(seed, end.saturating_add(1), 1);
    (start..=end)
        .map(|position| {
            let value = values[position];
            let rotom_roll = values
                .get(position.saturating_add(1))
                .map(|next| (next % 100) as u8);
            BlinkTableRow {
                frame: position as i64,
                duration_seconds: blink_interval_seconds(value, fps),
                rotom_roll,
                random_value: value,
            }
        })
        .collect()
}

/// 記事に記載された、孵化後の瞬き位置から本番開始位置への補正を計算する。
///
/// 観測位置はBボタン直後の位置B。追加消費を適用した位置の64bit SFMT値を使い、
/// 乱数値%100が閾値未満ならロトム総消費を+2、そうでなければ+1とする。
/// NPC初期読み込みはロトム消費の後に加える。
/// オフセットは時間量なので、SFMT位置へ直接加算しない。
#[allow(dead_code)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::timing::seconds_to_blink_ticks;
    #[test]
    fn sfmt_reference() {
        let mut s = Sfmt::new(1);
        let x = [
            0x99CB63CF56A0FAA4u64,
            0xF56A5F31D9AFB700,
            0x8E862E8A4187D5CF,
            0x5EF96EADFEAB0285,
            0xCA84E80C31451224,
            0x3ECC65A8E0158BED,
            0x679CBCDB49C98DEA,
            0x9F6D633A4BA1800E,
            0x42583D2617861D00,
            0x6F549688380241BD,
        ];
        for v in x {
            assert_eq!(s.next_u64(), v);
        }
    }

    #[test]
    fn saved_sequence_reference() {
        // 参照列はアプリケーション資産ではなくテストfixtureとして保持する。
        // `include_str!`により、テスト実行時の作業ディレクトリにも依存しない。
        let text = include_str!("../../tests/fixtures/SEED一覧.txt");
        let mut s = Sfmt::new(1);
        let mut count = 0;
        for line in text.lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() < 4 {
                continue;
            }
            let expected = u64::from_str_radix(fields[3], 16).unwrap();
            assert_eq!(s.next_u64(), expected, "saved row {}", fields[0]);
            count += 1;
        }
        assert_eq!(count, 478);
    }

    #[test]
    fn cached_stream_matches_independent_simulation() {
        let seed = 1;
        let start = 37;
        let target = 137;
        let models = 3;

        let mut main = Sfmt::new(seed);
        for _ in 0..start {
            main.next_u64();
        }
        let mut blink_rng = main.clone();
        let mut status = ModelStatus::new(models);
        let mut frame = start;
        let mut tick = 0;
        let mut direct_blinks = Vec::new();
        while frame <= target && tick < 10_000_000 {
            let before = frame;
            let (used, blink) = status.next_state(&mut blink_rng);
            if !blink.is_empty() {
                direct_blinks.push(before + used as i64);
            }
            for _ in 0..used {
                main.next_u64();
            }
            frame += used as i64;
            tick += 1;
        }

        let cached = simulate(seed, start, target, models);
        assert_eq!(cached.blink_frames, direct_blinks);
    }

    #[test]
    fn observation_search_does_not_depend_on_npc_count() {
        let seed = 1;
        let start = 37;
        let target = 137;
        let observation_target = target + OBSERVATION_LOOKAHEAD;
        let values = generate_u64_stream(seed, observation_target as usize, 1);
        let expected =
            simulate_observation_with_stream(&values, start, observation_target, 59.8621)
                .blinks
                .iter()
                .map(|blink| blink.elapsed_frames)
                .collect::<Vec<_>>();
        let result = search_observation(seed, start..=start, target, 59.8621);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0]
                .blinks
                .iter()
                .map(|blink| blink.elapsed_frames)
                .collect::<Vec<_>>(),
            expected
        );
        assert!(result[0].blinks.windows(2).all(|window| {
            window[0].next_blink_ticks
                == Some(
                    (window[1].elapsed_frames - window[0].elapsed_frames)
                        / GAME_FRAMES_PER_BLINK_TICK,
                )
        }));
    }

    #[test]
    fn production_starts_from_observation_end_position() {
        let seed = 1;
        let target = 137;
        let start = 37;
        let production = production_timeline(seed, start, target, 2);
        let direct = simulate(seed, start, target, 2);
        assert_eq!(production.target_tick, direct.target_tick);
    }

    #[test]
    fn production_timeline_cache_reuses_one_continuous_state() {
        let seed = 1;
        let start = 37;
        let models = 3;
        let mut cache = ProductionTimelineCache::new(seed, start, models, 1_000).unwrap();
        let target = advance_timeline(seed, start, models, 25).unwrap();
        cache.extend_until(target);
        let point_count = cache.points.len();
        let cached = cache.target(target).unwrap();
        let direct = timeline_target(seed, start, target, models).unwrap();

        assert_eq!(cached, direct);
        assert!(point_count > 0);
        cache.extend_until(target);
        assert_eq!(cache.points.len(), point_count);
        cache.extend_to(None);
        assert!(cache.points.len() > point_count);
    }

    #[test]
    fn visible_blink_events_include_cooldown_correction() {
        // 既知のSFMT seed 0x75EBF3BFに対する参照列の先頭部分を検証する。
        // 表示上の瞬き記号はlow-7-bitのトリガーから5tick後だが、表示遅延は
        // 瞬き間隔列を変えない。
        let values = generate_u64_stream(0x75EBF3BF, 2_000, 1);
        let mut status = ModelStatus::new(1);
        let mut cursor = 0usize;
        let mut elapsed = 0i64;
        let mut points = Vec::new();

        while cursor < values.len() && points.len() < 6 {
            let (used, blink) = status.next_state_with(|| {
                let value = values[cursor];
                cursor += 1;
                value
            });
            if !blink.is_empty() {
                points.push((elapsed * GAME_FRAMES_PER_BLINK_TICK, cursor as i64));
            }
            elapsed += 1;
            assert!(used <= 1);
        }

        let elapsed_frames = points
            .iter()
            .map(|(elapsed, _)| *elapsed)
            .collect::<Vec<_>>();
        let intervals = elapsed_frames
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .collect::<Vec<_>>();

        assert_eq!(&elapsed_frames[..4], &[532, 742, 1146, 1304]);
        assert_eq!(&intervals[..3], &[210, 404, 158]);
        assert_eq!(points[0].1, 263);
    }

    #[test]
    fn hatch_blink_interval_uses_one_u64_value() {
        let mut sfmt = Sfmt::new(1);
        let first = sfmt.next_u64();
        let expected = (first % BLINK_INTERVAL_MODULUS + BLINK_INTERVAL_BASE_TICKS) as i64;

        let timeline = search_observation(1, 0..=0, 0, 59.8621).remove(0);
        assert_eq!(timeline.blinks[0].elapsed_frames, 0);
        assert_eq!(timeline.blinks[0].sfmt_position, 0);
        assert_eq!(timeline.blinks[0].next_blink_ticks, Some(expected));
        assert_eq!(
            timeline.blinks[1].elapsed_frames,
            expected * GAME_FRAMES_PER_BLINK_TICK
        );
        assert!(timeline.blinks[1].next_blink_ticks.is_some());
        assert_eq!(timeline.blinks[1].sfmt_position, 1);
    }

    #[test]
    fn hatch_blink_interval_is_in_display_frame_range() {
        let timeline = search_observation(1, 0..=0, 0, 59.8621).remove(0);
        for blink in &timeline.blinks[..timeline.blinks.len() - 1] {
            assert!((130..=249).contains(&blink.next_blink_ticks.unwrap()));
        }
    }

    #[test]
    fn blink_table_uses_raw_u64_and_internal_seconds() {
        let rows = blink_table(1, 0..=1, 59.8621);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].frame, 0);
        assert_eq!(rows[0].random_value, 0x99CB63CF56A0FAA4);
        assert!((4.333..=8.3).contains(&rows[0].duration_seconds));
        assert_eq!(rows[0].rotom_roll, Some((rows[1].random_value % 100) as u8));
        assert_eq!(rows[1].random_value, 0xF56A5F31D9AFB700);
        assert!(rows[1].rotom_roll.is_some());
    }

    #[test]
    fn observed_seconds_are_compared_in_display_frames() {
        let fps = 59.8621;
        let ticks = 150_i64;
        let seconds = ticks as f64 * GAME_FRAMES_PER_BLINK_TICK as f64 / fps;
        assert_eq!(seconds_to_blink_ticks(seconds, fps), ticks);
        assert_eq!(
            crate::domain::timing::seconds_to_game_frames(300.0 / fps, fps).round() as i64,
            300,
            "観測間隔は表示Frame単位で保持する"
        );
    }

    #[test]
    fn observation_cache_scans_shared_values_without_timeline_duplicates() {
        let cache = generate_observation_cache(1, 0..=8, 8, 59.8621).unwrap();
        let range = cache.interval_seconds_range(0, 3).unwrap();
        assert_eq!(range.len(), 3);
        assert_eq!(range[0], cache.interval_seconds_at(0).unwrap().unwrap());
        let rows = cache.table_rows(0, 3).unwrap();
        let observed = rows
            .iter()
            .take(2)
            .map(|row| {
                crate::domain::timing::seconds_to_game_frames(row.duration_seconds, 59.8621).round()
                    as i64
            })
            .collect::<Vec<_>>();
        let matches = cache.find_matches(&observed, 0).unwrap();
        assert!(matches.iter().any(|matched| {
            matched.start_position == 0
                && matched.sfmt_position == 2
                && matched.next_blink_ticks
                    == seconds_to_blink_ticks(rows[2].duration_seconds, 59.8621)
        }));
    }

    #[test]
    fn hatch_interval_tolerance_is_in_display_frames() {
        let cache = generate_observation_cache(1, 0..=8, 8, 59.8621).unwrap();
        let exact = cache
            .interval_ticks_at(0)
            .unwrap()
            .expect("first interval exists")
            * GAME_FRAMES_PER_BLINK_TICK;

        let within = cache.find_matches(&[exact + 5], 5).unwrap();
        assert!(within.iter().any(|matched| matched.start_position == 0));

        let outside = cache.find_matches(&[exact + 6], 5).unwrap();
        assert!(!outside.iter().any(|matched| matched.start_position == 0));
    }

    #[test]
    fn f4bae999_saved_seconds_match_the_hatch_formula() {
        let fps = 59.8621;
        let observed_seconds = [
            7.2771875, 7.7706219, 7.1247601, 7.1316971, 5.7158849, 6.2914987, 7.4083393, 7.8265522,
            4.3885160, 6.2150460, 4.9858013, 6.0072102, 6.6241911, 8.2498605,
        ];
        let observed = observed_seconds
            .iter()
            .map(|seconds| {
                crate::domain::timing::seconds_to_game_frames(*seconds, fps).round() as i64
            })
            .collect::<Vec<_>>();
        let cache = generate_observation_cache(0xF4BAE999, 0..=2_000, 2_000, fps).unwrap();
        let matches = cache.find_matches(&observed, 15).unwrap();
        let tolerance_counts = (0..=15)
            .map(|tolerance| {
                (
                    tolerance,
                    cache.find_matches(&observed, tolerance).unwrap().len(),
                )
            })
            .collect::<Vec<_>>();
        println!("F4BAE999 observed display frames: {observed:?}");
        println!("F4BAE999 candidates by tolerance: {tolerance_counts:?}");
        println!(
            "F4BAE999 matches: {} {:?}",
            matches.len(),
            matches
                .iter()
                .map(|item| item.start_position)
                .collect::<Vec<_>>()
        );
        if let Some(matched) = matches.first() {
            println!(
                "F4BAE999 next blink: {} ticks ({:.6} seconds)",
                matched.next_blink_ticks,
                matched.next_blink_ticks as f64 * GAME_FRAMES_PER_BLINK_TICK as f64 / fps
            );
            let predicted = cache
                .table_rows(matched.start_position, observed.len())
                .unwrap()
                .into_iter()
                .map(|row| {
                    crate::domain::timing::seconds_to_game_frames(row.duration_seconds, fps).round()
                        as i64
                })
                .collect::<Vec<_>>();
            let errors = predicted
                .iter()
                .zip(&observed)
                .map(|(actual, expected)| actual - expected)
                .collect::<Vec<_>>();
            println!("F4BAE999 predicted display frames: {predicted:?}");
            println!("F4BAE999 errors: {errors:?}");
        }
        assert!(!matches.is_empty());
    }

    #[test]
    fn encounter_plan_applies_rotom_total_consumption() {
        let mut sfmt = Sfmt::new(1);
        let b_value = sfmt.next_u64();
        let plan = encounter_plan(1, 0, 17).unwrap();
        let expected_rotom = if b_value % 100 < 79 { 2 } else { 1 };
        assert_eq!(plan.b_position, 0);
        assert_eq!(plan.rotom_consumption, expected_rotom);
        assert_eq!(plan.offset, 17);
        assert_eq!(plan.encounter_start, expected_rotom + 1);
    }

    #[test]
    fn encounter_plan_uses_b_value_for_talk_and_applies_rotom_extra_consumption() {
        let seed = 1;
        let mut sfmt = Sfmt::new(seed);
        let b_value = sfmt.next_u64();
        let expected_talk = b_value % 100 < 79;

        let predicted = encounter_plan_with_observed_rotom(seed, 0, 0, true, 79, None).unwrap();
        let forced_silent =
            encounter_plan_with_observed_rotom(seed, 0, 0, true, 79, Some(false)).unwrap();
        let forced_talk =
            encounter_plan_with_observed_rotom(seed, 0, 0, true, 79, Some(true)).unwrap();

        assert_eq!(
            predicted.rotom_consumption,
            if expected_talk { 2 } else { 1 }
        );
        assert_eq!(forced_silent.rotom_consumption, 1);
        assert_eq!(forced_silent.encounter_start, 2);
        assert_eq!(forced_talk.rotom_consumption, 2);
        assert_eq!(forced_talk.encounter_start, 3);
    }

    #[test]
    fn rotom_is_decided_before_npc_initial_load() {
        let seed = 1;
        let b_roll = rotom_roll_at(seed, 0).unwrap();
        let expected_total = 1 + i64::from(b_roll < 79);
        let plan =
            encounter_plan_with_observed_rotom_and_initial_load(seed, 0, 0, true, 79, None, 4)
                .unwrap();

        assert_eq!(plan.b_position, 0);
        assert_eq!(plan.rotom_consumption, expected_total);
        assert_eq!(plan.encounter_start, expected_total + 4);
    }

    #[test]
    fn pre_rotom_consumption_moves_rotom_and_initial_load_in_order() {
        let seed = 1;
        let pre = 5;
        let plan =
            encounter_plan_with_pre_rotom_consumption(seed, 0, 0, true, 79, None, pre, 4).unwrap();
        let roll = rotom_roll_at(seed, pre).unwrap();
        let rotom = 1 + i64::from(roll < 79);

        assert_eq!(plan.b_position, pre);
        assert_eq!(plan.rotom_consumption, rotom);
        assert_eq!(plan.encounter_start, pre + rotom + 4);
    }

    #[test]
    fn encounter_plan_can_disable_the_npc_initial_load() {
        let with_initial_load =
            encounter_plan_with_observed_rotom_and_initial_load(1, 0, 0, false, 79, None, 1)
                .unwrap();
        let without_initial_load =
            encounter_plan_with_observed_rotom_and_initial_load(1, 0, 0, false, 79, None, 0)
                .unwrap();
        assert_eq!(with_initial_load.b_position, 0);
        assert_eq!(without_initial_load.b_position, 0);
        assert_eq!(with_initial_load.encounter_start, 1);
        assert_eq!(without_initial_load.encounter_start, 0);
    }

    #[test]
    fn batched_reachability_matches_individual_timeline_simulation() {
        let seed = 1;
        let target = 40;
        let batch = target_reachability_range(seed, 10..=20, target, 2, None);
        for (start, reachable) in batch {
            assert_eq!(
                reachable,
                production_timeline(seed, start, target, 2)
                    .target_tick
                    .is_some()
            );
        }
    }

    #[test]
    fn batched_reachability_keeps_absolute_positions_with_a_relative_window() {
        let seed = 1;
        let start = 50_000;
        let target = start + 40;
        let batch = target_reachability_range(seed, start..=start + 10, target, 3, None);
        for (position, reachable) in batch {
            assert_eq!(
                reachable,
                production_timeline(seed, position, target, 3)
                    .target_tick
                    .is_some()
            );
        }
    }

    #[test]
    fn timeline_target_reports_crossing_at_end_of_step() {
        let result = timeline_target(1, 0, 1, 2).unwrap();
        assert_eq!(result.exact_tick, None);
        assert_eq!(result.timeline_tick, 1);
        assert_eq!((result.frame_before, result.frame_after), (0, 2));
    }
}
