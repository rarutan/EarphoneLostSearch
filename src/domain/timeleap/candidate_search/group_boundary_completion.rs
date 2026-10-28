use super::{Frame1CandidateRange, TimeleapCandidate};

pub(super) struct BoundaryCandidates {
    pub(super) candidates: Vec<TimeleapCandidate>,
    /// Number of unique Frame1 origins retained from this connected edge run.
    pub(super) connected_ticks: u64,
    /// True only when every tick in this chunk is part of the connected run.
    pub(super) reaches_chunk_edge: bool,
}

pub(super) fn take_contiguous_prefix(
    candidates: Vec<TimeleapCandidate>,
    chunk: Frame1CandidateRange,
) -> BoundaryCandidates {
    let Some((connected_start, connected_end)) = connected_edge(&candidates, chunk, true) else {
        return BoundaryCandidates {
            candidates: Vec::new(),
            connected_ticks: 0,
            reaches_chunk_edge: false,
        };
    };
    BoundaryCandidates {
        candidates: candidates
            .into_iter()
            .filter(|candidate| (connected_start..connected_end).contains(&candidate.frame1_tick))
            .collect(),
        connected_ticks: connected_end.saturating_sub(connected_start) as u64,
        reaches_chunk_edge: connected_end == chunk.end_tick,
    }
}

pub(super) fn take_contiguous_suffix(
    candidates: Vec<TimeleapCandidate>,
    chunk: Frame1CandidateRange,
) -> BoundaryCandidates {
    let Some((connected_start, connected_end)) = connected_edge(&candidates, chunk, false) else {
        return BoundaryCandidates {
            candidates: Vec::new(),
            connected_ticks: 0,
            reaches_chunk_edge: false,
        };
    };
    BoundaryCandidates {
        candidates: candidates
            .into_iter()
            .filter(|candidate| (connected_start..connected_end).contains(&candidate.frame1_tick))
            .collect(),
        connected_ticks: connected_end.saturating_sub(connected_start) as u64,
        reaches_chunk_edge: connected_start == chunk.start_tick,
    }
}

fn connected_edge(
    candidates: &[TimeleapCandidate],
    chunk: Frame1CandidateRange,
    from_start: bool,
) -> Option<(i64, i64)> {
    if chunk.is_empty() {
        return None;
    }
    let mut ticks = candidates
        .iter()
        .map(|candidate| candidate.frame1_tick)
        .filter(|tick| chunk.contains(*tick))
        .collect::<Vec<_>>();
    ticks.sort_unstable();
    ticks.dedup();

    if from_start {
        if ticks.first().copied() != Some(chunk.start_tick) {
            return None;
        }
        let mut end_tick = chunk.start_tick.saturating_add(1);
        for tick in ticks.into_iter().skip(1) {
            if tick != end_tick {
                break;
            }
            end_tick = end_tick.saturating_add(1);
        }
        Some((chunk.start_tick, end_tick))
    } else {
        let expected_last = chunk.end_tick.saturating_sub(1);
        if ticks.last().copied() != Some(expected_last) {
            return None;
        }
        let mut start_tick = expected_last;
        for tick in ticks.into_iter().rev().skip(1) {
            if tick != start_tick.saturating_sub(1) {
                break;
            }
            start_tick = start_tick.saturating_sub(1);
        }
        Some((start_tick, chunk.end_tick))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::timeleap::candidate_search::{TimeleapStatus, TimeleapTargetInput};
    use std::sync::Arc;

    fn candidate(tick: i64, target_frame: i64) -> TimeleapCandidate {
        let status = TimeleapStatus::new(1);
        TimeleapCandidate {
            frame1: 1_000 + tick * 2,
            frame1_tick: tick,
            post_tick: tick + 20,
            grace_frames: 0,
            pre_frame: 1_000 + tick * 2,
            pre_sfmt_position: tick,
            pre_status: status.clone(),
            post_frame: target_frame,
            post_sfmt_position: target_frame,
            post_status: status,
            target_inputs: Arc::from([TimeleapTargetInput {
                frame: target_frame,
                tick: tick + 20,
            }]),
        }
    }

    #[test]
    fn boundary_completion_keeps_only_the_connected_prefix_and_preserves_duplicates() {
        let chunk = Frame1CandidateRange::new(5, 10);
        let boundary = take_contiguous_prefix(
            vec![
                candidate(5, 100),
                candidate(6, 100),
                candidate(6, 101),
                candidate(8, 100),
                candidate(9, 100),
            ],
            chunk,
        );

        assert!(!boundary.reaches_chunk_edge);
        assert_eq!(boundary.connected_ticks, 2);
        assert_eq!(
            boundary
                .candidates
                .iter()
                .map(|candidate| candidate.frame1_tick)
                .collect::<Vec<_>>(),
            [5, 6, 6]
        );
    }

    #[test]
    fn boundary_completion_keeps_only_the_connected_suffix_and_detects_chunk_continuation() {
        let chunk = Frame1CandidateRange::new(2, 7);
        let boundary = take_contiguous_suffix(
            vec![
                candidate(2, 100),
                candidate(4, 100),
                candidate(5, 100),
                candidate(6, 100),
                candidate(6, 101),
            ],
            chunk,
        );

        assert!(!boundary.reaches_chunk_edge);
        assert_eq!(boundary.connected_ticks, 3);
        assert_eq!(
            boundary
                .candidates
                .iter()
                .map(|candidate| candidate.frame1_tick)
                .collect::<Vec<_>>(),
            [4, 5, 6, 6]
        );

        let full = take_contiguous_suffix((2..7).map(|tick| candidate(tick, 200)).collect(), chunk);
        assert!(full.reaches_chunk_edge);
        assert_eq!(full.connected_ticks, 5);
        assert_eq!(full.candidates.len(), 5);
    }
}
