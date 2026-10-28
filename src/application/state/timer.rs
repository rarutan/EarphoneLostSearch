use super::{AppState, MAX_PRODUCTION_LOOKAHEAD};
use crate::application::state_types::{ArticleTimerStage, Mode};
use std::time::{Duration, Instant};

impl AppState {
    // --- Production timer and encounter transition -----------------------

    /// 記事方式のエンカウント操作へ移行する。互換タイマーは呼び出さない。
    pub fn begin_encounter(&mut self) {
        if !self.can_start_timer() {
            return;
        }
        if self.encounter_offset_error().is_some() {
            return;
        }
        if self.candidates.get(self.selected).is_none() {
            return;
        }
        let Some(config) = self.search_config().ok() else {
            return;
        };
        if config
            .target
            .saturating_sub(self.selected_candidate_position().unwrap_or(config.target))
            > MAX_PRODUCTION_LOOKAHEAD
        {
            return;
        }
        let cache_matches_seed = self
            .observation_cache
            .as_ref()
            .is_some_and(|cache| cache.seed == config.seed);
        if !cache_matches_seed {
            return;
        }
        self.initialize_observed_rotom_talk();
        if self.production_preparation_pending {
            self.encounter_start_requested = true;
            return;
        }
        let Some(production) = self.production else {
            self.mark_production_preparation_pending();
            self.encounter_start_requested = true;
            return;
        };
        if production.target_tick.is_none() {
            return;
        }
        let Some(_plan) = self.article_plan_for_selected() else {
            return;
        };

        self.production = Some(production);
        self.encounter_start_requested = false;
        // 補正はSet後の表示値ではなく、本番へ移行した時点の値を基準にする。
        self.correction_source_offset = Some(self.encounter_offset_value());
        // Readyを押した時点から区間を最初から数え直さず、Shift観測と補正から
        // すでに予約済みの次回瞬きまでの「残り時間」を引き継ぐ。
        if self.fastest_close {
            // 最速閉じは○までの案内だけで完了する。○表示後にアプリ側で
            // Enterを受けたりTargetタイマーを開始したりしない。
            return;
        }

        // 追加待機方式では、現在表示が○の時点でEnterと実機Bを同時に押す。
        // B後オフセット中はTimelineを進めない。オフセットとTargetまでの本番待機を
        // 別々に表示し、区切りでbeepを鳴らせるよう二段階で計時する。
        let target_seconds = production
            .target_tick
            .map(|ticks| self.game_ticks_to_seconds(ticks))
            .unwrap_or(0.0);
        self.correction_source_frame = self.timeline_display_position();
        self.correction_source_index = 0;
        self.correction_target_frame = Some(production.target);
        let offset_seconds = self.encounter_offset_seconds();
        self.next_blink = None;
        if self.use_offset {
            self.article_next_seconds = Some(target_seconds);
            self.start_article_timer(ArticleTimerStage::Offset, offset_seconds);
        } else {
            self.article_next_seconds = None;
            self.start_article_timer(ArticleTimerStage::ToTarget, target_seconds);
        }
    }

    /// 最速閉じの○を実際に使用した時点で、瞬き予測を停止して
    /// ずれ検証へ渡せる状態にする。オフセットの有効／無効には依存しない。
    pub fn finish_fastest_close_for_correction(&mut self) -> bool {
        if !self.can_finish_fastest_close_for_correction() {
            return false;
        }
        let Some(current) = self.timeline_display_position() else {
            return false;
        };
        self.initialize_observed_rotom_talk();
        // 最速閉じでは記録済みの○だけをB押下候補として残す。現在位置が×でも、
        // 既に記録した○を補正元へ使い、到達不能な現在位置は候補へ追加しない。
        let frames = self.correction_start_candidates();
        let source_index = frames
            .iter()
            .position(|frame| *frame == current)
            .unwrap_or(0);
        let Some(source_frame) = frames.get(source_index).copied() else {
            return false;
        };
        self.correction_source_frame = Some(source_frame);
        self.correction_source_index = source_index;
        self.correction_target_frame = self.search_config().ok().map(|config| config.target);
        self.correction_source_offset = Some(self.encounter_offset_value());
        self.next_blink = None;
        self.next_blink_index = Some(source_frame);
        self.timer_start = None;
        self.clear_article_timer();
        self.enter_finished();
        true
    }

    pub fn remaining(&self) -> f64 {
        self.remaining_at(Instant::now())
    }

    pub(super) fn remaining_at(&self, now: Instant) -> f64 {
        self.timer_start
            .map(|start| {
                (self.timer_seconds - now.saturating_duration_since(start).as_secs_f64()).max(0.0)
            })
            .unwrap_or(0.0)
    }

    pub fn article_remaining(&self) -> f64 {
        self.article_remaining_at(Instant::now())
    }

    pub(super) fn article_remaining_at(&self, now: Instant) -> f64 {
        self.article_timer_start
            .map(|start| {
                (self.article_timer_seconds - now.saturating_duration_since(start).as_secs_f64())
                    .max(0.0)
            })
            .unwrap_or(0.0)
    }

    /// 記事方式のカウントダウンを進め、段階が切り替わったときにその段階を返す。
    pub fn tick_article_timer(&mut self) -> Option<ArticleTimerStage> {
        self.tick_article_timer_at(Instant::now())
    }

    pub(super) fn tick_article_timer_at(&mut self, now: Instant) -> Option<ArticleTimerStage> {
        if self.mode != Mode::Countdown || self.article_remaining_at(now) > 0.0 {
            return None;
        }
        let stage = self.article_timer_stage?;
        match stage {
            ArticleTimerStage::Offset => {
                let target_seconds = self.article_next_seconds.take().unwrap_or(0.0);
                let next_start = self
                    .article_timer_start
                    .map(|start| start + Duration::from_secs_f64(self.article_timer_seconds))
                    .unwrap_or(now);
                self.article_timer_stage = Some(ArticleTimerStage::ToTarget);
                self.article_timer_seconds = target_seconds;
                self.article_timer_start = Some(next_start);
            }
            ArticleTimerStage::ToTarget => {
                self.enter_finished();
                self.clear_article_timer();
            }
        }
        Some(stage)
    }
}
