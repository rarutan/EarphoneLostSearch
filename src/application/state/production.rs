use super::{AppState, MAX_FRAME, MAX_PRODUCTION_LOOKAHEAD};
use super::{PreparedProduction, ProductionPreparationRequest};
use crate::application::production::ProductionQuery;
use crate::application::state_types::{
    NormalTargetWaitKey, NormalTargetWaitRequest, SearchConfig, TimelineCandidate,
};
use crate::domain::rng::{
    advance_timeline_from_values, timeline_target_from_cache, ObservationCache, ObservationMatch,
    ProductionTimeline, ProductionTimelineCache, PRODUCTION_TIMELINE_CACHE_FRAMES,
};
use crate::errors::{AppError, ArithmeticOperation, CacheError, StateIssue};

impl AppState {
    /// Target確定後に必要なSFMT範囲を計算し、本番Timelineを準備する要求を作る。
    pub(crate) fn production_preparation_request(
        &self,
    ) -> Option<(ProductionPreparationRequest, ObservationCache)> {
        if self.target_dirty
            || self.encounter_offset_error().is_some()
            || self.candidates.get(self.selected).is_none()
        {
            return None;
        }
        let config = self.search_config().ok()?;
        let observed_position = self.selected_candidate_position()?;
        if config.target < observed_position
            || config.target.saturating_sub(observed_position) > MAX_PRODUCTION_LOOKAHEAD
        {
            return None;
        }
        let query = ProductionQuery {
            seed: config.seed,
            target: config.target,
            models: config.npc_models,
            offset: self.encounter_offset_value(),
            consider_rotom: self.consider_rotom_talk,
            rotom_threshold: self.rotom_threshold_value(),
            observed_rotom_talk: self.observed_rotom_talk,
            pre_rotom_consumption: self.pre_rotom_consumption_value(),
            npc_initial_load: self.npc_initial_load_value(),
            fastest_ticks: self
                .fastest_close
                .then(|| self.encounter_offset_ticks())
                .flatten(),
        };
        if self.fastest_close && query.fastest_ticks.is_none() {
            return None;
        }
        let plan = self.plan_for_encounter(
            config.seed,
            observed_position,
            query.offset,
            query.consider_rotom,
            query.rotom_threshold,
            query.observed_rotom_talk,
        )?;
        let cache = self
            .observation_cache
            .as_ref()
            .filter(|cache| cache.seed == config.seed)?
            .clone();
        Some((
            ProductionPreparationRequest {
                session_generation: self.guidance.session_generation(),
                selected_candidate: self.selected,
                observed_position,
                encounter_start: plan.encounter_start,
                query,
            },
            cache,
        ))
    }

    /// ワーカー上でSFMTキャッシュを必要位置まで延長し、本番Timelineを計算する。
    pub(crate) fn calculate_production_preparation(
        request: ProductionPreparationRequest,
        cache: ObservationCache,
    ) -> Result<PreparedProduction, AppError> {
        let query = request.query;
        if cache.seed != query.seed {
            return Err(AppError::Cache(CacheError::SeedMismatch));
        }
        let base_start = request.encounter_start;
        let mut start = base_start;
        if let Some(ticks) = query.fastest_ticks {
            let advance_values = ticks
                .max(0)
                .saturating_mul(query.models as i64)
                .saturating_add(query.models as i64 + 1);
            let required_end = base_start.saturating_add(advance_values).min(MAX_FRAME);
            cache.ensure_through(required_end, None)?;
            let count = usize::try_from(required_end.saturating_sub(base_start) + 1)
                .map_err(|_| AppError::ArithmeticOverflow(ArithmeticOperation::TimelineLength))?;
            let values = cache.read_values(base_start, count)?;
            start =
                advance_timeline_from_values(&values, base_start, base_start, query.models, ticks)
                    .ok_or(AppError::InvalidState(StateIssue::TimelineUnavailable))?;
        }

        let timeline_values = PRODUCTION_TIMELINE_CACHE_FRAMES
            .saturating_add(query.models as i64)
            .saturating_add(1);
        // Timeline用の固定窓を要求する。ObservationCache側が要求位置の先に
        // 1チャンクを維持し、末尾1チャンクへ入った時点で次を追記する。
        let required_end = start
            .saturating_add(timeline_values)
            .saturating_sub(1)
            .min(MAX_FRAME);
        cache.ensure_through(required_end, None)?;

        let available = cache
            .available_end()
            .saturating_sub(start)
            .saturating_add(1);
        let count = usize::try_from(available.min(timeline_values).max(0))
            .map_err(|_| AppError::ArithmeticOverflow(ArithmeticOperation::TimelineLength))?;
        let values = cache.read_values(start, count)?;
        let mut timeline = ProductionTimelineCache::new_from_values(
            query.seed,
            start,
            query.models,
            PRODUCTION_TIMELINE_CACHE_FRAMES,
            values,
        )
        .ok_or(AppError::InvalidState(StateIssue::TimelineUnavailable))?;
        timeline.extend_until(query.target);
        let target_tick = timeline
            .target_from(start, query.target)
            .and_then(|timing| timing.exact_tick);
        Ok(PreparedProduction {
            cache: timeline,
            production: ProductionTimeline {
                target: query.target,
                target_tick,
            },
        })
    }

    /// 入力条件がワーカー要求と同じ場合だけ、計算済みTimelineを状態へ反映する。
    pub(crate) fn apply_production_preparation(
        &mut self,
        request: &ProductionPreparationRequest,
        prepared: PreparedProduction,
    ) -> bool {
        let matches = self
            .production_preparation_request()
            .is_some_and(|(current, _)| current == *request);
        if !matches {
            return false;
        }
        self.production_cache = Some(prepared.cache);
        self.production = Some(prepared.production);
        for (index, candidate) in self.candidates.iter_mut().enumerate() {
            candidate.target_reachable =
                index == self.selected && prepared.production.target_tick.is_some();
        }
        self.production_preparation_pending = false;
        if std::mem::take(&mut self.encounter_start_requested) {
            self.begin_encounter();
        }
        true
    }

    pub(crate) fn mark_production_preparation_pending(&mut self) {
        self.clear_guidance_reachability();
        self.production = None;
        self.production_preparation_pending = true;
        self.encounter_start_requested = false;
        for candidate in &mut self.candidates {
            candidate.target_reachable = false;
        }
    }

    pub(crate) fn mark_production_preparation_pending_preserving_guidance(&mut self) {
        self.production = None;
        self.production_preparation_pending = true;
        for candidate in &mut self.candidates {
            candidate.target_reachable = false;
        }
    }

    pub(crate) fn finish_production_preparation_without_result(&mut self) {
        self.production_preparation_pending = false;
        self.encounter_start_requested = false;
        self.production = None;
        for candidate in &mut self.candidates {
            candidate.target_reachable = false;
        }
    }

    pub(super) fn normal_target_wait_query(&self, position: i64) -> Option<(SearchConfig, i64)> {
        let Ok(config) = self.search_config() else {
            return None;
        };
        if config.target < position
            || config.target.saturating_sub(position) > MAX_PRODUCTION_LOOKAHEAD
        {
            return None;
        }
        let Some(start) = self.plan_for_encounter(
            config.seed,
            position,
            self.encounter_offset_value(),
            self.consider_rotom_talk,
            self.rotom_threshold_value(),
            self.observed_rotom_talk,
        ) else {
            return None;
        };
        Some((config, start.encounter_start))
    }

    pub(super) fn normal_target_wait_key(&self, position: i64) -> Option<NormalTargetWaitKey> {
        let (config, encounter_start) = self.normal_target_wait_query(position)?;
        Some(NormalTargetWaitKey {
            session_generation: self.guidance.session_generation(),
            seed: config.seed,
            position,
            encounter_start,
            target: config.target,
            models: config.npc_models,
        })
    }

    /// 通常待機でTargetを正確に踏めない場合も、TimelineがTargetを通過する
    /// ステップを目安として返す。cache missの正確な計算結果はworker結果だけを読む。
    pub(super) fn normal_target_wait_ticks(&self, position: i64) -> Option<Option<i64>> {
        let Some(key) = self.normal_target_wait_key(position) else {
            return Some(None);
        };
        let cached_timing = self
            .production_cache
            .as_ref()
            .filter(|cache| cache.seed == key.seed && cache.models == key.models)
            .and_then(|cache| cache.target_from(key.encounter_start, key.target));
        if let Some(timing) = cached_timing {
            return Some(Some(timing.timeline_tick));
        }
        self.normal_target_wait_cache
            .iter()
            .find_map(|(cached_key, result)| (*cached_key == key).then_some(*result))
    }

    /// 現在の表示または次の○に必要なcache missを一件だけworkerへ渡す。
    pub fn normal_target_wait_fallback_request(
        &self,
        current: i64,
        next_circle: Option<i64>,
    ) -> Option<NormalTargetWaitRequest> {
        for position in [Some(current), next_circle].into_iter().flatten() {
            let Some(key) = self.normal_target_wait_key(position) else {
                continue;
            };
            let production_hit = self
                .production_cache
                .as_ref()
                .filter(|cache| cache.seed == key.seed && cache.models == key.models)
                .and_then(|cache| cache.target_from(key.encounter_start, key.target))
                .is_some();
            let result_cached = self
                .normal_target_wait_cache
                .iter()
                .any(|(cached_key, _)| *cached_key == key);
            if !production_hit && !result_cached {
                return Some(NormalTargetWaitRequest { key });
            }
        }
        None
    }

    pub fn apply_normal_target_wait_fallback(
        &mut self,
        request: NormalTargetWaitRequest,
        result: Option<i64>,
    ) -> bool {
        let key = request.key;
        if !self.normal_target_wait_fallback_is_current(request) {
            return false;
        }
        self.normal_target_wait_cache
            .retain(|(cached_key, _)| *cached_key != key);
        self.normal_target_wait_cache.push_back((key, result));
        while self.normal_target_wait_cache.len() > 32 {
            self.normal_target_wait_cache.pop_front();
        }
        true
    }

    pub fn normal_target_wait_fallback_is_current(&self, request: NormalTargetWaitRequest) -> bool {
        let key = request.key;
        key.session_generation == self.guidance.session_generation()
            && self.normal_target_wait_key(key.position) == Some(key)
            && self.normal_target_wait_result_is_relevant(key.position)
    }

    pub(super) fn normal_target_wait_result_is_relevant(&self, position: i64) -> bool {
        let Some(current) = self.timeline_display_position() else {
            return false;
        };
        if position == current {
            return true;
        }
        self.guidance
            .next_reachable(current, self.guidance.known_end(current, MAX_FRAME))
            == Some(position)
    }

    pub fn calculate_normal_target_wait_fallback(
        request: NormalTargetWaitRequest,
        cache: &ObservationCache,
    ) -> Result<Option<i64>, AppError> {
        if cache.seed != request.key.seed {
            return Err(AppError::Cache(CacheError::SeedMismatch));
        }
        Ok(timeline_target_from_cache(
            cache,
            request.key.encounter_start,
            request.key.target,
            request.key.models,
        )?
        .map(|timing| timing.timeline_tick))
    }

    pub(super) fn make_candidate(&self, matched: ObservationMatch) -> TimelineCandidate {
        TimelineCandidate {
            start_position: matched.start_position,
            sfmt_position: matched.sfmt_position,
            next_blink_ticks: Some(matched.next_blink_ticks),
            // 選択候補の本番到達可否は、候補確定後にWorkerで計算して反映する。
            target_reachable: false,
        }
    }

    pub(super) fn refresh_production(&mut self) {
        if self.encounter_offset_error().is_some() {
            self.production_preparation_pending = false;
            for candidate in &mut self.candidates {
                candidate.target_reachable = false;
            }
            self.production = None;
            return;
        }
        self.mark_production_preparation_pending();
    }

    /// 現在位置だけが進んだ場合は、本番Timelineを更新しつつ、絶対位置ごとの
    /// 到達判定キャッシュは維持する。設定変更時の通常更新とは分ける。
    pub(super) fn refresh_production_preserving_guidance(&mut self) {
        if self.encounter_offset_error().is_some() {
            self.production_preparation_pending = false;
            self.encounter_start_requested = false;
            for candidate in &mut self.candidates {
                candidate.target_reachable = false;
            }
            self.production = None;
            return;
        }
        self.mark_production_preparation_pending_preserving_guidance();
    }
}
