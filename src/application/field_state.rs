//! フィールド観測のセッション状態と検索結果を管理する。
//!
//! UIから受け取った観測値を検索入力へ変換し、候補の確定、補正、
//! 本番タイマーへの引き渡しに必要な状態を一つのセッションとして保持する。

use super::*;
use crate::application::defaults;
use crate::application::state_types::SessionPhase;
use crate::domain::rng::CacheReadError;
use crate::errors::{AppError, AppResult, CacheError, InputField, StateIssue};
use std::sync::{Arc, Mutex};

const RESOLVED_TIMELINE_CHECKPOINT_TICKS: usize = 4_096;
const RESOLVED_TIMELINE_LOCK_BATCH_TICKS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TimelineBlinkEvent {
    tick: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TimelineCheckpoint {
    generated_ticks: usize,
    consumption: i64,
}

/// Session-scoped append-only progression shared by Current, table extension and Target queries.
/// The tick deltas are compact; blink events and sparse checkpoints provide indexed lookups.
struct ResolvedTimelineCache {
    identity: TimelineIdentity,
    cursor: TimelineCursor,
    sfmt: Sfmt,
    selected_model: usize,
    anchor_tick: i64,
    anchor_consumption: i64,
    tick_deltas: Vec<u8>,
    checkpoints: Vec<TimelineCheckpoint>,
    blinks: Vec<TimelineBlinkEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TimelineIdentity {
    seed: u32,
    start_consumption: i64,
    first_wait_frame: i64,
    last_wait_frame: i64,
    first_consumption: i64,
    last_consumption: i64,
    models: usize,
    observed_model: usize,
    intervals: Vec<i64>,
    target_consumptions: Vec<i64>,
}

impl ResolvedTimelineCache {
    fn new(
        identity: TimelineIdentity,
        cursor: TimelineCursor,
        sfmt: Sfmt,
        selected_model: usize,
    ) -> Self {
        let anchor_tick = cursor.tick;
        let anchor_consumption = cursor.consumption;
        Self {
            identity,
            cursor,
            sfmt,
            selected_model,
            anchor_tick,
            anchor_consumption,
            tick_deltas: Vec::new(),
            checkpoints: vec![TimelineCheckpoint {
                generated_ticks: 0,
                consumption: anchor_consumption,
            }],
            blinks: Vec::new(),
        }
    }

    fn append_chunk(&mut self, max_ticks: usize) -> usize {
        self.append_chunk_cancelable(max_ticks, None)
    }

    fn append_chunk_cancelable(
        &mut self,
        max_ticks: usize,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> usize {
        let initial = self.tick_deltas.len();
        let maximum_tick = crate::application::field::MAX_FUTURE_TIMELINE_TICKS;
        let maximum_consumption = crate::domain::rng::MAX_SFMT_FRAME;
        while self.tick_deltas.len().saturating_sub(initial) < max_ticks
            && self.cursor.tick < maximum_tick
            && self.cursor.consumption < maximum_consumption
        {
            if cancel.is_some_and(|token| token.load(std::sync::atomic::Ordering::Acquire)) {
                break;
            }
            let (used, consumed) = self.cursor.status.next_state_with_positions(|| {
                let position = self.cursor.stream_cursor;
                let value = self.sfmt.next_u64();
                self.cursor.stream_cursor = self.cursor.stream_cursor.saturating_add(1);
                (position, value)
            });
            self.cursor.consumption = self.cursor.consumption.saturating_add(used as i64);
            self.cursor.tick = self.cursor.tick.saturating_add(1);
            self.tick_deltas.push(used as u8);
            if consumed
                .iter()
                .any(|event| event.model == self.selected_model && event.blink)
            {
                self.blinks.push(TimelineBlinkEvent {
                    tick: self.cursor.tick as u32,
                });
            }
            let generated = self.tick_deltas.len();
            if generated % RESOLVED_TIMELINE_CHECKPOINT_TICKS == 0 {
                self.checkpoints.push(TimelineCheckpoint {
                    generated_ticks: generated,
                    consumption: self.cursor.consumption,
                });
            }
        }
        self.tick_deltas.len().saturating_sub(initial)
    }

    fn consumption_at_tick(&self, tick: i64) -> i64 {
        let generated_tick = tick.saturating_sub(self.anchor_tick).max(0) as usize;
        let generated_tick = generated_tick.min(self.tick_deltas.len());
        let checkpoint_index = self
            .checkpoints
            .partition_point(|checkpoint| checkpoint.generated_ticks <= generated_tick)
            .saturating_sub(1);
        let checkpoint = self.checkpoints[checkpoint_index];
        self.tick_deltas[checkpoint.generated_ticks..generated_tick]
            .iter()
            .fold(checkpoint.consumption, |sum, delta| sum + i64::from(*delta))
    }

    fn target_timing(&self, target: i64) -> Option<(i64, bool, i64, i64, usize)> {
        if target < self.anchor_consumption {
            return None;
        }
        if target > self.cursor.consumption {
            return None;
        }
        if target == self.anchor_consumption {
            return Some((self.anchor_tick * 2, true, target, target, 0));
        }
        let checkpoint_index = self
            .checkpoints
            .partition_point(|checkpoint| checkpoint.consumption < target)
            .saturating_sub(1);
        let checkpoint = self.checkpoints[checkpoint_index];
        let mut consumption = checkpoint.consumption;
        let mut blink_index = self.blinks.partition_point(|event| {
            i64::from(event.tick) <= self.anchor_tick + checkpoint.generated_ticks as i64
        });
        let mut blink_count = blink_index;
        let end = self.tick_deltas.len();
        for index in checkpoint.generated_ticks..end {
            let before = consumption;
            consumption += i64::from(self.tick_deltas[index]);
            let tick = self.anchor_tick + index as i64 + 1;
            if before <= target && target < consumption {
                return Some((tick * 2, false, before, consumption, blink_count));
            }
            if consumption == target {
                return Some((tick * 2, true, target, target, blink_count));
            }
            if self
                .blinks
                .get(blink_index)
                .is_some_and(|event| i64::from(event.tick) == tick)
            {
                blink_index += 1;
                blink_count += 1;
            }
        }
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
// ---------------------------------------------------------------------------
// フィールドタブのセッション状態とUI向け遷移
// ---------------------------------------------------------------------------

pub enum FieldMode {
    Idle,
    Observing,
    Correction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldTimerKind {
    NextBlink,
    Target,
}

#[derive(Clone)]
pub struct FieldState {
    mode: FieldMode,
    /// 観測対象idxを指定するか、全idxを候補として扱うか。
    pub target_mode: FieldTargetMode,
    pub seed: String,
    pub range_start: String,
    pub range_end: String,
    /// 「start +」で検索終点を決めるための幅。
    pub range_after_start: String,
    /// タイムライン仕様のNPC数欄。主人公1体は`config`内で暗黙に加算する。
    pub npc_count: String,
    /// NPC欄を`始点~終点`として展開する検索モード。
    pub multiple_npc_search: bool,
    /// 初期読み込み境界（-5分岐）を基準位置から探す幅。
    pub initial_branch_scan_span: String,
    /// 観測対象idx。0は主人公、1以上はNPC。
    pub target_model: String,
    /// 実際に取得できたTargetFrame。idx自動判定に使用する。
    pub actual_frame: String,
    pub tolerance: String,
    pub target_consumption: String,
    target_last_committed: String,
    /// Targetタイマーだけに適用する表示Frameオフセット。観測Timelineは変更しない。
    pub encounter_offset: String,
    pub use_offset: bool,
    /// TargetFrame欄を編集してから、Enterまたはフォーカス移動で再検索するための状態。
    pub target_dirty: bool,
    /// Targetだけを変更して、観測済みの候補を再評価している状態。
    /// この間も既存の瞬きタイマーは止めない。
    pub target_research_pending: bool,
    /// SFMT/Timeline上限まで進み、次の瞬きが範囲外になった状態。
    pub next_blink_out_of_range: bool,
    /// 明示的にEnterで補正へ進んだ後はTargetFrameを固定する。
    /// 観測結果確定直後からEnterまでは、瞬きタイマー実行中でも編集できる。
    target_input_locked: bool,
    pub observed: Vec<Instant>,
    pub intervals: Vec<i64>,
    pub results: Vec<FieldResult>,
    /// 指定範囲を照合して候補0件で終了した状態。追加のShiftでは再検索しない。
    pub search_exhausted: bool,
    /// 実測Frameから絞り込まれた観測対象idxの候補。
    pub inferred_indices: Vec<usize>,
    /// ずれ補正を最初に計算した結果を、同じ補正セッションで再利用する。
    pub correction_calculated: bool,
    pub correction_suggested_offset: Option<i64>,
    /// 指定idxで一致せず、別idxへフォールバックしたときの一致idx。
    pub idx_fallback_indices: Vec<usize>,
    /// idx指定時に、候補開始Frameのうち477以上で最も近いもの。
    pub nearest_timeline_frame: Option<i64>,
    pub selected_result: usize,
    timer_start: Option<Instant>,
    timer_seconds: f64,
    /// Targetタイマーが完了して表示を0へ固定している状態。
    timer_finished: bool,
    timer_kind: FieldTimerKind,
    /// ずれ検証へ入った瞬間のTimeline位置。補正中は観測アンカーの経過
    /// 時間ではなく、この位置を表のCurrentとして保持する。
    correction_timeline_frame: Option<i64>,
    /// 既に計算済みの次回以降の瞬き時刻。各瞬きのたびにTimelineを
    /// 再走査せず、観測タイマーを滑らかにつなぐために使う。
    blink_wait_queue: VecDeque<i64>,
    /// 右上表で前回スクロール追従した行。表示状態のリセットにも使う。
    pub table_last_followed_row: Option<usize>,
    /// 右上のTimeline表を現在位置へ自動追従させるか。初期値は有効。
    pub table_follow_current: bool,
    /// タイマーの音・表示位相を実機に合わせる表示Frame補正。
    pub adjust: i64,
    /// Field validation notices that are rendered in the observation panel.
    pub notice: Option<String>,
    observation_cache: Option<ObservationCache>,
    resolved_timeline_cache: Option<Arc<Mutex<ResolvedTimelineCache>>>,
    timeline_projection_through_tick: i64,
    last_current_sfmt_frame: Option<i64>,
    timeline_stream_error: Option<std::sync::Arc<AppError>>,
}

impl Default for FieldState {
    fn default() -> Self {
        Self {
            mode: FieldMode::Idle,
            target_mode: FieldTargetMode::Known,
            seed: defaults::SEED.into(),
            range_start: defaults::FIELD_RANGE_START.into(),
            range_end: defaults::FIELD_RANGE_END.into(),
            range_after_start: defaults::FIELD_RANGE_AFTER_START.into(),
            npc_count: defaults::FIELD_NPC_COUNT.into(),
            multiple_npc_search: false,
            initial_branch_scan_span: defaults::FIELD_INITIAL_BRANCH_SCAN_SPAN.into(),
            target_model: defaults::FIELD_TARGET_MODEL.into(),
            actual_frame: String::new(),
            tolerance: defaults::FIELD_TOLERANCE.into(),
            target_consumption: String::new(),
            target_last_committed: String::new(),
            encounter_offset: defaults::FIELD_ENCOUNTER_OFFSET.into(),
            use_offset: defaults::DEFAULT_USE_OFFSET,
            target_dirty: false,
            target_research_pending: false,
            next_blink_out_of_range: false,
            target_input_locked: false,
            observed: Vec::new(),
            intervals: Vec::new(),
            results: Vec::new(),
            search_exhausted: false,
            inferred_indices: Vec::new(),
            correction_calculated: false,
            correction_suggested_offset: None,
            idx_fallback_indices: Vec::new(),
            nearest_timeline_frame: None,
            selected_result: 0,
            timer_start: None,
            timer_seconds: 0.0,
            timer_finished: false,
            timer_kind: FieldTimerKind::NextBlink,
            correction_timeline_frame: None,
            blink_wait_queue: VecDeque::new(),
            table_last_followed_row: None,
            table_follow_current: defaults::DEFAULT_TABLE_FOLLOW_CURRENT,
            adjust: defaults::DEFAULT_ADJUST,
            notice: None,
            observation_cache: None,
            resolved_timeline_cache: None,
            timeline_projection_through_tick: 0,
            last_current_sfmt_frame: None,
            timeline_stream_error: None,
        }
    }
}

impl FieldState {
    /// 保存値またはUI確定値を、Target上限と同期して状態へ反映する。
    pub(crate) fn commit_target_input(&mut self) -> AppResult<()> {
        if self.target_consumption.trim().is_empty() {
            self.target_consumption.clear();
            self.target_last_committed.clear();
            self.target_dirty = false;
            return Ok(());
        }

        let parsed = match self.target_consumption.trim().parse::<i64>() {
            Ok(value) => value,
            Err(_) => {
                self.target_consumption = self.target_last_committed.clone();
                self.target_dirty = false;
                return Err(AppError::InvalidInteger(InputField::TargetConsumption));
            }
        };
        if parsed < 0 {
            self.target_consumption = self.target_last_committed.clone();
            self.target_dirty = false;
            return Err(AppError::NegativeValue(InputField::TargetConsumption));
        }
        if parsed > crate::domain::rng::MAX_SFMT_FRAME {
            self.target_consumption = self.target_last_committed.clone();
            self.target_dirty = false;
            return Err(AppError::OutOfRange {
                field: InputField::TargetConsumption,
                min: 0,
                max: crate::domain::rng::MAX_SFMT_FRAME,
            });
        }

        self.target_consumption = parsed.to_string();
        self.target_last_committed = self.target_consumption.clone();
        self.target_dirty = false;
        Ok(())
    }

    /// 永続化値は起動時にも上限を検証し、範囲外値を再計算へ渡さない。
    pub(crate) fn restore_target_input(&mut self, value: String) {
        self.target_consumption = value;
        self.target_last_committed.clear();
        if self.commit_target_input().is_err() {
            self.target_consumption.clear();
            self.target_last_committed.clear();
            self.target_dirty = false;
        }
    }

    pub(crate) fn set_candidate_not_found_notice(&mut self, message: impl Into<String>) {
        self.notice = Some(message.into());
    }

    pub(crate) fn set_search_range_notice(&mut self, message: impl Into<String>) {
        let message = message.into();
        if message.is_empty() {
            self.clear_notice();
        } else {
            self.notice = Some(message);
        }
    }

    pub(crate) fn clear_notice(&mut self) {
        self.notice = None;
    }

    /// 設定画面の初期分岐探索範囲を検証して返す。
    pub fn initial_branch_scan_span_value(&self) -> AppResult<i64> {
        let value = parse_nonnegative(
            &self.initial_branch_scan_span,
            InputField::InitialBranchSearchSpan,
        )?;
        if value > crate::application::field::MAX_INITIAL_BRANCH_SCAN_SPAN {
            return Err(AppError::OutOfRange {
                field: InputField::InitialBranchSearchSpan,
                min: 0,
                max: crate::application::field::MAX_INITIAL_BRANCH_SCAN_SPAN,
            });
        }
        Ok(value)
    }

    /// UIのNPC欄を単一または範囲として展開した検索設定。
    /// 主人公1体は各設定の`models`へ暗黙に加算する。
    pub fn configs(&self) -> AppResult<Vec<FieldConfig>> {
        let seed = u32::from_str_radix(self.seed.trim().trim_start_matches("0x"), 16)
            .map_err(|_| AppError::InvalidSeed)?;
        let range_start = parse_nonnegative(&self.range_start, InputField::SearchStart)?;
        let range_end = parse_nonnegative(&self.range_end, InputField::SearchEnd)?;
        let npc_counts = parse_npc_counts(&self.npc_count, self.multiple_npc_search)?;
        // NPC番号は検索開始時のTimelineを絞る条件にしないが、入力自体は必須とする。
        // 空欄を0へ暗黙変換すると、未入力のまま主人公として検索されるため、
        // UIの入力ミスと本当に0を指定したケースを区別できなくなる。
        let target_model = usize::try_from(parse_nonnegative(
            &self.target_model,
            InputField::NpcNumber,
        )?)
        .map_err(|_| AppError::NpcNumberTooLarge)?;
        let tolerance = parse_nonnegative(&self.tolerance, InputField::Tolerance)?;
        let target_consumption = if self.target_consumption.trim().is_empty() {
            None
        } else {
            Some(parse_nonnegative(
                &self.target_consumption,
                InputField::TargetConsumption,
            )?)
        };
        let branch_span = parse_nonnegative(
            &self.initial_branch_scan_span,
            InputField::InitialBranchSearchSpan,
        )?;
        if branch_span > crate::application::field::MAX_INITIAL_BRANCH_SCAN_SPAN {
            return Err(AppError::OutOfRange {
                field: InputField::InitialBranchSearchSpan,
                min: 0,
                max: crate::application::field::MAX_INITIAL_BRANCH_SCAN_SPAN,
            });
        }

        let mut configs = Vec::with_capacity(npc_counts.len());
        for npc_count in npc_counts {
            let models = npc_count.checked_add(1).ok_or(AppError::NpcCountTooLarge)?;
            if models > MAX_MODELS {
                return Err(AppError::OutOfRange {
                    field: InputField::NpcCount,
                    min: 0,
                    max: MAX_MODELS as i64 - 1,
                });
            }
            configs.push(FieldConfig {
                seed,
                range_start,
                range_end,
                models,
                target_model,
                observed_intervals: self.intervals.clone(),
                tolerance,
                target_consumption,
            });
        }
        if configs.is_empty() {
            return Err(AppError::NpcRangeDoesNotContainTarget);
        }
        for config in &configs {
            validate_static_config(config)?;
        }
        Ok(configs)
    }

    /// 単一設定を必要とする計算へ、展開後の先頭設定を返す。
    #[allow(dead_code)]
    pub fn config(&self) -> AppResult<FieldConfig> {
        self.configs()?
            .into_iter()
            .next()
            .ok_or(AppError::NpcRangeDoesNotContainTarget)
    }

    /// 観測列に一致した候補を、完全な現在状態が同じ場合だけ統合する。
    ///
    /// 異なる開始位置が観測対象の瞬きだけ同じになる場合、有限の観測列から
    /// 全Timelineの同一性は保証できない。そのため、保存済みカーソルがある候補は
    /// 全モデルの状態とSFMTカーソルが一致した場合だけ統合する。過去の対象NPCの
    /// SFMT位置列や間隔列は、現在状態が一致した候補を分ける理由にはしない。
    /// カーソルを持たない合成結果については内部状態を比較できないため、従来どおり
    /// 観測履歴と開始位置が完全一致する場合だけ統合する。
    pub(crate) fn normalize_search_results(
        &mut self,
        results: Vec<FieldResult>,
    ) -> Vec<FieldResult> {
        let requested = self.target_model.trim().parse::<usize>().ok();
        let mut groups: Vec<Vec<FieldResult>> = Vec::new();
        for result in results {
            let same_window = |candidate: &FieldResult| {
                let same_identity = candidate.models == result.models
                    && candidate.observed_model == result.observed_model;
                if !same_identity {
                    return false;
                }

                let has_full_state = candidate
                    .target_cursor
                    .as_ref()
                    .or(candidate.timeline_cursor.as_ref())
                    .is_some()
                    && result
                        .target_cursor
                        .as_ref()
                        .or(result.timeline_cursor.as_ref())
                        .is_some();
                if has_full_state {
                    // 現在状態が完全一致していれば、異なる開始位置や過去の
                    // target_consumptionsを持つ候補でも同じTimelineへ合流している。
                    same_full_timeline_state(candidate, &result)
                } else {
                    // 状態を保存していないテスト用・合成候補は、観測履歴まで
                    // 一致しなければ同一性を判定できない。
                    candidate.start_consumption == result.start_consumption
                        && candidate.intervals == result.intervals
                        && candidate.target_consumptions == result.target_consumptions
                        && same_full_timeline_state(candidate, &result)
                }
            };
            if let Some(group) = groups
                .iter_mut()
                .find(|group| group.first().is_some_and(same_window))
            {
                group.push(result);
            } else {
                groups.push(vec![result]);
            }
        }

        let requested_groups = requested
            .map(|index| {
                groups
                    .iter()
                    .enumerate()
                    .filter(|(_, group)| group.iter().any(|result| result.observed_model == index))
                    .map(|(group_index, _)| group_index)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let selected_group =
            (requested.is_some() && requested_groups.len() == 1).then(|| requested_groups[0]);
        let mut representatives = Vec::with_capacity(selected_group.map_or(groups.len(), |_| 1));
        for (group_index, group) in groups.iter().enumerate() {
            if selected_group.is_some_and(|selected| selected != group_index) {
                continue;
            }
            let representative = requested
                .and_then(|index| group.iter().find(|result| result.observed_model == index))
                .or_else(|| group.first())
                .cloned();
            if let Some(representative) = representative {
                representatives.push(representative);
            }
        }
        representatives.sort_by_key(|result| {
            (
                result.start_consumption,
                result.models,
                result.first_wait_frame,
                result.last_wait_frame,
            )
        });
        if representatives.len() == 1 {
            representatives[0].candidate_blinks.clear();
        }
        representatives
    }

    /// 確定済みTimelineの観測NPC番号と、条件設定のNPC番号を直接比較する。
    ///
    /// 表示用の派生値であり、検索やタイマーの状態として保存しない。
    pub(crate) fn selected_npc_number_match(&self) -> Option<bool> {
        let requested = self.target_model.trim().parse::<usize>().ok();
        match (requested, self.results.as_slice()) {
            (Some(requested), [result]) => Some(result.observed_model == requested),
            _ => None,
        }
    }

    fn enter_idle(&mut self) {
        self.mode = FieldMode::Idle;
    }

    fn enter_observing(&mut self) {
        self.mode = FieldMode::Observing;
    }

    /// フィールド側の内部状態を共通フェーズへ写像する。
    pub fn session_phase(&self) -> SessionPhase {
        if self.mode == FieldMode::Correction {
            return SessionPhase::Correction;
        }
        if self.timer_start.is_some() {
            return SessionPhase::Timer;
        }
        if self.timer_finished {
            return SessionPhase::Timer;
        }
        if self.mode == FieldMode::Observing {
            return SessionPhase::Observing;
        }
        if self.results.len() == 1 {
            return SessionPhase::TimelineReady;
        }
        SessionPhase::Input
    }

    pub fn can_edit_general_settings(&self) -> bool {
        matches!(
            self.session_phase(),
            SessionPhase::Input | SessionPhase::Observing | SessionPhase::TimelineReady
        )
    }

    /// 瞬き観測の範囲・許容差・観測対象は開始前に確定する。
    /// 観測後に変更する場合はリセットして新しいセッションを開始する。
    pub fn can_edit_observation_settings(&self) -> bool {
        self.session_phase() == SessionPhase::Input
    }

    pub fn can_edit_correction_inputs(&self) -> bool {
        self.session_phase() == SessionPhase::Correction
    }

    /// TargetFrameは観測結果確定後からEnterまでは編集でき、
    /// 明示的に補正モードへ進んだ後だけロックする。
    pub fn can_edit_target(&self) -> bool {
        !self.target_input_locked && self.mode != FieldMode::Correction
    }

    pub fn can_start_timer(&self) -> bool {
        self.session_phase() == SessionPhase::TimelineReady
    }

    pub fn is_observing(&self) -> bool {
        self.mode == FieldMode::Observing
    }

    pub fn is_correction_mode(&self) -> bool {
        self.mode == FieldMode::Correction
    }

    pub fn timer_is_running(&self) -> bool {
        self.timer_start.is_some()
    }

    pub fn timer_start_time(&self) -> Option<Instant> {
        self.timer_start
    }

    pub fn timer_kind(&self) -> FieldTimerKind {
        self.timer_kind
    }

    pub fn timer_is_finished(&self) -> bool {
        self.timer_finished
    }

    pub fn timer_seconds_value(&self) -> f64 {
        self.timer_seconds
    }

    /// 現在の瞬き区間を終了し、次の区間へ進めるための遷移。
    /// タイマーの開始時刻をUIから直接消去しないようにする。
    pub fn finish_blink_timer_segment(&mut self, fps: f64) -> AppResult<()> {
        self.timer_start = None;
        self.advance_blink_timer(fps)
    }

    /// Target区間を0秒で保持する終端遷移。以後の再開可否は、明示的な
    /// `toggle_timer_kind`または`enter_correction_mode`だけが決める。
    pub fn finish_target_timer(&mut self) {
        self.timer_finished = true;
    }

    /// 指定時刻までにTargetタイマーが経過していれば、0秒表示の完了状態へ移す。
    pub(crate) fn finish_target_timer_if_elapsed(&mut self, now: Instant) -> bool {
        if self.timer_kind != FieldTimerKind::Target
            || self.timer_finished
            || !self
                .timer_remaining_at(now)
                .is_some_and(|remaining| remaining <= 0.0)
        {
            return false;
        }
        self.finish_target_timer();
        true
    }

    /// 検索結果が一意になったときの遷移を一箇所へ集約する。
    pub fn mark_timeline_ready(&mut self) {
        if self.results.len() == 1 && self.mode != FieldMode::Correction {
            self.enter_idle();
        }
    }

    /// 検索スレッドの結果を現在のセッションへ反映できる状態か返す。
    ///
    /// 一意候補を得た後にTargetだけを変更した再計算も受け付ける。
    /// 通常のキャンセル後に古いスレッド結果を受け取らない条件は維持する。
    pub fn accepts_search_result(&self) -> bool {
        if self.target_research_pending {
            return true;
        }
        self.mode == FieldMode::Observing
            || (self.mode == FieldMode::Idle
                && self.results.is_empty()
                && self.intervals.len() >= crate::application::state::MIN_BLINK_INTERVALS
                && self.timer_start.is_none())
    }

    pub fn is_observation_input_enabled(&self) -> bool {
        // 候補0件で検索が終わっても、孵化側と同じく追加の瞬き入力を
        // 受け付けて再検索できるようにする。一意候補だけは停止する。
        self.mode == FieldMode::Observing && self.results.len() != 1
    }

    /// 入力条件が変わったとき、古い検索結果を現在の条件へ流用しない。
    /// 観測した時刻列は残すので、Target変更などは同じ観測から再検索できる。
    pub fn invalidate_search_results(&mut self) {
        self.results.clear();
        self.target_research_pending = false;
        self.search_exhausted = false;
        self.inferred_indices.clear();
        self.correction_calculated = false;
        self.correction_suggested_offset = None;
        self.idx_fallback_indices.clear();
        self.nearest_timeline_frame = None;
        self.selected_result = 0;
        self.timer_start = None;
        self.timer_finished = false;
        self.correction_timeline_frame = None;
        self.blink_wait_queue.clear();
        self.table_last_followed_row = None;
        self.observation_cache = None;
        self.resolved_timeline_cache = None;
        self.timeline_projection_through_tick = 0;
        self.last_current_sfmt_frame = None;
        self.timeline_stream_error = None;
    }

    /// Targetだけを再計算する。観測候補と瞬きタイマーは保持し、Target
    /// の計算中も次の瞬き通知を止めない。
    pub fn prepare_target_research(&mut self, fps: f64) {
        if self.timer_kind == FieldTimerKind::Target {
            self.timer_kind = FieldTimerKind::NextBlink;
            if self.results.len() == 1 {
                let _ = self.start_blink_timer(fps);
            } else {
                self.timer_start = None;
            }
        }
        for result in &mut self.results {
            result.target_wait_frame = None;
            result.target_exact = None;
            result.target_frame_before = None;
            result.target_frame_after = None;
            result.target_blink_count = None;
        }
        self.target_research_pending = true;
        self.search_exhausted = false;
        self.correction_calculated = false;
        self.correction_suggested_offset = None;
    }

    /// Target欄を空に戻した場合は、再検索せず既存Timelineを使い続ける。
    pub fn clear_target_prediction(&mut self, fps: f64) {
        self.target_research_pending = false;
        self.correction_calculated = false;
        self.correction_suggested_offset = None;
        if self.timer_kind == FieldTimerKind::Target {
            self.timer_kind = FieldTimerKind::NextBlink;
            if self.results.len() == 1 {
                let _ = self.start_blink_timer(fps);
            }
        }
        for result in &mut self.results {
            result.target_wait_frame = None;
            result.target_exact = None;
            result.target_frame_before = None;
            result.target_frame_after = None;
            result.target_blink_count = None;
        }
    }

    pub fn refresh_nearest_timeline_frame(&mut self) {
        self.nearest_timeline_frame =
            if self.target_mode == FieldTargetMode::Known && self.results.len() == 1 {
                self.results
                    .iter()
                    .filter(|result| result.start_consumption >= MIN_TIMELINE_FRAME)
                    .map(|result| result.start_consumption)
                    .min()
            } else {
                None
            };
    }

    pub fn start_observation(&mut self) {
        self.enter_observing();
        self.clear_notice();
        self.observed.clear();
        self.intervals.clear();
        self.results.clear();
        self.search_exhausted = false;
        self.inferred_indices.clear();
        self.correction_calculated = false;
        self.correction_suggested_offset = None;
        self.idx_fallback_indices.clear();
        self.nearest_timeline_frame = None;
        self.target_dirty = false;
        self.target_research_pending = false;
        self.next_blink_out_of_range = false;
        self.target_input_locked = false;
        self.selected_result = 0;
        self.timer_start = None;
        self.timer_finished = false;
        self.correction_timeline_frame = None;
        self.blink_wait_queue.clear();
        self.table_last_followed_row = None;
        self.observation_cache = None;
        self.resolved_timeline_cache = None;
        self.timeline_projection_through_tick = 0;
        self.last_current_sfmt_frame = None;
        self.timeline_stream_error = None;
    }

    pub fn cancel_observation(&mut self) {
        self.enter_idle();
        self.clear_notice();
        self.observed.clear();
        self.intervals.clear();
        self.results.clear();
        self.search_exhausted = false;
        self.inferred_indices.clear();
        self.correction_calculated = false;
        self.correction_suggested_offset = None;
        self.idx_fallback_indices.clear();
        self.nearest_timeline_frame = None;
        self.target_dirty = false;
        self.target_research_pending = false;
        self.next_blink_out_of_range = false;
        self.target_input_locked = false;
        self.selected_result = 0;
        self.timer_start = None;
        self.timer_finished = false;
        self.correction_timeline_frame = None;
        self.blink_wait_queue.clear();
        self.table_last_followed_row = None;
        self.observation_cache = None;
        self.resolved_timeline_cache = None;
        self.timeline_projection_through_tick = 0;
        self.last_current_sfmt_frame = None;
        self.timeline_stream_error = None;
    }

    pub(crate) fn set_observation_cache(&mut self, cache: ObservationCache) {
        self.observation_cache = Some(cache);
        self.resolved_timeline_cache = None;
        self.timeline_projection_through_tick = 0;
        self.last_current_sfmt_frame = None;
        self.timeline_stream_error = None;
    }

    pub(crate) fn observation_cache_for_seed(&self, seed: u32) -> Option<ObservationCache> {
        self.observation_cache
            .as_ref()
            .filter(|cache| cache.seed == seed)
            .cloned()
    }

    fn resolved_timeline_identity(seed: u32, result: &FieldResult) -> TimelineIdentity {
        TimelineIdentity {
            seed,
            start_consumption: result.start_consumption,
            first_wait_frame: result.first_wait_frame,
            last_wait_frame: result.last_wait_frame,
            first_consumption: result.first_consumption,
            last_consumption: result.last_consumption,
            models: result.models,
            observed_model: result.observed_model,
            intervals: result.intervals.clone(),
            target_consumptions: result.target_consumptions.clone(),
        }
    }

    fn ensure_resolved_timeline_cache(&mut self) -> AppResult<Arc<Mutex<ResolvedTimelineCache>>> {
        let seed = u32::from_str_radix(self.seed.trim().trim_start_matches("0x"), 16)
            .map_err(|_| AppError::InvalidSeed)?;
        let result = self
            .results
            .get(self.selected_result)
            .cloned()
            .ok_or(AppError::InvalidState(StateIssue::CandidateUnavailable))?;
        let identity = Self::resolved_timeline_identity(seed, &result);
        if let Some(shared) = &self.resolved_timeline_cache {
            let matches = shared
                .lock()
                .map(|cache| cache.identity == identity)
                .unwrap_or(false);
            if matches {
                return Ok(shared.clone());
            }
        }

        let cursor = result
            .target_cursor
            .as_ref()
            .or(result.timeline_cursor.as_ref())
            .cloned()
            .ok_or(AppError::InvalidState(
                StateIssue::TimelineCursorUnavailable,
            ))?;
        let cache = self
            .observation_cache
            .as_ref()
            .filter(|cache| cache.seed == seed)
            .ok_or(AppError::InvalidState(StateIssue::CacheUnavailable))?;
        let sfmt = cache
            .sfmt_at_checked(cursor.stream_cursor)
            .map_err(|error| match error {
                CacheReadError::OutOfRange => AppError::Cache(CacheError::OutOfRange),
                CacheReadError::Io { operation, source } => {
                    AppError::Cache(CacheError::Io { operation, source })
                }
            })?
            .ok_or(AppError::Cache(CacheError::StateUnavailable))?;
        let mut resolved =
            ResolvedTimelineCache::new(identity, cursor, sfmt, result.observed_model);
        let existing_end_tick = result
            .timeline_cursor
            .as_ref()
            .map(|cursor| cursor.tick)
            .unwrap_or(resolved.cursor.tick);
        while resolved.cursor.tick < existing_end_tick {
            let remaining = existing_end_tick.saturating_sub(resolved.cursor.tick) as usize;
            if resolved.append_chunk(remaining.min(RESOLVED_TIMELINE_CHECKPOINT_TICKS)) == 0 {
                break;
            }
        }
        self.timeline_projection_through_tick = existing_end_tick;
        let shared = Arc::new(Mutex::new(resolved));
        self.resolved_timeline_cache = Some(shared.clone());
        Ok(shared)
    }

    pub(crate) fn prepare_shared_timeline_cache(&mut self) -> AppResult<()> {
        self.ensure_resolved_timeline_cache().map(|_| ())
    }

    pub(crate) fn retarget_confirmed_result_from_cache(
        &mut self,
        target: i64,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> AppResult<FieldResult> {
        let result = self
            .results
            .get(self.selected_result)
            .cloned()
            .ok_or(AppError::InvalidState(StateIssue::CandidateUnavailable))?;
        if target < result.start_consumption || target < result.last_consumption {
            let mut result = result;
            result.target_wait_frame = None;
            result.target_exact = Some(false);
            result.target_frame_before = Some(target);
            result.target_frame_after = Some(result.last_consumption);
            result.target_blink_count = None;
            return Ok(result);
        }
        let shared = self.ensure_resolved_timeline_cache()?;
        loop {
            if cancel.is_some_and(|token| token.load(std::sync::atomic::Ordering::Acquire)) {
                return Err(AppError::Cancelled);
            }
            let Ok(mut cache) = shared.lock() else {
                return Err(AppError::InvalidState(
                    StateIssue::TimelineCursorUnavailable,
                ));
            };
            if let Some(timing) = cache.target_timing(target) {
                return Ok(crate::application::field::apply_cached_target_timing(
                    result,
                    target,
                    Some(timing),
                ));
            }
            let appended =
                cache.append_chunk_cancelable(RESOLVED_TIMELINE_LOCK_BATCH_TICKS, cancel);
            drop(cache);
            std::thread::yield_now();
            if appended == 0 {
                return Ok(crate::application::field::apply_cached_target_timing(
                    result, target, None,
                ));
            }
        }
    }

    /// 検索始点から指定幅を足して、検索終点を設定する。
    pub fn set_range_from_start(&mut self) {
        let Ok(start) = self.range_start.trim().parse::<i64>() else {
            return;
        };
        let Ok(span) = self.range_after_start.trim().parse::<i64>() else {
            return;
        };
        if !(0..=MAX_SEARCH_FRAME).contains(&start) || !(0..=MAX_SEARCH_SPAN).contains(&span) {
            return;
        }
        let Some(end) = start.checked_add(span) else {
            return;
        };
        if end > MAX_SEARCH_FRAME {
            return;
        }
        self.range_end = end.to_string();
    }

    pub(crate) fn take_timeline_stream_error(&mut self) -> Option<std::sync::Arc<AppError>> {
        self.timeline_stream_error.take()
    }

    /// 共有済みTimelineのtick消費差分から、現在のSFMT消費位置を引く。
    /// UI更新のたびにModelStatusやSFMTを別カーソルで再実行しない。
    pub fn current_sfmt_frame(&mut self, fps: f64) -> Option<i64> {
        let current_frame = self.current_timeline_frame(fps)?;
        let shared = match self.resolved_timeline_cache.clone() {
            Some(shared) => shared,
            None => match self.ensure_resolved_timeline_cache() {
                Ok(shared) => shared,
                Err(error) => {
                    self.timeline_stream_error = Some(Arc::new(error));
                    return None;
                }
            },
        };
        let current = match shared.try_lock() {
            Ok(cache) => {
                let frame = cache
                    .consumption_at_tick(current_frame.max(cache.anchor_tick * 2) / 2)
                    .min(crate::domain::rng::MAX_SFMT_FRAME);
                self.last_current_sfmt_frame = Some(frame);
                Some(frame)
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                self.last_current_sfmt_frame.or_else(|| {
                    self.results
                        .get(self.selected_result)
                        .map(|result| result.last_consumption)
                })
            }
            Err(std::sync::TryLockError::Poisoned(_)) => None,
        };
        current
    }

    /// 現在位置が先読み末尾へ近づいたとき、選択候補のTimelineを延長する。
    /// 1回の呼び出しでは有限チャンクだけを計算し、長時間待機でもUIスレッドに
    /// 大きな計算を集中させない。
    pub(crate) fn extend_timeline_if_needed(&mut self, fps: f64) -> AppResult<bool> {
        if self.results.len() != 1 || self.is_correction_mode() {
            return Ok(false);
        }
        let shared = self.ensure_resolved_timeline_cache()?;
        let Some(current) = self.current_timeline_frame(fps) else {
            return Ok(false);
        };
        if self.results.get(self.selected_result).is_none() {
            return Ok(false);
        }

        let display_end_tick = current
            .div_euclid(2)
            .saturating_add(TIMELINE_CHUNK_TICKS * 2);
        let ticks_needed = match shared.lock() {
            Ok(cache) => display_end_tick.saturating_sub(cache.cursor.tick).max(0) as usize,
            Err(_) => return Ok(false),
        };
        let mut ticks_remaining = ticks_needed.min(TIMELINE_CHUNK_TICKS as usize);
        while ticks_remaining > 0 {
            let Ok(mut cache) = shared.lock() else {
                return Ok(false);
            };
            let appended =
                cache.append_chunk(ticks_remaining.min(RESOLVED_TIMELINE_LOCK_BATCH_TICKS));
            drop(cache);
            std::thread::yield_now();
            if appended == 0 {
                break;
            }
            ticks_remaining = ticks_remaining.saturating_sub(appended);
        }
        let Some(result) = self.results.get_mut(self.selected_result) else {
            return Ok(false);
        };
        let Ok(cache) = shared.lock() else {
            return Ok(false);
        };
        self.timeline_projection_through_tick = self
            .timeline_projection_through_tick
            .max(display_end_tick.min(cache.cursor.tick));
        let last_row_tick = result
            .timeline
            .last()
            .map_or(result.last_wait_frame / 2, |row| row.timeline_frame / 2);
        let first_event = cache
            .blinks
            .partition_point(|event| i64::from(event.tick) <= last_row_tick);
        let events = cache.blinks[first_event..]
            .iter()
            .take_while(|event| i64::from(event.tick) <= display_end_tick)
            .take(MAX_RETAINED_TIMELINE_ROWS + 1)
            .map(|event| {
                let tick = i64::from(event.tick);
                (tick * 2, cache.consumption_at_tick(tick))
            })
            .collect::<Vec<_>>();
        let cursor = cache.cursor.clone();
        let cache_at_limit =
            cursor.tick >= MAX_FUTURE_TIMELINE_TICKS || cursor.consumption >= MAX_SFMT_FRAME;
        let can_continue = !cache_at_limit || self.timeline_projection_through_tick < cursor.tick;
        drop(cache);
        for (wait_frame, consumption) in events {
            if let Some(previous) = result.timeline.last_mut() {
                previous.duration_frames = wait_frame.saturating_sub(previous.timeline_frame);
            }
            result.timeline.push(FieldTimelineRow {
                frame: consumption,
                timeline_frame: wait_frame,
                duration_frames: 0,
            });
        }
        // Targetまでの瞬き数は、Target判定時に観測アンカーから全距離を走査して
        // 確定済み。表示用Timelineの先読み延長では再加算しない。
        trim_timeline_history(result);
        // 1チャンク内に対象の瞬きがなくても、SFMT／Timelineの上限に達して
        // いなければ次のチャンクへ進める。瞬きがなかったこと自体は終端ではない。
        result.timeline_cursor = Some(cursor);
        Ok(can_continue)
    }

    /// UIスレッドでTimeline延長を開始する前の軽量な判定。
    /// 実際のModelStatus/SFMT走査は呼び出し側のワーカーで行う。
    pub(crate) fn timeline_extension_needed(&self, fps: f64) -> bool {
        if self.results.len() != 1 || self.is_correction_mode() {
            return false;
        }
        let Some(current) = self.current_timeline_frame(fps) else {
            return false;
        };
        if self.results.get(self.selected_result).is_none() {
            return false;
        }
        self.timeline_projection_through_tick
            .saturating_sub(current.div_euclid(2))
            <= TIMELINE_CHUNK_TICKS * 2
    }

    /// バックグラウンドで延長した状態のうち、Timeline関連だけを反映する。
    /// 入力欄・観測列・タイマー状態はUI側の最新値を保持する。
    pub(crate) fn apply_timeline_extension(&mut self, extended: FieldState) {
        if self.results.len() != extended.results.len()
            || self.selected_result != extended.selected_result
        {
            return;
        }
        if self
            .results
            .iter()
            .zip(&extended.results)
            .any(|(current, extended)| {
                current.start_consumption != extended.start_consumption
                    || current.last_wait_frame != extended.last_wait_frame
                    || current.observed_model != extended.observed_model
            })
        {
            return;
        }
        let shared_cursor = self
            .resolved_timeline_cache
            .as_ref()
            .and_then(|shared| shared.lock().ok().map(|cache| cache.cursor.clone()));
        for (current, extended) in self.results.iter_mut().zip(extended.results) {
            current.timeline = extended.timeline;
            current.timeline_cursor = shared_cursor.clone().or(extended.timeline_cursor);
            current.next_blink_wait_frame = extended.next_blink_wait_frame;
        }
        self.nearest_timeline_frame = extended.nearest_timeline_frame;
        self.timeline_projection_through_tick = extended.timeline_projection_through_tick;
    }

    pub fn observe(&mut self, fps: f64) {
        if !self.is_observation_input_enabled() {
            return;
        }
        if self.observed.len() >= crate::application::state::MAX_BLINK_OBSERVATIONS {
            return;
        }
        let now = Instant::now();
        self.observed.push(now);
        if let Some(previous) = self.observed.iter().rev().nth(1) {
            self.intervals.push(
                seconds_to_game_frames(now.duration_since(*previous).as_secs_f64(), fps).round()
                    as i64,
            );
        }
    }

    /// 一致済み候補を観測した間隔1つ分だけ延長する。
    ///
    /// 候補には最後に一致した瞬き直後の状態が保存されているため、次の瞬きまで
    /// 進めればよく、Shift入力ごとにSFMT検索全体をやり直さずに済む。
    pub fn advance_candidates_by_interval(&mut self, expected: i64) -> usize {
        let tolerance = self.tolerance.trim().parse::<i64>().ok().unwrap_or(-1);
        if tolerance < 0 || self.results.is_empty() {
            return 0;
        }
        let mut survivors = Vec::with_capacity(self.results.len());
        for mut result in self.results.drain(..) {
            let previous_last = result.last_wait_frame;
            let Some(next_index) = result
                .candidate_blinks
                .iter()
                .position(|blink| blink.timeline_frame > previous_last && !blink.observed)
            else {
                continue;
            };
            let next = {
                let blink = &mut result.candidate_blinks[next_index];
                (blink.timeline_frame, blink.frame, blink.target_consumption)
            };
            let (next_wait, next_consumption, next_target_consumption) = next;
            let actual = next_wait.saturating_sub(previous_last);
            if (actual - expected).abs() > tolerance {
                continue;
            }

            let target_absolute = result
                .target_wait_frame
                .map(|wait| previous_last.saturating_add(wait));
            result.candidate_blinks[next_index].observed = true;
            result.last_wait_frame = next_wait;
            result.last_consumption = next_consumption;
            result.target_blink_count_offset_from_first_row = result
                .target_blink_count_offset_from_first_row
                .saturating_add(1);
            result.intervals.push(actual);
            if let Some(target_consumption) = next_target_consumption {
                result.target_consumptions.push(target_consumption);
            }
            result.next_blink_wait_frame = result
                .candidate_blinks
                .iter()
                .find(|blink| blink.timeline_frame > next_wait)
                .map(|blink| blink.timeline_frame - next_wait)
                .unwrap_or_default();
            result.target_wait_frame = target_absolute
                .and_then(|absolute| (absolute >= next_wait).then_some(absolute - next_wait));
            if let Some(absolute) = target_absolute {
                result.target_blink_count = Some(if next_wait < absolute {
                    result
                        .target_blink_count
                        .unwrap_or_default()
                        .saturating_sub(1)
                } else {
                    0
                });
            }
            survivors.push(result);
        }
        self.results = survivors;
        if self.results.len() == 1 {
            self.results[0].candidate_blinks.clear();
        }
        self.selected_result = 0;
        self.table_last_followed_row = None;
        self.refresh_nearest_timeline_frame();
        self.results.len()
    }

    pub fn start_blink_timer(&mut self, fps: f64) -> AppResult<()> {
        if self.mode == FieldMode::Correction {
            return Err(AppError::InvalidState(StateIssue::CorrectionModeLocksTimer));
        }
        if self.results.len() != 1 {
            return Err(AppError::InvalidState(StateIssue::CandidatesNotUnique));
        }
        if self.timeline_limit_reached() {
            self.stop_blink_timer_out_of_range();
            return Err(AppError::InvalidState(StateIssue::TimelineLimitReached));
        }
        if self
            .results
            .get(self.selected_result)
            .is_some_and(|result| {
                result.timeline_cursor.is_some() || result.target_cursor.is_some()
            })
        {
            self.prepare_shared_timeline_cache()?;
        }
        self.rebuild_blink_timer_queue(fps);
        if self.blink_wait_queue.is_empty() && self.timeline_limit_reached() {
            self.stop_blink_timer_out_of_range();
            return Err(AppError::InvalidState(StateIssue::TimelineLimitReached));
        }
        self.start_blink_timer_from_queue(fps)?;
        self.next_blink_out_of_range = false;
        Ok(())
    }

    fn timeline_limit_reached(&self) -> bool {
        if let Some(shared) = &self.resolved_timeline_cache {
            if let Ok(cache) = shared.lock() {
                if cache.cursor.consumption >= crate::domain::rng::MAX_SFMT_FRAME
                    || cache.cursor.tick >= MAX_FUTURE_TIMELINE_TICKS
                {
                    return true;
                }
            }
        }
        self.results
            .get(self.selected_result)
            .and_then(|result| result.timeline_cursor.as_ref())
            .is_some_and(|cursor| {
                cursor.consumption >= crate::domain::rng::MAX_SFMT_FRAME
                    || cursor.tick >= MAX_FUTURE_TIMELINE_TICKS
            })
    }

    /// 検索上限まで到達し、次の瞬きがないと確定したときにタイマーを停止する。
    pub(crate) fn stop_blink_timer_out_of_range(&mut self) {
        self.timer_start = None;
        self.timer_seconds = 0.0;
        self.timer_finished = false;
        self.next_blink_out_of_range = true;
        self.clear_notice();
    }

    /// 明示的なずれ検証操作でのみ呼び出す。押下時の現在位置を保存して
    /// Timeline表を停止し、実測Frame・idx入力を解放する。
    pub fn enter_correction_mode(&mut self, fps: f64) {
        self.correction_timeline_frame = self.current_timeline_frame(fps);
        self.mode = FieldMode::Correction;
        self.target_input_locked = true;
        self.correction_calculated = false;
        self.correction_suggested_offset = None;
        self.inferred_indices.clear();
        self.timer_start = None;
        self.timer_finished = false;
        self.clear_notice();
    }

    fn rebuild_blink_timer_queue(&mut self, fps: f64) {
        let result = self.results.get(self.selected_result).cloned();

        let Some(result) = result else {
            self.blink_wait_queue.clear();
            return;
        };
        let current = self
            .current_timeline_frame(fps)
            .unwrap_or(result.last_wait_frame);
        let mut queue = self.cached_blink_waits_after(
            current,
            result.last_wait_frame,
            BLINK_TIMER_PREFETCH_COUNT,
        );
        if queue.is_empty()
            && result.timeline_cursor.is_none()
            && result.next_blink_wait_frame > 0
            && result
                .last_wait_frame
                .saturating_add(result.next_blink_wait_frame)
                > current
        {
            queue.push(result.next_blink_wait_frame);
        }
        self.blink_wait_queue = queue.into();
    }

    /// 先読みキューが空になる前に、既に生成済みのTimeline行を補充する。
    /// Timeline延長ワーカーが完了した次のUI更新でも同じ処理が走るため、
    /// 末尾到達を待ってから全キューを作り直すことはない。
    pub fn maintain_blink_timer_queue(&mut self, fps: f64) {
        if self.results.len() != 1 || self.mode == FieldMode::Correction {
            return;
        }
        // Targetタイマー中も次の瞬き表示を途切れさせないよう、消化済みの
        // 区間を捨てて先読みキューを補充する。NextBlink中は完了処理側が
        // 先頭を1件だけ取り除くため、ここでは追加だけを行う。
        if self.timer_kind == FieldTimerKind::Target {
            while self
                .blink_wait_queue
                .front()
                .is_some_and(|wait| self.remaining_from_observation(*wait, fps).is_err())
            {
                self.blink_wait_queue.pop_front();
            }
        }
        if self.blink_wait_queue.is_empty() {
            self.rebuild_blink_timer_queue(fps);
            return;
        }
        if self.blink_wait_queue.len() > BLINK_TIMER_REFILL_THRESHOLD {
            return;
        }
        let Some(result) = self.results.get(self.selected_result) else {
            return;
        };
        let last_queued_absolute = result
            .last_wait_frame
            .saturating_add(self.blink_wait_queue.back().copied().unwrap_or_default());
        let remaining = BLINK_TIMER_PREFETCH_COUNT.saturating_sub(self.blink_wait_queue.len());
        let additions =
            self.cached_blink_waits_after(last_queued_absolute, result.last_wait_frame, remaining);
        self.blink_wait_queue.extend(additions);
    }

    fn cached_blink_waits_after(
        &self,
        absolute_frame: i64,
        anchor_frame: i64,
        limit: usize,
    ) -> Vec<i64> {
        if limit == 0 {
            return Vec::new();
        }
        let Some(shared) = &self.resolved_timeline_cache else {
            let Some(result) = self.results.get(self.selected_result) else {
                return Vec::new();
            };
            if result.timeline_cursor.is_some() {
                return Vec::new();
            }
            return result
                .timeline
                .iter()
                .filter(|row| row.timeline_frame > absolute_frame)
                .take(limit)
                .map(|row| row.timeline_frame - anchor_frame)
                .collect();
        };
        let Ok(cache) = shared.lock() else {
            return Vec::new();
        };
        let first = cache
            .blinks
            .partition_point(|event| i64::from(event.tick) * 2 <= absolute_frame);
        cache.blinks[first..]
            .iter()
            .take(limit)
            .map(|event| i64::from(event.tick) * 2 - anchor_frame)
            .collect()
    }

    /// Targetタイマー右上に表示する次の瞬きまでの残り時間。
    ///
    /// Target区間では瞬きタイマー自体を進めないため、表示専用の先読み
    /// キューから「まだ未来にある最初の区間」を選ぶ。区間の境界で古い
    /// 行が0秒になっても、次の行へ直ちに切り替わるので空表示にならない。
    pub fn preview_next_blink_remaining(&self, fps: f64) -> Option<f64> {
        for wait_frame in &self.blink_wait_queue {
            if let Ok(seconds) = self.remaining_from_observation(*wait_frame, fps) {
                return Some(seconds);
            }
        }
        let result = self.results.get(self.selected_result)?;
        let current = self.current_timeline_frame(fps)?;
        let wait_frame = self
            .cached_blink_waits_after(current, result.last_wait_frame, 1)
            .into_iter()
            .next()?;
        self.remaining_from_observation(wait_frame, fps).ok()
    }

    fn start_blink_timer_from_queue(&mut self, fps: f64) -> AppResult<()> {
        let wait_frame = self
            .blink_wait_queue
            .front()
            .copied()
            .ok_or(AppError::InvalidState(StateIssue::BlinkQueuePending))?;
        self.timer_seconds = self.remaining_from_observation(wait_frame, fps)?;
        self.timer_kind = FieldTimerKind::NextBlink;
        self.timer_start = Some(Instant::now());
        self.timer_finished = false;
        Ok(())
    }

    /// 現在のタイマー区間が終わった後、先読み済みの次の区間へ切り替える。
    /// Timelineがまだ延長中なら、キューを再構築してから一度だけ再試行する。
    pub fn advance_blink_timer(&mut self, fps: f64) -> AppResult<()> {
        if self.timer_kind != FieldTimerKind::NextBlink || self.results.len() != 1 {
            return Err(AppError::InvalidState(StateIssue::BlinkTimerCannotContinue));
        }
        self.blink_wait_queue.pop_front();
        self.maintain_blink_timer_queue(fps);
        match self.start_blink_timer_from_queue(fps) {
            Ok(()) => Ok(()),
            Err(AppError::InvalidState(StateIssue::BlinkQueuePending)) => {
                self.rebuild_blink_timer_queue(fps);
                self.start_blink_timer_from_queue(fps)
            }
            Err(error) => Err(error),
        }
    }

    pub fn start_target_timer(&mut self, fps: f64) -> AppResult<()> {
        if self.mode == FieldMode::Correction {
            return Err(AppError::InvalidState(StateIssue::CorrectionModeLocksTimer));
        }
        if self.results.len() != 1 {
            return Err(AppError::InvalidState(StateIssue::CandidatesNotUnique));
        }
        let result = self
            .results
            .get(self.selected_result)
            .cloned()
            .ok_or(AppError::InvalidState(StateIssue::CandidateUnavailable))?;
        if result.timeline_cursor.is_some() || result.target_cursor.is_some() {
            self.prepare_shared_timeline_cache()?;
        }
        let wait_frame = self
            .target_wait_frame_with_offset(&result)
            .ok_or(AppError::InvalidState(StateIssue::TargetUnreachable))?;
        self.timer_kind = FieldTimerKind::Target;
        match self.remaining_from_observation(wait_frame, fps) {
            Ok(seconds) => {
                self.next_blink_out_of_range = false;
                self.timer_seconds = seconds;
                self.timer_start = Some(Instant::now());
                self.timer_finished = false;
            }
            Err(_) => {
                // Target情報は残っているためTargetモードへは切り替えられる。
                // ただし既に通過しているので、0秒で固定して通知音は鳴らさない。
                self.timer_seconds = 0.0;
                self.timer_start = None;
                self.timer_finished = true;
            }
        }
        Ok(())
    }

    /// 表示オフセット更新後、実行中のTargetタイマーの基準時刻を取り直す。
    /// 瞬きタイマーと保持中のTimelineは変更しない。
    pub fn refresh_target_timer_for_offset(&mut self, fps: f64) {
        if self.timer_kind != FieldTimerKind::Target || self.timer_start.is_none() {
            return;
        }
        let Some(result) = self.results.get(self.selected_result) else {
            return;
        };
        let Some(wait_frame) = self.target_wait_frame_with_offset(result) else {
            return;
        };
        match self.remaining_from_observation(wait_frame, fps) {
            Ok(seconds) => {
                self.timer_seconds = seconds;
                self.timer_start = Some(Instant::now());
                self.timer_finished = false;
            }
            Err(_) => {
                self.timer_seconds = 0.0;
                self.timer_start = None;
                self.timer_finished = true;
            }
        }
    }

    pub fn toggle_timer_kind(&mut self, fps: f64) -> AppResult<()> {
        if self.mode == FieldMode::Correction {
            return Err(AppError::InvalidState(StateIssue::CorrectionModeLocksTimer));
        }
        let previous = self.timer_kind;
        let next = match previous {
            FieldTimerKind::NextBlink => FieldTimerKind::Target,
            FieldTimerKind::Target => FieldTimerKind::NextBlink,
        };
        if next == FieldTimerKind::Target && !self.target_timer_available(fps) {
            return Err(AppError::InvalidState(StateIssue::TargetTimerUnavailable));
        }
        self.timer_kind = next;
        if self.timer_start.is_some() {
            let result = match self.timer_kind {
                FieldTimerKind::NextBlink => self.start_blink_timer(fps),
                FieldTimerKind::Target => self.start_target_timer(fps),
            };
            if result.is_err() {
                self.timer_kind = previous;
            }
            result
        } else {
            // 完了したTargetタイマーから戻る場合は表示だけでなく瞬きカウントダウンを
            // 再開する。表示だけを切り替えると`timer_start`がなく通知音を予約できない。
            self.timer_finished = false;
            if self.timer_kind == FieldTimerKind::NextBlink {
                match self.start_blink_timer(fps) {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        self.timer_kind = previous;
                        Err(error)
                    }
                }
            } else {
                Ok(())
            }
        }
    }

    pub fn target_timer_available(&self, _fps: f64) -> bool {
        if self.results.len() != 1 {
            return false;
        }
        self.results
            .get(self.selected_result)
            .is_some_and(|result| result.target_wait_frame.is_some())
    }

    /// Targetまでの表示Frameへ、フィールド専用オフセットを適用する。
    /// 候補のTarget到達判定・瞬き列・観測アンカーは変更しない。
    pub fn target_wait_frame_with_offset(&self, result: &FieldResult) -> Option<i64> {
        let wait = result.target_wait_frame?;
        if !self.use_offset {
            return Some(wait);
        }
        let trimmed = self.encounter_offset.trim();
        let offset = if trimmed.is_empty() {
            0
        } else {
            trimmed.parse::<i64>().ok()?
        };
        if !(-MAX_SFMT_FRAME..=MAX_SFMT_FRAME).contains(&offset) || offset % 2 != 0 {
            return None;
        }
        Some(wait.saturating_add(offset))
    }

    pub fn target_is_passed(&self, fps: f64) -> bool {
        let Some(result) = self.results.get(self.selected_result) else {
            return false;
        };
        let Some(wait_frame) = self.target_wait_frame_with_offset(result) else {
            return result.target_exact == Some(false)
                && result.target_frame_before.is_some()
                && result.target_frame_after.is_some();
        };
        self.remaining_from_observation(wait_frame, fps).is_err()
    }

    /// Target情報が消えた／無効になった場合は、表示を必ず瞬き側へ戻す。
    pub fn normalize_timer_kind(&mut self, fps: f64) {
        if self.timer_kind == FieldTimerKind::Target && !self.target_timer_available(fps) {
            self.timer_kind = FieldTimerKind::NextBlink;
            if self.timer_start.is_none() && self.results.len() == 1 {
                let _ = self.start_blink_timer(fps);
            }
        }
    }

    pub fn timer_remaining(&self) -> Option<f64> {
        self.timer_remaining_at(Instant::now())
    }

    pub(crate) fn timer_remaining_at(&self, now: Instant) -> Option<f64> {
        self.timer_start.map(|start| {
            (self.timer_seconds - now.saturating_duration_since(start).as_secs_f64()).max(0.0)
        })
    }

    pub fn adjust_by(&mut self, delta: i64, fps: f64) {
        self.adjust = self.adjust.saturating_add(delta);
        let shift = game_frames_to_seconds(delta.unsigned_abs() as f64, fps);
        if let Some(start) = self.timer_start {
            let shifted = if delta.is_positive() {
                start.checked_sub(std::time::Duration::from_secs_f64(shift))
            } else {
                start.checked_add(std::time::Duration::from_secs_f64(shift))
            };
            self.timer_start = shifted.or(Some(start));
        }
    }

    pub fn preview_remaining(&self, kind: FieldTimerKind, fps: f64) -> Option<f64> {
        if kind == self.timer_kind {
            if let Some(remaining) = self.timer_remaining() {
                return Some(remaining);
            }
            if self.timer_finished && kind == FieldTimerKind::Target {
                return Some(0.0);
            }
        }
        let result = self.results.get(self.selected_result)?;
        let wait_frame = match kind {
            FieldTimerKind::NextBlink => {
                let current = self.current_timeline_frame(fps)?;
                result
                    .timeline
                    .iter()
                    .find(|row| row.timeline_frame > current)
                    .map(|row| row.timeline_frame - result.last_wait_frame)?
            }
            FieldTimerKind::Target => self.target_wait_frame_with_offset(result)?,
        };
        self.remaining_from_observation(wait_frame, fps).ok()
    }

    fn remaining_from_observation(&self, wait_frame: i64, fps: f64) -> AppResult<f64> {
        self.remaining_from_observation_at(wait_frame, fps, Instant::now())
    }

    fn remaining_from_observation_at(
        &self,
        wait_frame: i64,
        fps: f64,
        now: Instant,
    ) -> AppResult<f64> {
        let anchor = self
            .observed
            .last()
            .ok_or(AppError::InvalidState(StateIssue::ObservationAnchorMissing))?;
        let total = game_frames_to_seconds(wait_frame as f64, fps)
            - game_frames_to_seconds(self.adjust as f64, fps);
        let remaining = total - now.saturating_duration_since(*anchor).as_secs_f64();
        if remaining <= 0.0 {
            return Err(AppError::InvalidState(StateIssue::TimerTargetPassed));
        }
        Ok(remaining)
    }

    pub fn current_timeline_frame(&self, fps: f64) -> Option<i64> {
        self.current_timeline_frame_at(fps, Instant::now())
    }

    fn current_timeline_frame_at(&self, fps: f64, now: Instant) -> Option<i64> {
        let result = self.results.get(self.selected_result)?;
        if self.mode == FieldMode::Correction {
            if let Some(frame) = self.correction_timeline_frame {
                return Some(frame);
            }
        }
        let anchor = self.observed.last()?;
        Some(
            result.last_wait_frame
                + seconds_to_game_frames(now.saturating_duration_since(*anchor).as_secs_f64(), fps)
                    .floor() as i64,
        )
    }

    pub fn current_timeline_row(&self, fps: f64) -> Option<usize> {
        let result = self.results.get(self.selected_result)?;
        let current = self.current_timeline_frame(fps)?;
        Some(
            result
                .timeline
                .partition_point(|row| row.timeline_frame <= current)
                .saturating_sub(1),
        )
    }
}

/// 表示用の履歴だけを上限内に保つ。Timeline継続カーソルや観測結果の要約には触れない。
fn trim_timeline_history(result: &mut FieldResult) {
    let excess = result
        .timeline
        .len()
        .saturating_sub(crate::application::field::MAX_RETAINED_TIMELINE_ROWS);
    if excess > 0 {
        result.timeline.drain(..excess);
        result.target_blink_count_offset_from_first_row = result
            .target_blink_count_offset_from_first_row
            .saturating_sub(excess as i64);
    }
}

/// 保存済みの全モデル状態が同じかを判定する。
///
/// 観測対象の未来列だけが一致する候補は、他モデルの`remain`やSFMTカーソルが異なる
/// 可能性があるため統合しない。`TimelineCursor`が保存されていない合成テスト結果では、
/// 開始位置まで含めた完全重複だけを統合対象とする。
fn same_full_timeline_state(left: &FieldResult, right: &FieldResult) -> bool {
    let Some(left_cursor) = left
        .target_cursor
        .as_ref()
        .or(left.timeline_cursor.as_ref())
    else {
        return right.target_cursor.is_none()
            && right.timeline_cursor.is_none()
            && left.start_consumption == right.start_consumption;
    };
    let Some(right_cursor) = right
        .target_cursor
        .as_ref()
        .or(right.timeline_cursor.as_ref())
    else {
        return false;
    };
    // `tick`は各候補の観測開始からの相対経過時間であり、乱数状態ではない。
    // 開始位置が異なる候補は同じSFMTカーソルへ到達してもtickだけ異なるため、
    // remain・モデル状態・SFMTカーソル・消費数だけを合流判定に使う。
    left.observed_model == right.observed_model
        && equivalent_model_status(&left_cursor.status, &right_cursor.status)
        && left_cursor.stream_cursor == right_cursor.stream_cursor
        && left_cursor.consumption == right_cursor.consumption
}

/// `remain=0`と`remain=1`は、どちらも次のtickでSFMTを1つ読み、
/// `SFMT % 128 == 0`の判定へ進む。同じSFMTカーソルを持つ場合は、
/// 未来の消費・瞬き遷移が同値になるため、合流判定では同じ待機状態とする。
fn equivalent_model_status(left: &ModelStatus, right: &ModelStatus) -> bool {
    left.remain.len() == right.remain.len()
        && left.remain.iter().zip(&right.remain).all(|(left, right)| {
            left == right || (*left == 0 && *right == 1) || (*left == 1 && *right == 0)
        })
}

pub(super) fn parse_nonnegative(text: &str, field: InputField) -> AppResult<i64> {
    let value = text
        .trim()
        .parse::<i64>()
        .map_err(|_| AppError::InvalidInteger(field))?;
    if value < 0 {
        return Err(AppError::NegativeValue(field));
    }
    Ok(value)
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn searches_all_start_consumptions() {
        let config = FieldConfig {
            seed: 1,
            range_start: 0,
            range_end: 50_000,
            models: 1,
            target_model: 0,
            observed_intervals: vec![1],
            tolerance: 1_000,
            target_consumption: None,
        };
        let results = search(&config).unwrap();
        assert!(results.len() > 1);
        // 1つのSFMT開始位置から複数の観測窓が生じるため、順序は一意ではなく
        // 非減少となる。
        assert!(results
            .windows(2)
            .all(|rows| { rows[0].start_consumption <= rows[1].start_consumption }));
        assert!(results.iter().all(|result| {
            result
                .candidate_blinks
                .iter()
                .any(|blink| blink.frame == result.last_consumption && blink.observed)
        }));
    }

    #[test]
    fn shared_progression_cache_answers_target_like_reference_replay() {
        let config = FieldConfig {
            seed: 1,
            range_start: 0,
            range_end: 10,
            models: 1,
            target_model: 0,
            observed_intervals: vec![1],
            tolerance: 1_000,
            target_consumption: None,
        };
        let result = search(&config).unwrap().remove(0);
        let observation_cache = crate::domain::rng::generate_observation_cache(
            config.seed,
            0..=10,
            100_000,
            crate::domain::timing::DEFAULT_FPS,
        )
        .unwrap();
        let mut state = FieldState {
            seed: format!("{:08X}", config.seed),
            results: vec![result],
            observed: vec![Instant::now()],
            ..FieldState::default()
        };
        state.set_observation_cache(observation_cache);

        let observed_result = state.results[0].clone();
        for target in [
            observed_result.last_consumption + 500,
            observed_result.last_consumption + 5_000,
            observed_result.last_consumption + 1_000,
        ] {
            let reference = crate::application::field::retarget_confirmed_result(
                observed_result.clone(),
                config.seed,
                target,
                state.observation_cache.as_ref().unwrap(),
                None,
            )
            .unwrap();
            let cached = state
                .retarget_confirmed_result_from_cache(target, None)
                .unwrap();

            assert_eq!(cached.target_wait_frame, reference.target_wait_frame);
            assert_eq!(cached.target_exact, reference.target_exact);
            assert_eq!(cached.target_frame_before, reference.target_frame_before);
            assert_eq!(cached.target_frame_after, reference.target_frame_after);
            assert_eq!(cached.target_blink_count, reference.target_blink_count);
        }
        let generated_ticks = state
            .resolved_timeline_cache
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .tick_deltas
            .len();
        state
            .retarget_confirmed_result_from_cache(observed_result.last_consumption + 1_000, None)
            .unwrap();
        assert_eq!(
            state
                .resolved_timeline_cache
                .as_ref()
                .unwrap()
                .lock()
                .unwrap()
                .tick_deltas
                .len(),
            generated_ticks,
            "backward Target edits should reuse the generated prefix"
        );

        let anchor = observed_result.target_cursor.as_ref().unwrap();
        let mut reference_sfmt = state
            .observation_cache
            .as_ref()
            .unwrap()
            .sfmt_at_checked(anchor.stream_cursor)
            .unwrap()
            .unwrap();
        let mut reference_status = anchor.status.clone();
        let mut reference_cursor = anchor.stream_cursor;
        let mut reference_consumption = anchor.consumption;
        let mut expected_deltas = Vec::with_capacity(generated_ticks);
        let mut expected_blinks = Vec::new();
        for offset in 0..generated_ticks {
            let (used, consumed) = reference_status.next_state_with_positions(|| {
                let position = reference_cursor;
                let value = reference_sfmt.next_u64();
                reference_cursor += 1;
                (position, value)
            });
            reference_consumption += used as i64;
            expected_deltas.push(used as u8);
            if consumed
                .iter()
                .any(|event| event.model == observed_result.observed_model && event.blink)
            {
                expected_blinks.push(anchor.tick + offset as i64 + 1);
            }
        }
        let shared = state
            .resolved_timeline_cache
            .as_ref()
            .unwrap()
            .lock()
            .unwrap();
        assert_eq!(shared.tick_deltas, expected_deltas);
        assert_eq!(
            shared
                .blinks
                .iter()
                .map(|event| i64::from(event.tick))
                .collect::<Vec<_>>(),
            expected_blinks
        );
        assert_eq!(shared.cursor.consumption, reference_consumption);
        for offset in [0, generated_ticks / 2, generated_ticks] {
            let expected = anchor.consumption
                + expected_deltas[..offset]
                    .iter()
                    .map(|delta| i64::from(*delta))
                    .sum::<i64>();
            assert_eq!(
                shared.consumption_at_tick(anchor.tick + offset as i64),
                expected
            );
        }
    }

    #[test]
    fn observed_intervals_can_start_at_a_later_blink() {
        let values = SearchValues::new(0, generate_u64_stream(1, 10_000));
        let mut status = ModelStatus::new(1);
        let mut stream_cursor = 0;
        let mut events = Vec::new();
        for tick in 0..MAX_WAIT_TICKS {
            let Some((_used, blink)) =
                next_state_with_values(&mut status, &mut stream_cursor, &values)
            else {
                break;
            };
            if blink.contains(&0) {
                events.push((tick + 1) * 2);
            }
        }
        let window_len = 4;
        let selected = (1..events.len().saturating_sub(window_len - 1))
            .find(|&start| {
                let expected = events[start + 1..start + window_len]
                    .windows(2)
                    .map(|pair| pair[1] - pair[0])
                    .collect::<Vec<_>>();
                !events[..start].windows(window_len).any(|window| {
                    window
                        .windows(2)
                        .map(|pair| pair[1] - pair[0])
                        .eq(expected.iter().copied())
                })
            })
            .expect("deterministic SFMT stream should provide a later unique window");
        let config = FieldConfig {
            seed: 1,
            range_start: 0,
            range_end: 10_000,
            models: 1,
            target_model: 0,
            observed_intervals: events[selected..selected + window_len]
                .windows(2)
                .map(|pair| pair[1] - pair[0])
                .collect(),
            tolerance: 0,
            target_consumption: None,
        };
        let result = search_candidate(0, &config, &values).expect("later blink window matches");
        assert!(selected > 0);
        assert_eq!(result.first_wait_frame, events[selected]);
    }

    #[test]
    fn targeted_search_refines_interval_matches_after_chunk_scan() {
        let config = FieldConfig {
            seed: 1,
            range_start: 0,
            range_end: 100,
            models: 2,
            target_model: 0,
            observed_intervals: vec![1],
            tolerance: 1_000,
            target_consumption: Some(5_000),
        };
        let results = search(&config).unwrap();
        assert!(!results.is_empty());
        assert!(
            results
                .iter()
                .all(|result| result.target_wait_frame.is_some()
                    || result.target_exact == Some(false))
        );
    }

    #[test]
    fn search_values_are_anchored_at_range_start() {
        let full = crate::domain::rng::generate_u64_values(1, 32);
        let anchored = generate_search_values(&FieldConfig {
            seed: 1,
            range_start: 10,
            range_end: 12,
            models: 1,
            target_model: 0,
            observed_intervals: vec![1],
            tolerance: 0,
            target_consumption: None,
        });

        assert_eq!(anchored.base, 10);
        assert_eq!(&anchored.values[..5], &full[10..15]);
    }

    #[test]
    fn search_pool_can_be_prepared_before_blink_intervals_exist() {
        let config = FieldConfig {
            seed: 1,
            range_start: 10,
            range_end: 12,
            models: 2,
            target_model: 0,
            observed_intervals: Vec::new(),
            tolerance: 5,
            target_consumption: Some(20),
        };
        let pool = prepare_search_pool(&config).unwrap();
        assert_eq!(pool.cache.read_values(10, 1).unwrap().len(), 1);
        assert!(pool.covers(&config));
    }

    #[test]
    fn cancelled_pool_generation_returns_without_building_results() {
        let config = FieldConfig {
            seed: 1,
            range_start: 0,
            range_end: 100,
            models: 2,
            target_model: 0,
            observed_intervals: Vec::new(),
            tolerance: 5,
            target_consumption: None,
        };
        let token = AtomicBool::new(true);
        assert!(matches!(
            prepare_search_pool_with_configs_cancelable(&[config], Some(&token)),
            Err(AppError::Cancelled)
        ));
    }

    #[test]
    fn rejects_invalid_target_model() {
        let config = FieldConfig {
            seed: 1,
            range_start: 0,
            range_end: 1_000,
            models: 1,
            target_model: 1,
            observed_intervals: vec![1],
            tolerance: 0,
            target_consumption: None,
        };
        assert!(search(&config).is_err());
    }

    #[test]
    fn ui_idx_controls_observed_model() {
        let mut state = FieldState {
            npc_count: "2".into(),
            target_model: "0".into(),
            ..FieldState::default()
        };
        assert_eq!(state.config().unwrap().models, 3);
        assert_eq!(state.config().unwrap().target_model, 0);
        state.target_model = "1".into();
        assert_eq!(state.config().unwrap().target_model, 1);
    }

    #[test]
    fn normalize_keeps_distinct_target_sfmt_branches_separate() {
        let make_result = |target_consumptions| FieldResult {
            start_consumption: 500,
            first_wait_frame: 100,
            last_wait_frame: 200,
            first_consumption: 600,
            last_consumption: 700,
            intervals: vec![100],
            target_consumptions,
            next_blink_wait_frame: 100,
            target_wait_frame: None,
            target_exact: None,
            target_frame_before: None,
            target_frame_after: None,
            observed_model: 5,
            models: 13,
            timeline: Vec::new(),
            candidate_blinks: Vec::new(),
            target_blink_count: None,
            target_blink_count_offset_from_first_row: 0,
            timeline_cursor: None,
            target_cursor: None,
        };
        let mut state = FieldState {
            target_model: "5".into(),
            ..FieldState::default()
        };
        let normalized = state.normalize_search_results(vec![
            make_result(vec![610, 710]),
            make_result(vec![611, 710]),
        ]);
        assert_eq!(normalized.len(), 2);
    }

    #[test]
    fn normalize_keeps_candidates_with_different_internal_states() {
        let make_result =
            |start_consumption: i64, tick: i64, target_consumptions: Vec<i64>| FieldResult {
                start_consumption,
                first_wait_frame: 100,
                last_wait_frame: 200,
                first_consumption: target_consumptions[0],
                last_consumption: *target_consumptions.last().unwrap(),
                intervals: vec![100, 100],
                target_consumptions,
                next_blink_wait_frame: 100,
                target_wait_frame: None,
                target_exact: None,
                target_frame_before: None,
                target_frame_after: None,
                observed_model: 10,
                models: 13,
                timeline: Vec::new(),
                candidate_blinks: Vec::new(),
                target_blink_count: None,
                target_blink_count_offset_from_first_row: 0,
                timeline_cursor: None,
                target_cursor: Some(TimelineCursor {
                    status: ModelStatus {
                        remain: vec![1, 1, 1, 1, 1, 1, 1, 3, 1, 31, 30, 1, 1],
                    },
                    stream_cursor: 4_290,
                    consumption: 4_290,
                    tick,
                }),
            };
        let mut state = FieldState {
            target_model: "10".into(),
            ..FieldState::default()
        };
        let left = make_result(482, 369, vec![1_005, 4_287, 4_939]);
        let mut right = make_result(508, 364, vec![1_007, 4_287, 4_939]);
        right.target_cursor.as_mut().unwrap().status.remain[1] = 2;
        let normalized = state.normalize_search_results(vec![left, right]);
        assert_eq!(normalized.len(), 2);
    }

    #[test]
    fn normalize_merges_candidates_with_identical_full_state() {
        let make_result = |start_consumption: i64, intervals, target_consumptions| FieldResult {
            start_consumption,
            first_wait_frame: 100,
            last_wait_frame: 200,
            first_consumption: 600,
            last_consumption: 700,
            intervals,
            target_consumptions,
            next_blink_wait_frame: 100,
            target_wait_frame: None,
            target_exact: None,
            target_frame_before: None,
            target_frame_after: None,
            observed_model: 8,
            models: 23,
            timeline: Vec::new(),
            candidate_blinks: Vec::new(),
            target_blink_count: None,
            target_blink_count_offset_from_first_row: 0,
            timeline_cursor: None,
            target_cursor: Some(TimelineCursor {
                status: ModelStatus {
                    remain: vec![1; 23],
                },
                stream_cursor: 800,
                consumption: 800,
                tick: 100,
            }),
        };
        let mut state = FieldState {
            target_model: "8".into(),
            ..FieldState::default()
        };
        let left = make_result(480, vec![100, 100], vec![600, 700, 800]);
        let mut right = make_result(491, vec![98, 102], vec![604, 696, 800]);
        right.target_cursor.as_mut().unwrap().status.remain[5] = 0;
        right.target_cursor.as_mut().unwrap().tick = 101;
        let normalized = state.normalize_search_results(vec![left, right]);
        assert_eq!(normalized.len(), 1);
    }

    #[test]
    fn normalize_confirms_same_sfmt_series_and_uses_npc_number() {
        let make_result = |start_consumption: i64, observed_model: usize| FieldResult {
            start_consumption,
            first_wait_frame: 100,
            last_wait_frame: 200,
            first_consumption: 600,
            last_consumption: 700,
            intervals: vec![100],
            target_consumptions: vec![610, 710],
            next_blink_wait_frame: 100,
            target_wait_frame: None,
            target_exact: None,
            target_frame_before: None,
            target_frame_after: None,
            observed_model,
            models: 13,
            timeline: Vec::new(),
            candidate_blinks: Vec::new(),
            target_blink_count: None,
            target_blink_count_offset_from_first_row: 0,
            timeline_cursor: None,
            target_cursor: None,
        };
        let mut state = FieldState {
            target_model: "0".into(),
            ..FieldState::default()
        };
        let normalized =
            state.normalize_search_results(vec![make_result(480, 1), make_result(485, 0)]);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].start_consumption, 485);
        assert_eq!(normalized[0].observed_model, 0);
    }

    #[test]
    fn normalize_selects_the_matching_start_from_three_special_branches() {
        let make_result = |start_consumption: i64, observed_model: usize| FieldResult {
            start_consumption,
            first_wait_frame: 100,
            last_wait_frame: 300,
            first_consumption: 1_000,
            last_consumption: 1_200,
            intervals: vec![100, 100],
            target_consumptions: vec![1_000, 1_100, 1_200],
            next_blink_wait_frame: 100,
            target_wait_frame: None,
            target_exact: None,
            target_frame_before: None,
            target_frame_after: None,
            observed_model,
            models: 13,
            timeline: Vec::new(),
            candidate_blinks: Vec::new(),
            target_blink_count: None,
            target_blink_count_offset_from_first_row: 0,
            timeline_cursor: None,
            target_cursor: None,
        };
        let mut state = FieldState {
            target_model: "6".into(),
            ..FieldState::default()
        };
        let normalized = state.normalize_search_results(vec![
            make_result(480, 2),
            make_result(482, 6),
            make_result(490, 8),
        ]);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].start_consumption, 482);
        assert_eq!(normalized[0].observed_model, 6);
    }

    #[test]
    fn normalize_marks_npc_mismatch_and_keeps_representative_start() {
        let result = FieldResult {
            start_consumption: 500,
            first_wait_frame: 100,
            last_wait_frame: 200,
            first_consumption: 600,
            last_consumption: 700,
            intervals: vec![100],
            target_consumptions: vec![610, 710],
            next_blink_wait_frame: 100,
            target_wait_frame: None,
            target_exact: None,
            target_frame_before: None,
            target_frame_after: None,
            observed_model: 5,
            models: 13,
            timeline: Vec::new(),
            candidate_blinks: vec![CandidateBlink {
                frame: 710,
                timeline_frame: 200,
                observed: true,
                target_consumption: Some(710),
            }],
            target_blink_count: None,
            target_blink_count_offset_from_first_row: 0,
            timeline_cursor: None,
            target_cursor: None,
        };
        let mut state = FieldState {
            target_model: "9".into(),
            ..FieldState::default()
        };
        let normalized = state.normalize_search_results(vec![result]);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].start_consumption, 500);
        assert!(normalized[0].candidate_blinks.is_empty());
        assert_eq!(normalized[0].target_consumptions, vec![610, 710]);
    }

    #[test]
    fn timeline_table_follows_current_position_by_default() {
        assert!(FieldState::default().table_follow_current);
    }

    #[test]
    fn timeline_history_is_bounded_without_changing_rng_or_observation_state() {
        let config = FieldConfig {
            seed: 1,
            range_start: 0,
            range_end: 10,
            models: 1,
            target_model: 0,
            observed_intervals: vec![1],
            tolerance: 1_000,
            target_consumption: None,
        };
        let mut result = search(&config).unwrap().remove(0);
        result.timeline = (0..(MAX_RETAINED_TIMELINE_ROWS + 44))
            .map(|index| FieldTimelineRow {
                frame: index as i64,
                timeline_frame: index as i64 * 2,
                duration_frames: 2,
            })
            .collect();
        result.target_blink_count = Some(500);
        result.target_blink_count_offset_from_first_row = 50;
        let before_trim = result.target_blinks_after_row(44);
        let cursor_before = result.timeline_cursor.clone();
        let target_cursor_before = result.target_cursor.clone();
        let intervals_before = result.intervals.clone();
        let observations_before = result.target_consumptions.clone();

        trim_timeline_history(&mut result);

        assert_eq!(result.timeline.len(), MAX_RETAINED_TIMELINE_ROWS);
        assert_eq!(result.timeline[0].frame, 44);
        assert_eq!(result.timeline.last().unwrap().frame, 299);
        assert_eq!(result.target_blink_count_offset_from_first_row, 6);
        assert_eq!(result.target_blinks_after_row(0), before_trim);
        assert_eq!(before_trim, Some(506));
        assert_eq!(result.timeline_cursor, cursor_before);
        assert_eq!(result.target_cursor, target_cursor_before);
        assert_eq!(result.intervals, intervals_before);
        assert_eq!(result.target_consumptions, observations_before);
    }

    #[test]
    fn correction_mode_does_not_extend_or_schedule_the_timeline() {
        let config = FieldConfig {
            seed: 1,
            range_start: 0,
            range_end: 10,
            models: 1,
            target_model: 0,
            observed_intervals: vec![1],
            tolerance: 1_000,
            target_consumption: None,
        };
        let result = search(&config).unwrap().remove(0);
        let mut state = FieldState {
            results: vec![result],
            observed: vec![Instant::now()],
            ..FieldState::default()
        };
        state.enter_correction_mode(crate::domain::timing::DEFAULT_FPS);

        assert!(!state.timeline_extension_needed(crate::domain::timing::DEFAULT_FPS));
        assert!(!state
            .extend_timeline_if_needed(crate::domain::timing::DEFAULT_FPS)
            .unwrap());
    }

    #[test]
    fn session_phase_locks_timer_after_entering_correction() {
        let mut state = FieldState::default();
        assert_eq!(state.session_phase(), SessionPhase::Input);
        assert!(state.can_edit_observation_settings());
        state.mode = FieldMode::Idle;
        state.results.push(FieldResult {
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
            models: 1,
            timeline: Vec::new(),
            candidate_blinks: Vec::new(),
            target_blink_count: None,
            target_blink_count_offset_from_first_row: 0,
            timeline_cursor: None,
            target_cursor: None,
        });
        assert_eq!(state.session_phase(), SessionPhase::TimelineReady);
        assert!(!state.can_edit_observation_settings());
        state.enter_correction_mode(crate::domain::timing::DEFAULT_FPS);
        assert_eq!(state.session_phase(), SessionPhase::Correction);
        assert!(!state.can_edit_general_settings());
        assert!(state.can_edit_correction_inputs());
        assert!(state
            .start_blink_timer(crate::domain::timing::DEFAULT_FPS)
            .is_err());
    }

    #[test]
    fn npc_count_adds_the_implicit_player_model() {
        let state = FieldState {
            npc_count: "0".into(),
            ..FieldState::default()
        };
        assert_eq!(state.config().unwrap().models, 1);
    }

    #[test]
    fn npc_number_is_required_even_for_the_player() {
        let state = FieldState {
            target_model: String::new(),
            ..FieldState::default()
        };
        assert!(state.configs().is_err());
    }

    #[test]
    fn multiple_npc_search_expands_the_requested_range() {
        let state = FieldState {
            npc_count: "10~20".into(),
            multiple_npc_search: true,
            ..FieldState::default()
        };
        let configs = state.configs().unwrap();
        assert_eq!(configs.len(), 11);
        assert_eq!(configs.first().unwrap().models, 11);
        assert_eq!(configs.last().unwrap().models, 21);
    }

    #[test]
    fn multiple_npc_search_rejects_ranges_wider_than_twenty_npc() {
        let state = FieldState {
            npc_count: "10~31".into(),
            multiple_npc_search: true,
            ..FieldState::default()
        };
        assert!(state.configs().is_err());
    }

    #[test]
    fn multiple_npc_search_allows_a_width_of_twenty() {
        let state = FieldState {
            npc_count: "10~30".into(),
            multiple_npc_search: true,
            ..FieldState::default()
        };
        assert_eq!(state.configs().unwrap().len(), 21);
    }

    #[test]
    fn npc_count_is_limited_to_zero_through_fifty() {
        let mut state = FieldState {
            npc_count: MAX_NPC_COUNT.to_string(),
            ..FieldState::default()
        };
        assert!(state.configs().is_ok());
        state.npc_count = (MAX_NPC_COUNT + 1).to_string();
        assert!(state.configs().is_err());
    }

    #[test]
    fn search_rejects_ranges_wider_than_one_hundred_thousand_frames() {
        let state = FieldState {
            range_start: "9000000".into(),
            range_end: "9200000".into(),
            ..FieldState::default()
        };
        assert!(state.config().is_err());
    }

    #[test]
    fn range_end_can_be_set_from_start_plus_width() {
        let mut state = FieldState {
            range_start: "9000000".into(),
            range_end: "0".into(),
            range_after_start: "10000".into(),
            ..FieldState::default()
        };
        state.set_range_from_start();
        assert_eq!(state.range_end, "9010000");
    }

    #[test]
    fn unique_timeline_extends_from_the_previous_remain_state() {
        let config = FieldConfig {
            seed: 1,
            range_start: 0,
            range_end: 10,
            models: 1,
            target_model: 0,
            observed_intervals: vec![1],
            tolerance: 1_000,
            target_consumption: Some(100_000),
        };
        let mut results = search(&config).unwrap();
        let mut state = FieldState {
            range_start: "0".into(),
            range_end: "10".into(),
            npc_count: "0".into(),
            target_model: "0".into(),
            target_consumption: "100000".into(),
            intervals: vec![1],
            observed: vec![Instant::now() - std::time::Duration::from_secs(170)],
            results: vec![results.remove(0)],
            ..FieldState::default()
        };
        state.set_observation_cache(
            crate::domain::rng::generate_observation_cache(
                1,
                0..=10,
                100_000,
                crate::domain::timing::DEFAULT_FPS,
            )
            .unwrap(),
        );
        let before = state.results[0].timeline.len();
        assert!(state
            .extend_timeline_if_needed(crate::domain::timing::DEFAULT_FPS)
            .unwrap());
        assert!(state.results[0].timeline.len() > before);
        assert!(state.results[0].timeline.last().unwrap().timeline_frame > 10_000);
    }

    #[test]
    fn empty_timeline_chunks_continue_until_the_target_model_can_blink() {
        let config = FieldConfig {
            seed: 1,
            range_start: 0,
            range_end: 10,
            models: 1,
            target_model: 0,
            observed_intervals: vec![1],
            tolerance: 1_000,
            target_consumption: None,
        };
        let mut result = search(&config).unwrap().remove(0);
        // Keep the observed model dormant through two complete chunks. Other state
        // remains valid, but neither chunk can add a visible blink row.
        let anchor = result.target_cursor.as_mut().unwrap();
        anchor.status.remain[0] = TIMELINE_CHUNK_TICKS as i32 * 3 + 10;
        let anchor = anchor.clone();
        let original_tick = anchor.tick;
        let original_timeline = vec![FieldTimelineRow {
            frame: anchor.consumption,
            timeline_frame: anchor.tick * 2,
            duration_frames: 0,
        }];
        result.timeline = original_timeline.clone();
        result.timeline_cursor = Some(anchor);
        let mut state = FieldState {
            seed: "00000001".into(),
            range_start: "0".into(),
            range_end: "10".into(),
            npc_count: "0".into(),
            target_model: "0".into(),
            intervals: vec![1],
            observed: vec![Instant::now() - std::time::Duration::from_secs(170)],
            results: vec![result],
            ..FieldState::default()
        };
        state.set_observation_cache(
            crate::domain::rng::generate_observation_cache(
                1,
                0..=10,
                100_000,
                crate::domain::timing::DEFAULT_FPS,
            )
            .unwrap(),
        );

        for chunk in 1..=2 {
            assert!(state.timeline_extension_needed(crate::domain::timing::DEFAULT_FPS));
            assert!(state
                .extend_timeline_if_needed(crate::domain::timing::DEFAULT_FPS)
                .unwrap());
            assert_eq!(state.results[0].timeline, original_timeline);
            assert_eq!(
                state.results[0].timeline_cursor.as_ref().unwrap().tick,
                original_tick + TIMELINE_CHUNK_TICKS * chunk
            );
        }
    }

    #[test]
    fn per_tick_work_and_remain_stay_bounded_across_seed_and_model_extremes() {
        let seeds = [0, 1, u32::MAX, 0x367C_4DFE];
        let model_counts = [1, 22, MAX_NPC_COUNT + 1];
        for seed in seeds {
            for models in model_counts {
                let mut sfmt = Sfmt::new(seed);
                let mut status = ModelStatus::new(models);
                let mut total_consumed = 0_usize;
                for _ in 0..20_000 {
                    let (used, consumed) = status.next_state_with_positions(|| {
                        let position = total_consumed as i64;
                        let value = sfmt.next_u64();
                        total_consumed += 1;
                        (position, value)
                    });
                    assert!(used <= models);
                    assert_eq!(used, consumed.len());
                    assert!(consumed.iter().all(|event| event.model < models));
                    assert!(status
                        .remain
                        .iter()
                        .all(|remain| (-5..=36).contains(remain)));
                }
                assert!(total_consumed <= models * 20_000);
            }
        }
    }

    #[test]
    #[ignore = "50,000,000FまでGUIのTimeline延長要求を連続再現する長時間テスト"]
    fn field_timeline_worker_requests_stay_bounded_through_max_frame() {
        let seed = 0x367C4DFE;
        let models = MAX_NPC_COUNT + 1;
        let target_model = models - 1;
        let start = 486_i64;
        let target = crate::domain::rng::MAX_SFMT_FRAME;
        let fps = crate::domain::timing::DEFAULT_FPS;
        let mut sfmt = Sfmt::new(seed);
        let mut stream_cursor = 0_i64;
        while stream_cursor < start {
            sfmt.next_u64();
            stream_cursor += 1;
        }
        let mut status = ModelStatus::new(models);
        let mut consumption = start;
        let mut tick = 0_i64;
        let mut observed_events = Vec::new();
        while observed_events.len() < 4 {
            let (used, consumed) = status.next_state_with_positions(|| {
                let position = stream_cursor;
                let value = sfmt.next_u64();
                stream_cursor += 1;
                (position, value)
            });
            consumption += used as i64;
            tick += 1;
            if consumed
                .iter()
                .any(|event| event.model == target_model && event.blink)
            {
                observed_events.push(BlinkEvent {
                    wait_frame: tick * 2,
                    consumption,
                    status: status.clone(),
                    stream_cursor,
                    model_consumption: consumed
                        .iter()
                        .find(|event| event.model == target_model && event.blink)
                        .map(|event| event.position),
                });
            }
        }

        let values = SearchValues::new(0, generate_u64_values_from(seed, 0, 200_000));
        let (timeline, candidate_blinks, timeline_cursor) =
            build_timeline(&observed_events, target_model, None, &values);
        let first = &observed_events[0];
        let last = observed_events.last().unwrap();
        let intervals = observed_events
            .windows(2)
            .map(|pair| pair[1].wait_frame - pair[0].wait_frame)
            .collect::<Vec<_>>();
        let result = FieldResult {
            start_consumption: start,
            first_wait_frame: first.wait_frame,
            last_wait_frame: last.wait_frame,
            first_consumption: first.consumption,
            last_consumption: last.consumption,
            intervals,
            target_consumptions: observed_events
                .iter()
                .filter_map(|event| event.model_consumption)
                .collect(),
            next_blink_wait_frame: timeline
                .iter()
                .find(|row| row.timeline_frame > last.wait_frame)
                .map(|row| row.timeline_frame - last.wait_frame)
                .unwrap_or_default(),
            target_wait_frame: None,
            target_exact: None,
            target_frame_before: None,
            target_frame_after: None,
            observed_model: target_model,
            models,
            timeline,
            candidate_blinks,
            target_blink_count: None,
            target_blink_count_offset_from_first_row: 0,
            timeline_cursor: Some(timeline_cursor),
            target_cursor: Some(TimelineCursor {
                status: last.status.clone(),
                stream_cursor: last.stream_cursor,
                consumption: last.consumption,
                tick: last.wait_frame / 2,
            }),
        };
        let cache =
            crate::domain::rng::generate_observation_cache(seed, 0..=200_000, 0, fps).unwrap();
        assert!(cache.available_end() < target);
        let direct_reference =
            apply_target_from_cursor(result.clone(), seed, target, Some(&cache), None).unwrap();
        let mut state = FieldState {
            seed: format!("{seed:08X}"),
            range_start: start.to_string(),
            range_end: (start + 10_000).to_string(),
            npc_count: (models - 1).to_string(),
            target_model: target_model.to_string(),
            target_consumption: target.to_string(),
            intervals: result.intervals.clone(),
            results: vec![result],
            observed: vec![Instant::now()],
            ..FieldState::default()
        };
        state.set_observation_cache(cache);
        state.prepare_shared_timeline_cache().unwrap();
        let target_started = Instant::now();
        let target_snapshot = state.clone();
        let target_worker = std::thread::spawn(move || {
            let mut snapshot = target_snapshot;
            snapshot.retarget_confirmed_result_from_cache(target, None)
        });
        let mut ui_snapshot = state.clone();
        let mut ui_updates_during_workers = 0_usize;
        let mut last_ui_update = Instant::now();
        let mut next_ui_update = last_ui_update + std::time::Duration::from_secs_f64(1.0 / 30.0);
        let mut max_ui_update_gap = std::time::Duration::ZERO;
        let mut max_ui_read_elapsed = std::time::Duration::ZERO;
        while !target_worker.is_finished() {
            let now = Instant::now();
            if now >= next_ui_update {
                let ui_read_started = Instant::now();
                let _ = ui_snapshot.current_sfmt_frame(fps);
                let ui_read_elapsed = ui_read_started.elapsed();
                max_ui_read_elapsed = max_ui_read_elapsed.max(ui_read_elapsed);
                max_ui_update_gap = max_ui_update_gap
                    .max(now - last_ui_update)
                    .max(ui_read_elapsed);
                last_ui_update = Instant::now();
                next_ui_update = last_ui_update + std::time::Duration::from_secs_f64(1.0 / 30.0);
                ui_updates_during_workers += 1;
            } else {
                std::thread::sleep((next_ui_update - now).min(std::time::Duration::from_millis(1)));
            }
        }
        let result = target_worker.join().unwrap().unwrap();
        let target_search_elapsed = target_started.elapsed();
        assert!(result.target_wait_frame.is_some());
        assert_eq!(result.target_wait_frame, direct_reference.target_wait_frame);
        assert_eq!(result.target_exact, direct_reference.target_exact);
        assert_eq!(
            result.target_frame_before,
            direct_reference.target_frame_before
        );
        assert_eq!(
            result.target_frame_after,
            direct_reference.target_frame_after
        );
        assert_eq!(
            result.target_blink_count,
            direct_reference.target_blink_count
        );
        let expected_target_blinks = result.target_blink_count.unwrap();
        let target_timeline_frame = result.last_wait_frame + result.target_wait_frame.unwrap();
        state.results[0] = result;

        let started = Instant::now();
        let mut request_durations = Vec::new();
        let mut request_count = 0_usize;
        loop {
            let projection_end = state.timeline_projection_through_tick;
            let cursor = state
                .resolved_timeline_cache
                .as_ref()
                .unwrap()
                .lock()
                .unwrap()
                .cursor
                .clone();
            if projection_end >= cursor.tick {
                break;
            }
            let current_frame = projection_end
                .saturating_mul(2)
                .saturating_sub(state.results[0].last_wait_frame)
                .saturating_add(2)
                .max(0);
            let elapsed = crate::domain::timing::game_frames_to_seconds(current_frame as f64, fps);
            state.observed = vec![Instant::now() - std::time::Duration::from_secs_f64(elapsed)];
            assert!(state.timeline_extension_needed(fps));

            // App::start_field_timeline_extensionと同様に、状態を複製して
            // ワーカーで1チャンクだけ延長し、完了後にTimeline部分だけを反映する。
            let snapshot = state.clone();
            let request_started = Instant::now();
            let worker = std::thread::spawn(move || {
                let mut extended = snapshot;
                let appended = extended.extend_timeline_if_needed(fps);
                (extended, appended)
            });
            while !worker.is_finished() {
                std::thread::yield_now();
            }
            let (extended, can_continue) = worker.join().unwrap();
            request_durations.push(request_started.elapsed());
            let can_continue = can_continue.unwrap();
            state.apply_timeline_extension(extended);
            request_count += 1;

            let result = &state.results[0];
            assert!(result.timeline.len() <= MAX_RETAINED_TIMELINE_ROWS);
            assert_eq!(result.target_blink_count, Some(expected_target_blinks));
            assert!(result
                .timeline
                .windows(2)
                .all(|pair| { pair[0].timeline_frame < pair[1].timeline_frame }));
            if !can_continue {
                let cursor = state.results[0].timeline_cursor.as_ref().unwrap();
                assert!(
                    cursor.consumption >= MAX_SFMT_FRAME
                        || cursor.tick >= MAX_FUTURE_TIMELINE_TICKS
                );
                break;
            }
        }

        let final_result = &state.results[0];
        let final_cursor = final_result.timeline_cursor.as_ref().unwrap();
        let mut sorted_durations = request_durations.clone();
        sorted_durations.sort_unstable();
        let first_ten_average = request_durations
            .iter()
            .take(10)
            .map(std::time::Duration::as_secs_f64)
            .sum::<f64>()
            / request_durations.len().min(10) as f64;
        let last_ten_average = request_durations
            .iter()
            .rev()
            .take(10)
            .map(std::time::Duration::as_secs_f64)
            .sum::<f64>()
            / request_durations.len().min(10) as f64;
        println!(
            "50M Frame field UI-worker simulation: target_calc={:.2}s, chunks={request_count}, extension_elapsed={:.2}s, first10_avg={:.2}ms, last10_avg={:.2}ms, p95={:.2}ms, ui_updates_during_target={ui_updates_during_workers}, max_ui_gap={:.2}ms, max_ui_read={:.2}ms, cache_end={}, final_consumption={}, display_rows={}",
            target_search_elapsed.as_secs_f64(),
            started.elapsed().as_secs_f64(),
            first_ten_average * 1_000.0,
            last_ten_average * 1_000.0,
            sorted_durations[(sorted_durations.len() * 95 / 100).min(sorted_durations.len() - 1)]
                .as_secs_f64()
                * 1_000.0,
            max_ui_update_gap.as_secs_f64() * 1_000.0,
            max_ui_read_elapsed.as_secs_f64() * 1_000.0,
            state.observation_cache.as_ref().unwrap().available_end(),
            final_cursor.consumption,
            final_result.timeline.len(),
        );
        assert!(final_cursor.consumption >= target);
        assert!(ui_updates_during_workers > 0);
        assert!(max_ui_update_gap <= std::time::Duration::from_millis(50));
        assert!(target_timeline_frame > final_result.last_wait_frame);
        assert_eq!(
            final_result.target_blink_count,
            Some(expected_target_blinks)
        );
        assert!(final_result.timeline.len() <= MAX_RETAINED_TIMELINE_ROWS);
    }

    #[test]
    fn prepared_timeline_index_keeps_the_same_interval_candidates() {
        let config = FieldConfig {
            seed: 1,
            range_start: 0,
            range_end: 100,
            models: 2,
            target_model: 0,
            observed_intervals: vec![1],
            tolerance: 1_000,
            target_consumption: None,
        };
        let baseline = search(&config).unwrap();
        let pool = prepare_search_pool_with_configs(std::slice::from_ref(&config)).unwrap();
        let prepared = search_with_preferred_idx_with_pool(&config, &pool)
            .unwrap()
            .results;
        assert_eq!(
            baseline
                .iter()
                .map(|result| result.start_consumption)
                .collect::<Vec<_>>(),
            prepared
                .iter()
                .map(|result| result.start_consumption)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn unknown_target_search_keeps_idx_candidates_but_skips_target_check() {
        let config = FieldConfig {
            seed: 1,
            range_start: 0,
            range_end: 100,
            models: 2,
            target_model: 0,
            observed_intervals: vec![1],
            tolerance: 1_000,
            target_consumption: Some(50),
        };
        let results = search_without_target_model(&config).unwrap();
        assert!(!results.is_empty());
        assert!(results.iter().any(|result| result.observed_model == 0));
        assert!(results.iter().any(|result| result.observed_model == 1));
        assert!(results
            .iter()
            .all(|result| result.target_wait_frame.is_none()));
    }

    #[test]
    fn observation_input_is_locked_after_unique_candidate() {
        let mut state = FieldState::default();
        state.mode = FieldMode::Observing;
        assert!(state.is_observation_input_enabled());
        state.results.push(FieldResult {
            start_consumption: 477,
            first_wait_frame: 2,
            last_wait_frame: 4,
            first_consumption: 478,
            last_consumption: 479,
            intervals: vec![2],
            target_consumptions: Vec::new(),
            next_blink_wait_frame: 2,
            target_wait_frame: None,
            target_exact: None,
            target_frame_before: None,
            target_frame_after: None,
            observed_model: 0,
            models: 1,
            timeline: Vec::new(),
            candidate_blinks: Vec::new(),
            target_blink_count: None,
            target_blink_count_offset_from_first_row: 0,
            timeline_cursor: None,
            target_cursor: None,
        });
        assert!(!state.is_observation_input_enabled());
        state.refresh_nearest_timeline_frame();
        assert_eq!(state.nearest_timeline_frame, Some(477));
    }

    #[test]
    fn target_remains_editable_until_explicit_correction_entry() {
        let mut state = FieldState::default();
        state.timer_start = Some(Instant::now());
        assert!(state.can_edit_target());

        state.enter_correction_mode(crate::domain::timing::DEFAULT_FPS);
        assert!(!state.can_edit_target());

        state.cancel_observation();
        assert!(state.can_edit_target());
    }

    #[test]
    fn target_above_limit_restores_last_committed_value_without_pending_search() {
        let mut state = FieldState::default();
        state.restore_target_input("12345".into());
        state.target_consumption = (crate::domain::rng::MAX_SFMT_FRAME + 1).to_string();
        state.target_dirty = true;

        let error = state.commit_target_input().unwrap_err();

        assert_eq!(state.target_consumption, "12345");
        assert!(!state.target_dirty);
        assert!(!state.target_research_pending);
        assert!(matches!(
            error,
            AppError::OutOfRange {
                field: InputField::TargetConsumption,
                ..
            }
        ));
    }

    #[test]
    fn target_input_reports_parse_and_negative_errors_and_restores_last_value() {
        let mut state = FieldState::default();
        state.restore_target_input("42".into());

        state.target_consumption = "not-a-frame".into();
        assert!(matches!(
            state.commit_target_input(),
            Err(AppError::InvalidInteger(InputField::TargetConsumption))
        ));
        assert_eq!(state.target_consumption, "42");

        state.target_consumption = "-1".into();
        assert!(matches!(
            state.commit_target_input(),
            Err(AppError::NegativeValue(InputField::TargetConsumption))
        ));
        assert_eq!(state.target_consumption, "42");
    }

    #[test]
    fn target_limit_itself_is_valid_and_becomes_the_rollback_value() {
        let mut state = FieldState::default();
        state.target_consumption = crate::domain::rng::MAX_SFMT_FRAME.to_string();
        state.target_dirty = true;

        state.commit_target_input().unwrap();

        assert_eq!(state.target_consumption, "50000000");
        state.target_consumption = (crate::domain::rng::MAX_SFMT_FRAME + 1).to_string();
        state.target_dirty = true;
        assert!(state.commit_target_input().is_err());
        assert_eq!(state.target_consumption, "50000000");
    }

    #[test]
    fn stopping_at_timeline_limit_clears_timer_and_marks_range_state() {
        let mut state = FieldState {
            timer_start: Some(Instant::now()),
            ..FieldState::default()
        };

        state.stop_blink_timer_out_of_range();

        assert!(!state.timer_is_running());
        assert!(state.next_blink_out_of_range);
        assert_eq!(state.timer_seconds_value(), 0.0);
        assert!(state.notice.is_none());
    }

    #[test]
    fn timer_does_not_fallback_to_a_blink_beyond_the_timeline_limit() {
        let mut state = FieldState::default();
        state.observed.push(Instant::now());
        state.results.push(FieldResult {
            start_consumption: crate::domain::rng::MAX_SFMT_FRAME,
            first_wait_frame: 0,
            last_wait_frame: 0,
            first_consumption: crate::domain::rng::MAX_SFMT_FRAME,
            last_consumption: crate::domain::rng::MAX_SFMT_FRAME,
            intervals: Vec::new(),
            target_consumptions: Vec::new(),
            next_blink_wait_frame: 30,
            target_wait_frame: None,
            target_exact: None,
            target_frame_before: None,
            target_frame_after: None,
            observed_model: 0,
            models: 1,
            timeline: Vec::new(),
            candidate_blinks: Vec::new(),
            target_blink_count: None,
            target_blink_count_offset_from_first_row: 0,
            timeline_cursor: Some(TimelineCursor {
                status: ModelStatus::new(1),
                stream_cursor: crate::domain::rng::MAX_SFMT_FRAME,
                consumption: crate::domain::rng::MAX_SFMT_FRAME,
                tick: 0,
            }),
            target_cursor: None,
        });

        let error = state
            .start_blink_timer(crate::domain::timing::DEFAULT_FPS)
            .unwrap_err();

        assert!(matches!(
            error,
            AppError::InvalidState(StateIssue::TimelineLimitReached)
        ));
        assert!(!state.timer_is_running());
        assert!(state.next_blink_out_of_range);
    }

    #[test]
    fn observation_input_is_capped_at_sixty_four_blinks() {
        let mut state = FieldState::default();
        state.mode = FieldMode::Observing;
        state.observed = (0..crate::application::state::MAX_BLINK_OBSERVATIONS)
            .map(|_| Instant::now())
            .collect();
        state.observe(crate::domain::timing::DEFAULT_FPS);
        assert_eq!(
            state.observed.len(),
            crate::application::state::MAX_BLINK_OBSERVATIONS
        );
        assert!(state.notice.is_none());
    }

    #[test]
    fn zero_candidate_search_allows_additional_shift_for_retry() {
        let mut state = FieldState::default();
        state.mode = FieldMode::Observing;
        state.search_exhausted = true;
        assert!(state.is_observation_input_enabled());
        state.start_observation();
        assert!(state.is_observation_input_enabled());
    }

    #[test]
    fn observation_reset_and_restart_clear_candidate_notice() {
        let mut state = FieldState::default();
        state.notice = Some("候補が見つかりません".to_owned());

        state.cancel_observation();

        assert!(state.notice.is_none());

        state.notice = Some("候補が見つかりません".to_owned());
        state.start_observation();

        assert!(state.notice.is_none());
    }

    #[test]
    fn invalidating_search_results_keeps_observation_but_clears_runtime_state() {
        let mut state = FieldState::default();
        state.observed.push(Instant::now());
        state.intervals.push(200);
        state.results.push(FieldResult {
            start_consumption: 477,
            first_wait_frame: 2,
            last_wait_frame: 4,
            first_consumption: 478,
            last_consumption: 479,
            intervals: vec![2],
            target_consumptions: Vec::new(),
            next_blink_wait_frame: 2,

            target_wait_frame: Some(4),
            target_exact: Some(true),
            target_frame_before: Some(10),
            target_frame_after: Some(10),
            observed_model: 0,
            models: 2,
            timeline: Vec::new(),
            candidate_blinks: Vec::new(),
            target_blink_count: Some(1),
            target_blink_count_offset_from_first_row: 0,
            timeline_cursor: None,
            target_cursor: None,
        });
        state.timer_start = Some(Instant::now());
        state.target_dirty = true;

        state.invalidate_search_results();

        assert!(state.results.is_empty());
        assert!(state.inferred_indices.is_empty());
        assert!(state.timer_start.is_none());
        assert_eq!(state.observed.len(), 1);
        assert_eq!(state.intervals, vec![200]);
        assert!(state.target_dirty);
        assert!(state.notice.is_none());
    }

    #[test]
    fn target_edit_search_results_are_accepted_while_cancelled_results_are_not() {
        let mut state = FieldState::default();
        state.mode = FieldMode::Idle;
        state.intervals = vec![200, 210];
        assert!(state.accepts_search_result());

        state.cancel_observation();
        assert!(!state.accepts_search_result());
    }

    #[test]
    fn timer_kind_can_toggle_when_idle() {
        let mut state = FieldState::default();
        state.observed.push(Instant::now());
        state.results.push(FieldResult {
            start_consumption: 477,
            first_wait_frame: 2,
            last_wait_frame: 4,
            first_consumption: 478,
            last_consumption: 479,
            intervals: vec![2],
            target_consumptions: Vec::new(),
            next_blink_wait_frame: 2,
            target_wait_frame: Some(4),
            target_exact: Some(true),
            target_frame_before: Some(10),
            target_frame_after: Some(10),
            observed_model: 0,
            models: 2,
            timeline: Vec::new(),
            candidate_blinks: Vec::new(),
            target_blink_count: Some(1),
            target_blink_count_offset_from_first_row: 0,
            timeline_cursor: None,
            target_cursor: None,
        });
        assert_eq!(state.timer_kind, FieldTimerKind::NextBlink);
        state.toggle_timer_kind(59.8621).unwrap();
        assert_eq!(state.timer_kind, FieldTimerKind::Target);
        state.toggle_timer_kind(59.8621).unwrap();
        assert_eq!(state.timer_kind, FieldTimerKind::NextBlink);
    }

    #[test]
    fn field_offset_only_shifts_target_wait_frame() {
        let mut state = FieldState::default();
        state.use_offset = true;
        state.encounter_offset = "+2".into();
        let result = FieldResult {
            start_consumption: 0,
            first_wait_frame: 2,
            last_wait_frame: 4,
            first_consumption: 1,
            last_consumption: 2,
            intervals: vec![2],
            target_consumptions: Vec::new(),
            next_blink_wait_frame: 2,
            target_wait_frame: Some(100),
            target_exact: Some(true),
            target_frame_before: Some(50),
            target_frame_after: Some(50),
            observed_model: 0,
            models: 2,
            timeline: Vec::new(),
            candidate_blinks: Vec::new(),
            target_blink_count: Some(1),
            target_blink_count_offset_from_first_row: 0,
            timeline_cursor: None,
            target_cursor: None,
        };
        assert_eq!(state.target_wait_frame_with_offset(&result), Some(102));
        state.encounter_offset = "-2".into();
        assert_eq!(state.target_wait_frame_with_offset(&result), Some(98));
        state.encounter_offset = (MAX_SFMT_FRAME + 2).to_string();
        assert_eq!(state.target_wait_frame_with_offset(&result), None);
        state.encounter_offset.clear();
        assert_eq!(state.target_wait_frame_with_offset(&result), Some(100));
        state.use_offset = false;
        assert_eq!(state.target_wait_frame_with_offset(&result), Some(100));
    }

    #[test]
    fn completed_target_timer_stays_at_zero_until_reset() {
        let mut state = FieldState::default();
        state.timer_kind = FieldTimerKind::Target;
        state.timer_finished = true;
        assert_eq!(
            state.preview_remaining(FieldTimerKind::Target, 59.8621),
            Some(0.0)
        );
        state.cancel_observation();
        assert_eq!(
            state.preview_remaining(FieldTimerKind::Target, 59.8621),
            None
        );
    }

    #[test]
    fn field_target_timer_and_current_position_handle_max_frame_virtual_elapsed() {
        let fps = crate::domain::timing::DEFAULT_FPS;
        let start = Instant::now();
        let target_duration = game_frames_to_seconds(MAX_SFMT_FRAME as f64, fps);
        let virtual_now = start
            + std::time::Duration::from_secs_f64(game_frames_to_seconds(
                (MAX_SFMT_FRAME + 100) as f64,
                fps,
            ));
        let mut state = FieldState::default();
        state.observed.push(start);
        state.timer_kind = FieldTimerKind::Target;
        state.timer_start = Some(start);
        state.timer_seconds = target_duration;
        state.results.push(FieldResult {
            start_consumption: 486,
            first_wait_frame: 0,
            last_wait_frame: 486,
            first_consumption: 486,
            last_consumption: 486,
            intervals: Vec::new(),
            target_consumptions: Vec::new(),
            next_blink_wait_frame: 0,
            target_wait_frame: Some(MAX_SFMT_FRAME),
            target_exact: Some(true),
            target_frame_before: Some(MAX_SFMT_FRAME),
            target_frame_after: Some(MAX_SFMT_FRAME),
            observed_model: 0,
            models: 1,
            timeline: Vec::new(),
            candidate_blinks: Vec::new(),
            target_blink_count: Some(0),
            target_blink_count_offset_from_first_row: 0,
            timeline_cursor: None,
            target_cursor: None,
        });

        assert_eq!(state.timer_remaining_at(virtual_now), Some(0.0));
        assert!(state.finish_target_timer_if_elapsed(virtual_now));
        assert!(state.timer_is_running());
        assert!(state.timer_is_finished());
        assert!(state
            .current_timeline_frame_at(fps, virtual_now)
            .is_some_and(|frame| frame > MAX_SFMT_FRAME));
        assert!(state
            .remaining_from_observation_at(MAX_SFMT_FRAME, fps, virtual_now)
            .is_err());
    }

    #[test]
    fn target_timing_distinguishes_exact_and_crossing_steps() {
        let values = SearchValues::new(0, generate_u64_stream(1, 32));
        let event = BlinkEvent {
            wait_frame: 0,
            consumption: 0,
            status: ModelStatus::new(2),
            stream_cursor: 0,
            model_consumption: None,
        };
        let crossing = find_target_timing(&event, 1, &values).unwrap();
        assert!(!crossing.exact);
        assert_eq!(crossing.absolute_wait_frame, 2);
        assert_eq!((crossing.frame_before, crossing.frame_after), (0, 2));

        let exact = find_target_timing(&event, 2, &values).unwrap();
        assert!(exact.exact);
        assert_eq!(exact.absolute_wait_frame, 2);
    }
}
