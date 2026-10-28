//! Resolves the user-selected Frame1 origin window on the ordinary Timeline.
//!
//! This module deliberately accepts only an origin window and ordinary
//! Timeline state. Consider Delay / No Dex configuration cannot affect which
//! Frame1 origins belong to the core search window.

use super::super::runtime::{advance_one_tick_without_blink_list, apply_event_with_profile};
use super::super::{CourierGenerationProfile, TimeleapMode, MAX_NORMAL_TIMELINE_TICKS};
use super::frame1_range::Frame1CandidateRange;
use super::{is_cancelled, Cursor, TimeleapStatus};
use crate::domain::rng::Sfmt;
use std::sync::atomic::AtomicBool;

const BOUNDARY_CHECKPOINT_INTERVAL: i64 = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Frame1OriginWindow {
    ElapsedTicks { min_tick: i64, max_tick: i64 },
    AbsoluteFrames { min_frame: i64, max_frame: i64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Frame1OriginQuery {
    pub(super) target_frame: i64,
    pub(super) window: Frame1OriginWindow,
}

#[derive(Clone)]
pub(super) struct CandidateTimelineCheckpoint {
    pub(super) cursor: Cursor,
    pub(super) status: TimeleapStatus,
}

pub(super) struct ResolvedFrame1OriginScope {
    pub(super) range: Frame1CandidateRange,
    pub(super) core_start: CandidateTimelineCheckpoint,
    pub(super) core_end: CandidateTimelineCheckpoint,
    pub(super) lower_checkpoints: Vec<CandidateTimelineCheckpoint>,
    /// Ordinary Timeline ticks advanced to position the requested window.
    pub(super) scope_resolution_ticks: u64,
}

/// One ordinary-Timeline tick origin together with its Timeleap outcome.
pub(super) struct ConsiderDelayCandidateInput {
    pub(super) frame1: i64,
    pub(super) frame1_tick: i64,
    pub(super) pre_frame: i64,
    pub(super) pre_sfmt_position: i64,
    pub(super) pre_status: TimeleapStatus,
    pub(super) post_event_status: TimeleapStatus,
    pub(super) post_event_cursor: Cursor,
}

/// Enumerates Frame1 origins and snapshots the Timeleap transition without
/// accepting any Consider Delay or No Dex options.
pub(super) struct CandidateSnapshotSource {
    pub(super) cursor: Cursor,
    pub(super) status: TimeleapStatus,
    pub(super) sfmt: Sfmt,
    pub(super) start_tick: i64,
    pub(super) end_tick: i64,
    pub(super) target_frame: i64,
    pub(super) target_reached: bool,
    pub(super) mode: TimeleapMode,
    pub(super) courier: CourierGenerationProfile,
}

impl CandidateSnapshotSource {
    pub(super) fn next(
        &mut self,
        cancel: Option<&AtomicBool>,
    ) -> Result<Option<ConsiderDelayCandidateInput>, String> {
        if self.cursor.frame >= self.target_frame {
            self.target_reached = true;
            return Ok(None);
        }
        while self.cursor.tick < self.end_tick {
            if is_cancelled(cancel) {
                return Err("Timeleap候補検索がキャンセルされました。".into());
            }
            if self.cursor.frame >= self.target_frame {
                self.target_reached = true;
                return Ok(None);
            }
            let candidate = if self.cursor.tick >= self.start_tick {
                let mut post_event_status = self.status.clone();
                let mut post_event_cursor = self.cursor;
                let mut post_event_sfmt = self.sfmt.clone();
                let pre_status = post_event_status.clone();
                let pre_frame = post_event_cursor.frame;
                let pre_sfmt_position = post_event_cursor.sfmt_position;
                apply_event_with_profile(
                    &mut post_event_status,
                    &mut post_event_cursor,
                    &mut post_event_sfmt,
                    self.mode,
                    &self.courier,
                );
                Some(ConsiderDelayCandidateInput {
                    frame1: self.cursor.frame,
                    frame1_tick: self.cursor.tick,
                    pre_frame,
                    pre_sfmt_position,
                    pre_status,
                    post_event_status,
                    post_event_cursor,
                })
            } else {
                None
            };
            advance_one_tick_without_blink_list(&mut self.status, &mut self.cursor, &mut self.sfmt);
            if self.cursor.frame > self.target_frame {
                self.target_reached = true;
                return Ok(None);
            }
            if self.cursor.frame == self.target_frame {
                self.target_reached = true;
            }
            if candidate.is_some() {
                return Ok(candidate);
            }
        }
        Ok(None)
    }
}

pub(super) fn resolve_frame1_origin_scope(
    query: Frame1OriginQuery,
    mut cursor: Cursor,
    mut status: TimeleapStatus,
    mut sfmt: Sfmt,
    cancel: Option<&AtomicBool>,
) -> Result<ResolvedFrame1OriginScope, String> {
    cursor.tick = 0;
    let mut lower_checkpoints = Vec::new();
    remember_lower_checkpoint(&mut lower_checkpoints, cursor, &status);

    let (start_tick, end_tick, core_start, core_end) = match query.window {
        Frame1OriginWindow::AbsoluteFrames {
            min_frame,
            max_frame,
        } => {
            while cursor.frame < min_frame
                && cursor.frame < query.target_frame
                && cursor.tick < MAX_NORMAL_TIMELINE_TICKS as i64
            {
                check_cancelled(cancel)?;
                remember_lower_checkpoint(&mut lower_checkpoints, cursor, &status);
                advance_one_tick_without_blink_list(&mut status, &mut cursor, &mut sfmt);
            }
            let start_tick = cursor.tick;
            let core_start = checkpoint(cursor, &status);
            if cursor.frame < min_frame
                || cursor.frame > max_frame
                || cursor.frame >= query.target_frame
            {
                (start_tick, start_tick, core_start.clone(), core_start)
            } else {
                while cursor.frame <= max_frame
                    && cursor.frame < query.target_frame
                    && cursor.tick < MAX_NORMAL_TIMELINE_TICKS as i64
                {
                    check_cancelled(cancel)?;
                    advance_one_tick_without_blink_list(&mut status, &mut cursor, &mut sfmt);
                }
                let end_tick = cursor.tick;
                let core_end = checkpoint(cursor, &status);
                (start_tick, end_tick, core_start, core_end)
            }
        }
        Frame1OriginWindow::ElapsedTicks { min_tick, max_tick } => {
            let start_tick = min_tick.max(0).min(MAX_NORMAL_TIMELINE_TICKS as i64);
            let end_tick = max_tick
                .saturating_add(1)
                .min(MAX_NORMAL_TIMELINE_TICKS as i64);
            while cursor.tick < start_tick && cursor.frame < query.target_frame {
                check_cancelled(cancel)?;
                remember_lower_checkpoint(&mut lower_checkpoints, cursor, &status);
                advance_one_tick_without_blink_list(&mut status, &mut cursor, &mut sfmt);
            }
            let actual_start_tick = cursor.tick;
            let core_start = checkpoint(cursor, &status);
            if cursor.tick < start_tick || cursor.frame >= query.target_frame {
                (
                    actual_start_tick,
                    actual_start_tick,
                    core_start.clone(),
                    core_start,
                )
            } else {
                while cursor.tick < end_tick && cursor.frame < query.target_frame {
                    check_cancelled(cancel)?;
                    advance_one_tick_without_blink_list(&mut status, &mut cursor, &mut sfmt);
                }
                let core_end = checkpoint(cursor, &status);
                (
                    actual_start_tick,
                    core_end.cursor.tick.min(end_tick),
                    core_start,
                    core_end,
                )
            }
        }
    };

    Ok(ResolvedFrame1OriginScope {
        range: Frame1CandidateRange::new(start_tick, end_tick),
        core_start,
        core_end,
        lower_checkpoints,
        scope_resolution_ticks: start_tick.max(0) as u64,
    })
}

fn checkpoint(cursor: Cursor, status: &TimeleapStatus) -> CandidateTimelineCheckpoint {
    CandidateTimelineCheckpoint {
        cursor,
        status: status.clone(),
    }
}

fn remember_lower_checkpoint(
    checkpoints: &mut Vec<CandidateTimelineCheckpoint>,
    cursor: Cursor,
    status: &TimeleapStatus,
) {
    if cursor.tick % BOUNDARY_CHECKPOINT_INTERVAL == 0
        && checkpoints
            .last()
            .is_none_or(|checkpoint| checkpoint.cursor.tick != cursor.tick)
    {
        checkpoints.push(checkpoint(cursor, status));
    }
}

fn check_cancelled(cancel: Option<&AtomicBool>) -> Result<(), String> {
    if is_cancelled(cancel) {
        Err("Timeleap候補検索がキャンセルされました。".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::rng::Sfmt;
    use crate::domain::timeleap::candidate_search::{Cursor, TimeleapStatus};

    #[test]
    fn resolves_only_the_requested_core_origin_window() {
        let cursor = Cursor {
            frame: 1_000,
            sfmt_position: 1_000,
            tick: 91,
        };
        let scope = resolve_frame1_origin_scope(
            Frame1OriginQuery {
                target_frame: 1_100,
                window: Frame1OriginWindow::ElapsedTicks {
                    min_tick: 8,
                    max_tick: 20,
                },
            },
            cursor,
            TimeleapStatus::new(2),
            Sfmt::new(1),
            None,
        )
        .expect("resolve the input window");

        assert_eq!(scope.range, Frame1CandidateRange::new(8, 21));
        assert_eq!(scope.core_start.cursor.tick, 8);
        assert_eq!(scope.core_end.cursor.tick, 21);
        assert_eq!(scope.scope_resolution_ticks, 8);
    }

    #[test]
    fn absolute_frame_window_uses_timeline_frame_not_generation_profile() {
        let initial_cursor = Cursor {
            frame: 1_000,
            sfmt_position: 1_000,
            tick: 0,
        };
        let initial_status = TimeleapStatus::new(2);
        let initial_sfmt = Sfmt::new(1);
        let mut expected_cursor = initial_cursor;
        let mut expected_status = initial_status.clone();
        let mut expected_sfmt = initial_sfmt.clone();
        while expected_cursor.frame < 1_008 {
            advance_one_tick_without_blink_list(
                &mut expected_status,
                &mut expected_cursor,
                &mut expected_sfmt,
            );
        }
        let scope = resolve_frame1_origin_scope(
            Frame1OriginQuery {
                target_frame: 1_100,
                window: Frame1OriginWindow::AbsoluteFrames {
                    min_frame: 1_008,
                    max_frame: 1_020,
                },
            },
            initial_cursor,
            initial_status,
            initial_sfmt,
            None,
        )
        .expect("resolve absolute frame window");

        assert_eq!(scope.core_start.cursor.frame, 1_008);
        assert_eq!(scope.range.start_tick, expected_cursor.tick);
        assert_eq!(
            scope.core_start.cursor.sfmt_position,
            expected_cursor.sfmt_position
        );
        assert!(scope.core_end.cursor.frame > 1_020);
        assert_eq!(scope.range.end_tick, scope.core_end.cursor.tick);
    }
}
