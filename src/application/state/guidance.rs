use super::{
    AppState, FASTEST_CLOSE_SEARCH_BUFFER, GUIDANCE_CHUNK, MAX_FRAME, MAX_PRODUCTION_LOOKAHEAD,
};
#[cfg(test)]
use crate::application::production::ProductionQuery;
use crate::application::state_types::{
    CandidateReachabilityKey, CandidateReachabilityRequest, GuidanceSearchRequest, HatchGuidance,
    SearchConfig,
};
#[cfg(test)]
use crate::domain::rng::target_reachability_range;
use crate::domain::rng::{
    advance_timeline_from_values, target_reachability_range_from_cache, ObservationCache,
};
use crate::errors::{AppError, CacheError};
#[cfg(test)]
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use std::collections::HashMap;
use std::time::Instant;

impl AppState {
    // --- Candidate reachability and guidance ------------------------------

    /// 候補Vecを複製せず、到達判定要求の変更だけを検出する軽量key。
    pub fn candidate_reachability_key(&self) -> Option<CandidateReachabilityKey> {
        if self.candidates.len() <= 1 || self.encounter_offset_error().is_some() {
            return None;
        }
        let config = self.search_config().ok()?;
        let fastest_ticks = self
            .fastest_close
            .then(|| self.encounter_offset_ticks())
            .flatten();
        if self.fastest_close && fastest_ticks.is_none() {
            return None;
        }
        Some(CandidateReachabilityKey {
            session_generation: self.guidance.session_generation(),
            candidate_generation: self.candidate_generation,
            selected_candidate: self.selected,
            selected_position: self.candidate_position(self.selected)?,
            seed: config.seed,
            target: config.target,
            models: config.npc_models,
            offset: self.encounter_offset_value(),
            consider_rotom: self.consider_rotom_talk,
            rotom_threshold: self.rotom_threshold_value(),
            pre_rotom_consumption: self.pre_rotom_consumption_value(),
            npc_initial_load: self.npc_initial_load_value(),
            fastest_ticks,
        })
    }

    /// key変更時だけ、候補到達worker用の全件スナップショットを構築する。
    pub fn candidate_reachability_request_for_key(
        &self,
        key: CandidateReachabilityKey,
    ) -> Option<CandidateReachabilityRequest> {
        if self.candidate_reachability_key()? != key {
            return None;
        }
        let _profile = crate::application::hot_path_metrics::HotPathTimer::start(
            crate::application::hot_path_metrics::HotPath::CandidateSnapshot,
        )
        .with_items(self.candidates.len());
        Some(CandidateReachabilityRequest {
            session_generation: key.session_generation,
            seed: key.seed,
            target: key.target,
            models: key.models,
            offset: key.offset,
            consider_rotom: key.consider_rotom,
            rotom_threshold: key.rotom_threshold,
            // 実測のお喋り有無は補正／本番計算専用。右上の○×は設定値で判定する。
            observed_rotom_talk: None,
            pre_rotom_consumption: key.pre_rotom_consumption,
            npc_initial_load: key.npc_initial_load,
            fastest_ticks: key.fastest_ticks,
            candidate_positions: self
                .candidates
                .iter()
                .enumerate()
                .filter_map(|(index, _)| self.candidate_position(index).map(|frame| (index, frame)))
                .collect(),
        })
    }

    /// 候補ごとの本番到達判定をワーカースレッドへ渡すスナップショット。
    pub fn candidate_reachability_request(&self) -> Option<CandidateReachabilityRequest> {
        self.candidate_reachability_request_for_key(self.candidate_reachability_key()?)
    }

    /// 候補ごとの本番Timeline判定。重い処理なのでUiAppからバックグラウンド
    /// で呼び出し、完了後に同じ入力スナップショットへだけ反映する。
    #[cfg(test)]
    pub fn calculate_candidate_reachability(
        request: CandidateReachabilityRequest,
    ) -> Vec<(usize, bool)> {
        request
            .candidate_positions
            .into_par_iter()
            .map(|(candidate_pos, current)| {
                if request.target.saturating_sub(current) > MAX_PRODUCTION_LOOKAHEAD {
                    return (candidate_pos, false);
                }
                let reachable = ProductionQuery {
                    seed: request.seed,
                    target: request.target,
                    models: request.models,
                    offset: request.offset,
                    consider_rotom: request.consider_rotom,
                    rotom_threshold: request.rotom_threshold,
                    observed_rotom_talk: request.observed_rotom_talk,
                    pre_rotom_consumption: request.pre_rotom_consumption,
                    npc_initial_load: request.npc_initial_load,
                    fastest_ticks: request.fastest_ticks,
                }
                .target_reachable(current);
                (candidate_pos, reachable)
            })
            .collect()
    }

    /// 候補複数時の到達判定を、観測時に作成したSFMTキャッシュから計算する。
    ///
    /// 候補ごとに`ProductionQuery`を呼ぶと、それぞれがSeedから開始Frameまで
    /// SFMTを再生する。候補範囲を一度だけ位置指定で読み、同じ列を共有する。
    pub fn calculate_candidate_reachability_from_cache(
        request: CandidateReachabilityRequest,
        cache: &ObservationCache,
    ) -> Result<Vec<(usize, bool)>, AppError> {
        let false_results = || {
            request
                .candidate_positions
                .iter()
                .map(|(candidate, _)| (*candidate, false))
                .collect::<Vec<_>>()
        };
        if cache.seed != request.seed {
            return Err(AppError::Cache(CacheError::SeedMismatch));
        }
        if request.target.saturating_sub(
            request
                .candidate_positions
                .iter()
                .map(|(_, frame)| *frame)
                .min()
                .unwrap_or(request.target),
        ) > MAX_PRODUCTION_LOOKAHEAD
        {
            return Ok(false_results());
        }
        let Some(min_observed) = request
            .candidate_positions
            .iter()
            .map(|(_, frame)| *frame)
            .min()
        else {
            return Ok(Vec::new());
        };
        let max_observed = request
            .candidate_positions
            .iter()
            .map(|(_, frame)| *frame)
            .max()
            .unwrap_or(min_observed);
        let Some(b_start) = min_observed.checked_add(request.pre_rotom_consumption) else {
            return Ok(false_results());
        };
        let Some(b_count) = max_observed
            .checked_sub(min_observed)
            .and_then(|span| span.checked_add(1))
            .and_then(|count| usize::try_from(count).ok())
        else {
            return Ok(false_results());
        };
        let b_values = cache.read_values(b_start, b_count)?;
        let mut planned = Vec::with_capacity(request.candidate_positions.len());
        for &(candidate, observed) in &request.candidate_positions {
            let Some(value) = observed
                .checked_sub(min_observed)
                .and_then(|offset| usize::try_from(offset).ok())
                .and_then(|offset| b_values.get(offset).copied())
            else {
                return Ok(false_results());
            };
            let Some(plan) = crate::domain::rng::encounter_plan_with_pre_rotom_value(
                observed,
                request.offset,
                request.consider_rotom,
                request.rotom_threshold,
                request.observed_rotom_talk,
                request.pre_rotom_consumption,
                request.npc_initial_load,
                value,
            ) else {
                return Ok(false_results());
            };
            planned.push((candidate, plan.encounter_start));
        }
        if let Some(ticks) = request.fastest_ticks {
            let Some(min_start) = planned.iter().map(|(_, start)| *start).min() else {
                return Ok(Vec::new());
            };
            let max_start = planned
                .iter()
                .map(|(_, start)| *start)
                .max()
                .unwrap_or(min_start);
            let Some(count) = max_start
                .checked_sub(min_start)
                .and_then(|span| {
                    span.checked_add(ticks.max(0).saturating_mul(request.models as i64))
                })
                .and_then(|span| span.checked_add(request.models as i64 + 1))
                .and_then(|count| usize::try_from(count).ok())
            else {
                return Ok(false_results());
            };
            let values = cache.read_values(min_start, count)?;
            for (_, start) in &mut planned {
                let Some(advanced) =
                    advance_timeline_from_values(&values, min_start, *start, request.models, ticks)
                else {
                    return Ok(false_results());
                };
                *start = advanced;
            }
        }
        let Some(min_start) = planned.iter().map(|(_, start)| *start).min() else {
            return Ok(Vec::new());
        };
        let max_start = planned
            .iter()
            .map(|(_, start)| *start)
            .max()
            .unwrap_or(min_start);
        let by_start: HashMap<i64, bool> = target_reachability_range_from_cache(
            cache,
            min_start..=max_start,
            request.target,
            request.models,
            None,
        )?
        .into_iter()
        .collect();
        Ok(planned
            .into_iter()
            .map(|(candidate, start)| (candidate, by_start.get(&start).copied().unwrap_or(false)))
            .collect())
    }

    pub fn apply_candidate_reachability(
        &mut self,
        request: &CandidateReachabilityRequest,
        results: &[(usize, bool)],
    ) {
        if !self
            .candidate_reachability_request()
            .is_some_and(|current| current == *request)
        {
            return;
        }
        for &(candidate_pos, reachable) in results {
            if let Some(candidate) = self.candidates.get_mut(candidate_pos) {
                candidate.target_reachable = reachable;
            }
        }
    }

    pub fn recalculate_production(&mut self) {
        self.clear_guidance_reachability();
        self.fast_close_circle_frames.clear();
        self.production = None;
        self.production_preparation_pending = !self.candidates.is_empty();
        self.encounter_start_requested = false;
        for candidate in &mut self.candidates {
            candidate.target_reachable = false;
        }
    }

    /// 実測ロトム結果だけを更新したときの本番経路更新。右上の○×は
    /// 設定確率だけで判定するため、既に計算済みの案内結果を消さない。
    pub fn recalculate_production_preserving_guidance(&mut self) {
        self.mark_production_preparation_pending_preserving_guidance();
    }

    /// 表の指定位置からTarget Frameへ到達できるかを遅延計算する。
    /// 到達可否は瞬き検索の結果ではなく、本番NPCタイムラインの結果で決まる。
    #[cfg(test)]
    pub fn table_target_reachable(&mut self, frame: i64) -> Option<bool> {
        if let Some(reachable) = self.guidance.reachability(frame) {
            return Some(reachable);
        }
        let config = self.search_config().ok()?;
        if config.target.saturating_sub(frame) > MAX_PRODUCTION_LOOKAHEAD {
            self.record_guidance_reachability(frame, false);
            return Some(false);
        }
        let plan = self.plan_for_encounter(
            config.seed,
            frame,
            self.encounter_offset_value(),
            self.consider_rotom_talk,
            self.rotom_threshold_value(),
            None,
        )?;
        // 観測候補が1つに決まった後は、その候補上の未来位置に同じ本番経路を再利用する。
        // 瞬き表示ごとのModelStatus再初期化を避け、経路外の位置だけ独立計算へ戻す。
        if !self.fastest_close && self.candidates.len() == 1 {
            if let Some(cache) = self.production_cache.as_ref() {
                if cache.seed == config.seed && cache.models == config.npc_models {
                    if let Some(timing) = cache.target_from(plan.encounter_start, config.target) {
                        let reachable = timing.exact_tick.is_some();
                        self.record_guidance_reachability(frame, reachable);
                        return Some(reachable);
                    }
                }
            }
        }
        let reachable = ProductionQuery {
            seed: config.seed,
            target: config.target,
            models: config.npc_models,
            offset: self.encounter_offset_value(),
            consider_rotom: self.consider_rotom_talk,
            rotom_threshold: self.rotom_threshold_value(),
            observed_rotom_talk: None,
            pre_rotom_consumption: self.pre_rotom_consumption_value(),
            npc_initial_load: self.npc_initial_load_value(),
            fastest_ticks: self
                .fastest_close
                .then(|| self.encounter_offset_ticks())
                .flatten(),
        }
        .target_exactly_reached(frame)
        .unwrap_or(false);
        self.record_guidance_reachability(frame, reachable);
        Some(reachable)
    }

    /// 次の右上案内チャンクを作るための入力を返す。ここでは範囲と現在の
    /// キャッシュ終端だけを確認し、SFMT計算は行わない。
    pub fn guidance_search_request(&self) -> Option<GuidanceSearchRequest> {
        // Target欄の編集中は、1文字ごとの暫定値で案内Workerを起動しない。
        // 確定時にUIが案内結果を破棄してから、新しい設定で再検索する。
        if self.target_dirty {
            return None;
        }
        let plan = self.guidance_scan_plan()?;
        if plan.target_too_far {
            return None;
        }
        let observed_start = plan.known_end.saturating_add(1).max(plan.search_start);
        if observed_start > plan.search_end {
            return None;
        }
        let observed_end = observed_start
            .saturating_add(GUIDANCE_CHUNK.saturating_sub(1))
            .min(plan.search_end);
        let config = plan.config;
        Some(GuidanceSearchRequest {
            session_generation: self.guidance.session_generation(),
            observed_start,
            observed_end,
            range_start: config.range_start,
            range_end: config.range_end,
            seed: config.seed,
            target: config.target,
            models: config.npc_models,
            offset: self.encounter_offset_value(),
            consider_rotom: self.consider_rotom_talk,
            rotom_threshold: self.rotom_threshold_value(),
            // 右上の○×案内は実測ドロップダウンから独立させる。
            observed_rotom_talk: None,
            pre_rotom_consumption: self.pre_rotom_consumption_value(),
            npc_initial_load: self.npc_initial_load_value(),
            fastest_ticks: plan.fastest_ticks,
        })
    }

    /// バックグラウンド計算済みの案内結果だけを状態へ反映する。
    #[cfg(test)]
    pub fn apply_guidance_results(
        &mut self,
        request: GuidanceSearchRequest,
        results: Vec<(i64, bool)>,
    ) {
        // Workerの完了順は保証されないため、状態側でも現在の設定と同じ
        // セッションの完全な結果だけを受け付ける。
        if !self.guidance_batch_is_current_and_complete(request, results.len(), |index| {
            results.get(index).map(|(frame, _)| *frame)
        }) {
            return;
        }
        if let (Some(cache), Some(count)) = (
            self.observation_cache.as_ref(),
            request
                .observed_end
                .checked_sub(request.observed_start)
                .and_then(|span| span.checked_add(1))
                .and_then(|count| usize::try_from(count).ok()),
        ) {
            if let Ok(seconds) = cache.interval_seconds_range(request.observed_start, count) {
                self.guidance
                    .cache_interval_seconds(request.observed_start, &seconds);
            }
        }
        for (frame, reachable) in results {
            self.record_guidance_reachability(frame, reachable);
        }
        self.guidance.mark_computed_until(request.observed_end);
    }

    /// 到達可否と同じ世代で計算された秒数も一括で反映する。
    pub fn apply_guidance_batch_results(
        &mut self,
        request: GuidanceSearchRequest,
        results: Vec<(i64, bool, f64)>,
    ) {
        if !self.guidance_batch_is_current_and_complete(request, results.len(), |index| {
            results.get(index).map(|(frame, _, _)| *frame)
        }) || results
            .iter()
            .any(|(_, _, seconds)| !seconds.is_finite() || *seconds < 0.0)
        {
            return;
        }
        let seconds = results
            .iter()
            .map(|(_, _, seconds)| *seconds)
            .collect::<Vec<_>>();
        self.guidance
            .cache_interval_seconds(request.observed_start, &seconds);
        for (frame, reachable, _) in results {
            self.record_guidance_reachability(frame, reachable);
        }
        self.guidance.mark_computed_until(request.observed_end);
    }

    pub(super) fn guidance_batch_is_current_and_complete(
        &self,
        request: GuidanceSearchRequest,
        result_len: usize,
        frame_at: impl Fn(usize) -> Option<i64>,
    ) -> bool {
        let Some(current_request) = self.guidance_search_request() else {
            return false;
        };
        if !current_request.same_configuration(request) {
            return false;
        }
        let expected_len = request
            .observed_end
            .checked_sub(request.observed_start)
            .and_then(|span| span.checked_add(1))
            .and_then(|length| usize::try_from(length).ok());
        expected_len.is_some_and(|length| {
            result_len == length
                && (0..length).all(|index| {
                    frame_at(index) == Some(request.observed_start.saturating_add(index as i64))
                })
        })
    }

    /// 右上の○×案内。現在位置から次に○となる孵化瞬き位置と、その位置が
    /// 続く実時間を同じSFMT行から求める。
    pub fn hatch_guidance(&mut self) -> Result<Option<HatchGuidance>, AppError> {
        let mut profile = crate::application::hot_path_metrics::HotPathTimer::start(
            crate::application::hot_path_metrics::HotPath::HatchGuidance,
        );
        profile.set_items(1);
        let search_pending = self.guidance_search_pending();
        self.hatch_guidance_with_pending(search_pending)
    }

    /// 同じ画面frameで共有する案内snapshotと検索pending状態を一度に作る。
    pub fn hatch_guidance_snapshot(&mut self) -> Result<(Option<HatchGuidance>, bool), AppError> {
        let mut profile = crate::application::hot_path_metrics::HotPathTimer::start(
            crate::application::hot_path_metrics::HotPath::HatchGuidance,
        );
        profile.set_items(1);
        let search_pending = self.guidance_search_pending();
        let guidance = self.hatch_guidance_with_pending(search_pending)?;
        Ok((guidance, search_pending))
    }

    pub(super) fn hatch_guidance_with_pending(
        &mut self,
        search_pending: bool,
    ) -> Result<Option<HatchGuidance>, AppError> {
        if self.observation_cache.is_none() {
            return Ok(None);
        }
        let Some(plan) = self.guidance_scan_plan() else {
            return Ok(None);
        };
        if plan.target_too_far {
            return Ok(Some(HatchGuidance {
                current_frame: plan.current,
                current_reachability: Some(false),
                reachable_now: false,
                blinks_until_circle: 0,
                next_circle_frame: None,
                next_circle_seconds: None,
                grace_seconds: None,
                target_wait_seconds: None,
                target_unreachable: false,
                search_out_of_range: true,
            }));
        }
        let current = plan.current;
        // 最速閉じの理論範囲外はWorkerへ渡していないが、到達不能である
        // ことが既に確定している。未計算(None)として扱うと、検索完了後も
        // 現在位置が`?`のままになるため、ここで確定×へ変換する。
        let current_reachability =
            if self.fastest_close && (current < plan.search_start || current > plan.search_end) {
                Some(false)
            } else {
                self.guidance.reachability(current)
            };
        let fastest_bounds = plan.fastest_bounds;
        let search_start = plan.search_start;
        let scan_end = plan.known_end.min(plan.search_end);
        // 理論上の候補範囲より前は最速閉じでは○にならないため、そこを
        // 計算せずに飛ばしつつ、到達可能位置の順序付き索引から次○を取る。
        let next_circle = (search_start <= scan_end)
            .then(|| self.guidance.next_reachable(search_start, scan_end))
            .flatten();
        let blinks = next_circle
            .map(|frame| {
                frame
                    .saturating_sub(current)
                    .try_into()
                    .unwrap_or(usize::MAX)
            })
            .unwrap_or_else(|| {
                let frames_checked = scan_end
                    .checked_sub(search_start)
                    .and_then(|value| value.checked_add(1))
                    .unwrap_or(0);
                search_start
                    .saturating_sub(current)
                    .saturating_add(frames_checked)
                    .try_into()
                    .unwrap_or(usize::MAX)
            });
        let reachable_now = current_reachability == Some(true);
        // 現在が○のときは現在行を数えず、その次に現れる○までの
        // 瞬き回数を表示する。現在が×のときは従来どおり、現在位置
        // から最初の○までの回数を表示する。
        let blinks_until_circle = if reachable_now {
            let next_start = current.saturating_add(1);
            if next_start <= scan_end {
                self.guidance
                    .next_reachable(next_start, scan_end)
                    .map(|frame| {
                        frame
                            .saturating_sub(current)
                            .try_into()
                            .unwrap_or(usize::MAX)
                    })
                    .unwrap_or_else(|| 0)
            } else {
                0
            }
        } else {
            blinks
        };
        if self.fastest_close && reachable_now && !self.fast_close_circle_frames.contains(&current)
        {
            self.fast_close_circle_frames.push(current);
            if self.fast_close_circle_frames.len() > 16 {
                self.fast_close_circle_frames.remove(0);
            }
        }
        let next_circle_seconds = if let Some(end) = next_circle {
            if let Some(count) = end
                .checked_sub(current)
                .and_then(|value| usize::try_from(value).ok())
            {
                if count == 0 {
                    Some(0.0)
                } else if let (Some(full_seconds), Some(first_full)) = (
                    self.guidance.interval_seconds_sum(current, count),
                    self.guidance.interval_seconds_at(current),
                ) {
                    // 最初の区間は既に一部経過している場合がある。瞬きタイマー実行中は区間全体の
                    // 長さを現在の締切へ置き換え、整数表示を実時間の経過に合わせる。
                    let first_remaining = self
                        .next_blink
                        .map(|deadline| {
                            deadline
                                .saturating_duration_since(Instant::now())
                                .as_secs_f64()
                        })
                        .unwrap_or(first_full);
                    Some((full_seconds - first_full + first_remaining).max(0.0))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        let grace_seconds = if let Some(start) = next_circle {
            // 未来の検索が続いている場合、既知範囲の末尾まで○が続くだけ
            // では猶予秒数を確定できない。既知の×に到達した場合は、そこが
            // ○区間の正常な終端なので、その時点で確定できる。
            let first_unreachable = self.guidance.first_unreachable(start, scan_end);
            let complete = first_unreachable.is_some() || !search_pending;
            let last_reachable = first_unreachable
                .map(|frame| frame.saturating_sub(1))
                .unwrap_or(scan_end);
            let count = last_reachable
                .checked_sub(start)
                .and_then(|value| value.checked_add(1))
                .and_then(|value| usize::try_from(value).ok())
                .unwrap_or(0);
            let total = self.guidance.interval_seconds_sum(start, count);
            total.filter(|seconds| complete && *seconds > 0.0)
        } else {
            None
        };
        // Targetの検索範囲外表示は、現在地からの共通上限を超えた場合
        // だけに限定する。通常待機では観測範囲の終点を越えても、現在の
        // TimelineからTargetまでの本番待機時間を計算できる。
        let search_out_of_range = !self.target_dirty && plan.target_too_far;
        // 最速閉じでは理論上の○が残っていない場合、通常待機ではTargetが
        // 現在位置より前にある場合を到達不能として表示する。検索範囲外は
        // 到達不能とは別の状態なので、そちらを優先する。
        let fastest_target_unreachable = self.fastest_close
            && next_circle.is_none()
            && !search_pending
            && fastest_bounds.is_some_and(|bounds| bounds.2);
        let target_unreachable = !self.target_dirty
            && !search_out_of_range
            && (fastest_target_unreachable
                || (!self.fastest_close && plan.config.target < current));
        let target_wait_seconds = if self.fastest_close || self.production_preparation_pending {
            None
        } else {
            let target_ticks = self
                .production
                .as_ref()
                .and_then(|production| production.target_tick);
            let target_ticks = match target_ticks {
                Some(ticks) => Some(ticks),
                None => match self.normal_target_wait_ticks(current) {
                    None => None,
                    Some(Some(ticks)) => Some(ticks),
                    Some(None) => next_circle
                        .and_then(|frame| self.normal_target_wait_ticks(frame))
                        .flatten(),
                },
            };
            target_ticks.map(|ticks| self.game_ticks_to_seconds(ticks))
        };
        Ok(Some(HatchGuidance {
            current_frame: current,
            current_reachability,
            reachable_now,
            blinks_until_circle,
            next_circle_frame: next_circle,
            next_circle_seconds,
            grace_seconds,
            target_wait_seconds,
            target_unreachable,
            search_out_of_range,
        }))
    }

    /// 右上案内の次チャンクを継続計算する必要があるか。
    pub fn guidance_search_pending(&self) -> bool {
        if self.target_dirty {
            return false;
        }
        let Some(plan) = self.guidance_scan_plan() else {
            return false;
        };
        !plan.target_too_far && plan.known_end < plan.search_end
    }

    /// 最速閉じでTargetへ到達し得る観測位置の範囲を返す。
    ///
    /// 1ステップで消費できるSFMT値は、現在のModelStatus実装ではモデル1体
    /// あたり最大1個である。そのため、`k = オフセットF / 2` ステップ後に
    /// Targetへ到達する開始位置は `Target - k * models` より前にならない。
    /// 観測位置はロトム総消費・NPC初期読み込みの分だけさらに手前なので、
    /// そこへ100Fの安全余白を加えて検索する。戻り値は
    /// (検索下限, 検索上限, 設定範囲が理論範囲全体を覆うか) の順。
    pub(super) fn fastest_close_guidance_bounds(
        &self,
        config: &SearchConfig,
    ) -> Option<(i64, i64, bool)> {
        let ticks = self.encounter_offset_ticks()?;
        let models = i64::try_from(config.npc_models).ok()?;
        let max_consumption = ticks.checked_mul(models)?;
        let rotom_max = if self.consider_rotom_talk { 2 } else { 0 };
        let rotom_min = if self.consider_rotom_talk { 1 } else { 0 };
        let pre_rotom_consumption = self.pre_rotom_consumption_value();
        let npc_initial_load = self.npc_initial_load_value();
        let theoretical_start = config
            .target
            .saturating_sub(max_consumption)
            .saturating_sub(pre_rotom_consumption)
            .saturating_sub(rotom_max)
            .saturating_sub(npc_initial_load);
        let theoretical_end = config
            .target
            .saturating_sub(pre_rotom_consumption)
            .saturating_sub(rotom_min)
            .saturating_sub(npc_initial_load);
        let effective_theoretical_start = theoretical_start.max(0);
        let effective_theoretical_end = theoretical_end.max(0);
        let search_start = theoretical_start
            .saturating_sub(FASTEST_CLOSE_SEARCH_BUFFER)
            .max(config.range_start)
            .max(0);
        let search_end = theoretical_end
            .saturating_add(FASTEST_CLOSE_SEARCH_BUFFER)
            .min(MAX_FRAME);
        let fully_covered =
            search_start <= effective_theoretical_start && search_end >= effective_theoretical_end;
        Some((search_start, search_end, fully_covered))
    }

    #[cfg(test)]
    pub fn calculate_guidance_batch(request: GuidanceSearchRequest) -> Vec<(i64, bool)> {
        // UIは表示中の現在位置から始まるチャンクだけを要求する。Targetが本番窓の上限を
        // 超えている場合は全てfalseの軽い結果を返し、遠いTargetまでSFMTを生成しない。
        if request.target.saturating_sub(request.observed_start) > MAX_PRODUCTION_LOOKAHEAD {
            return (request.observed_start..=request.observed_end)
                .map(|frame| (frame, false))
                .collect();
        }
        let query = ProductionQuery {
            seed: request.seed,
            target: request.target,
            models: request.models,
            offset: request.offset,
            consider_rotom: request.consider_rotom,
            rotom_threshold: request.rotom_threshold,
            observed_rotom_talk: request.observed_rotom_talk,
            pre_rotom_consumption: request.pre_rotom_consumption,
            npc_initial_load: request.npc_initial_load,
            fastest_ticks: request.fastest_ticks,
        };
        let mut planned = Vec::new();
        for frame in request.observed_start..=request.observed_end {
            // `target_reachability_range`のtickをB+ロトム開始位置から適用する。
            // ここは開始位置そのものを表すため、同じtickをここでも進めるとTarget比較
            // 前に全候補を二重に移動してしまうため進めない。
            if let Some(start) = query.base_encounter_start(frame) {
                planned.push((frame, start));
            }
        }
        let Some(min_start) = planned.iter().map(|(_, start)| *start).min() else {
            return Vec::new();
        };
        let max_start = planned
            .iter()
            .map(|(_, start)| *start)
            .max()
            .unwrap_or(min_start);
        let by_start: HashMap<i64, bool> = target_reachability_range(
            request.seed,
            min_start..=max_start,
            request.target,
            request.models,
            request.fastest_ticks,
        )
        .into_iter()
        .collect();
        // 要求された各観測Frameに必ず1行を返す。行を落とすと表示側の計算済み終点だけが
        // 未計算位置を越え、UIに横棒が残り続ける。
        let start_by_observed: HashMap<i64, i64> = planned.into_iter().collect();
        (request.observed_start..=request.observed_end)
            .map(|observed| {
                let reachable = start_by_observed
                    .get(&observed)
                    .and_then(|start| by_start.get(start))
                    .copied()
                    .unwrap_or(false);
                (observed, reachable)
            })
            .collect()
    }

    /// 既生成のSFMTキャッシュだけを使って、右上案内の1チャンクを計算する。
    ///
    /// チャンクごとにSeedから現在位置まで再生すると、高いFrameでは同じ長い
    /// SFMT prefixを何度も再計算することになる。ここではB位置とTarget判定の
    /// 両方を絶対位置指定でキャッシュから読み、作業量を対象チャンクに限定する。
    pub fn calculate_guidance_batch_from_cache(
        request: GuidanceSearchRequest,
        cache: &ObservationCache,
    ) -> Result<Vec<(i64, bool, f64)>, AppError> {
        let empty_results = || {
            (request.observed_start..=request.observed_end)
                .map(|frame| (frame, false, 0.0))
                .collect::<Vec<_>>()
        };
        if cache.seed != request.seed {
            return Err(AppError::Cache(CacheError::SeedMismatch));
        }
        if request.target.saturating_sub(request.observed_start) > MAX_PRODUCTION_LOOKAHEAD {
            return Ok(empty_results());
        }
        let Some(b_start) = request
            .observed_start
            .checked_add(request.pre_rotom_consumption)
        else {
            return Ok(empty_results());
        };
        let Some(b_count) = request
            .observed_end
            .checked_sub(request.observed_start)
            .and_then(|span| span.checked_add(1))
            .and_then(|count| usize::try_from(count).ok())
        else {
            return Ok(empty_results());
        };
        let interval_seconds = cache.interval_seconds_range(request.observed_start, b_count)?;
        if interval_seconds.len() != b_count {
            return Ok(empty_results());
        }
        let b_values = cache.read_values(b_start, b_count)?;
        let mut planned = Vec::with_capacity(b_count);
        for (index, frame) in (request.observed_start..=request.observed_end).enumerate() {
            let Some(value) = b_values.get(index).copied() else {
                return Ok(empty_results());
            };
            let Some(plan) = crate::domain::rng::encounter_plan_with_pre_rotom_value(
                frame,
                request.offset,
                request.consider_rotom,
                request.rotom_threshold,
                request.observed_rotom_talk,
                request.pre_rotom_consumption,
                request.npc_initial_load,
                value,
            ) else {
                return Ok(empty_results());
            };
            planned.push((frame, plan.encounter_start));
        }
        let Some(min_start) = planned.iter().map(|(_, start)| *start).min() else {
            return Ok(empty_results());
        };
        let max_start = planned
            .iter()
            .map(|(_, start)| *start)
            .max()
            .unwrap_or(min_start);
        let by_start: HashMap<i64, bool> = target_reachability_range_from_cache(
            cache,
            min_start..=max_start,
            request.target,
            request.models,
            request.fastest_ticks,
        )?
        .into_iter()
        .collect();
        Ok(planned
            .into_iter()
            .zip(interval_seconds)
            .map(|((observed, start), seconds)| {
                (
                    observed,
                    by_start.get(&start).copied().unwrap_or(false),
                    seconds,
                )
            })
            .collect())
    }

    pub fn rotom_talk_at(&self, frame: i64) -> Option<bool> {
        let seed = self.search_config().ok()?.seed;
        let position = frame.saturating_add(self.pre_rotom_consumption_value());
        let roll = self.cached_rotom_roll_at(seed, position)?;
        Some(u64::from(roll) < self.rotom_threshold_value())
    }
}
