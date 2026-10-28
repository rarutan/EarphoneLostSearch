use crate::application::field::{FieldResult, FieldSearchOutcome, FieldSearchPool};
use crate::application::state::CorrectionPreview;
use crate::errors::AppError;

pub(super) type GuidanceGenerationOutput = (
    crate::application::state::GuidanceSearchRequest,
    Result<Vec<(i64, bool, f64)>, AppError>,
);
pub(super) type NormalTargetWaitOutput = (
    crate::application::state::NormalTargetWaitRequest,
    Result<Option<i64>, AppError>,
);
pub(super) type CandidateReachabilityOutput = (
    crate::application::state::CandidateReachabilityRequest,
    Result<Vec<(usize, bool)>, AppError>,
);
pub(super) type ProductionPreparationOutput = (
    crate::application::state::ProductionPreparationRequest,
    Result<crate::application::state::PreparedProduction, AppError>,
);
pub(super) type FieldSearchOutput = (FieldSearchOutcome, FieldSearchPool);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FieldTargetKey {
    pub(super) seed: u32,
    pub(super) target: i64,
    pub(super) start_consumption: i64,
    pub(super) first_wait_frame: i64,
    pub(super) last_wait_frame: i64,
    pub(super) models: usize,
    pub(super) observed_model: usize,
    pub(super) target_consumptions: Vec<i64>,
}

impl FieldTargetKey {
    pub(super) fn for_result(seed: u32, target: i64, result: &FieldResult) -> Self {
        Self {
            seed,
            target,
            start_consumption: result.start_consumption,
            first_wait_frame: result.first_wait_frame,
            last_wait_frame: result.last_wait_frame,
            models: result.models,
            observed_model: result.observed_model,
            target_consumptions: result.target_consumptions.clone(),
        }
    }

    pub(super) fn matches_result(&self, seed: u32, target: i64, result: &FieldResult) -> bool {
        *self == Self::for_result(seed, target, result)
    }
}

pub(super) type FieldTargetOutput = (FieldTargetKey, Result<FieldResult, AppError>);
pub(super) struct PreparedTimeleapTarget {
    pub(super) seed: u32,
    pub(super) target: i64,
    pub(super) candidate: crate::domain::timeleap::TimeleapCandidate,
    pub(super) result: FieldResult,
}
pub(super) type PreparedTimeleapTargetOutput = Result<PreparedTimeleapTarget, AppError>;
pub(super) type TimeleapCandidateOutput = (
    crate::domain::timeleap::TimeleapSearchKey,
    Result<
        Vec<crate::domain::timeleap::TimeleapCandidateGroup>,
        crate::domain::timeleap::TimeleapSearchFailure,
    >,
);

pub(super) type TimeleapFrame1Output = Result<
    crate::domain::timeleap::TimeleapFrame1Preparation,
    crate::domain::timeleap::TimeleapSearchFailure,
>;
pub(super) enum TimeleapFrame1PreviewCompletion {
    Calculated(TimeleapFrame1Output),
    Panicked,
}
pub(super) type TimeleapFrame1PreviewOutput = (
    u64,
    crate::domain::timeleap::TimeleapConfig,
    i64,
    crate::domain::timeleap::TimeleapTimelineStart,
    std::time::Instant,
    TimeleapFrame1PreviewCompletion,
);
pub(super) enum TimeleapTargetCheckCompletion {
    Calculated(Result<bool, crate::domain::timeleap::TimeleapSearchFailure>),
    Panicked,
}

pub(super) type TimeleapTargetCheckOutput = (u64, i64, TimeleapTargetCheckCompletion);

fn catch_worker_panic<T>(work: impl FnOnce() -> T) -> Result<T, ()> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).map_err(|_| ())
}

pub(super) fn run_timeleap_frame1_preview(
    generation: u64,
    prepare: impl FnOnce() -> TimeleapFrame1Output,
) -> (u64, TimeleapFrame1PreviewCompletion) {
    let completion = match catch_worker_panic(prepare) {
        Ok(result) => TimeleapFrame1PreviewCompletion::Calculated(result),
        Err(()) => TimeleapFrame1PreviewCompletion::Panicked,
    };
    (generation, completion)
}

pub(super) fn run_timeleap_target_check(
    generation: u64,
    target_frame: i64,
    check: impl FnOnce() -> Result<bool, crate::domain::timeleap::TimeleapSearchFailure>,
) -> TimeleapTargetCheckOutput {
    let completion = match catch_worker_panic(check) {
        Ok(result) => TimeleapTargetCheckCompletion::Calculated(result),
        Err(()) => TimeleapTargetCheckCompletion::Panicked,
    };
    (generation, target_frame, completion)
}
pub(super) type TimeleapReachabilityOutput = (
    u64,
    Result<
        crate::domain::timeleap::TimeleapReachability,
        crate::domain::timeleap::TimeleapSearchFailure,
    >,
);
pub(super) type ConfirmedTargetSearchOutput = (
    u64,
    crate::domain::timeleap::ConfirmedTargetSearchRequest,
    Result<
        crate::domain::timeleap::ConfirmedTargetSearchResult,
        crate::domain::timeleap::TimeleapSearchFailure,
    >,
);
pub(super) type TimeleapShiftResolutionOutput = (
    u64,
    Option<crate::application::field::TimeleapShiftResolutionSummary>,
);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AppTab {
    Hatch,
    Field,
    NormalTimer,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum OffsetScope {
    Hatch,
    Field,
    Normal,
}

pub(super) struct OffsetManagerState {
    pub(super) scope: OffsetScope,
    pub(super) value: String,
    pub(super) memo: String,
    pub(super) editing: Option<usize>,
    pub(super) notice: String,
}

pub(super) enum OffsetAction {
    Apply(i64),
    Edit(usize),
    Delete(usize),
}

pub(super) enum CorrectionDialog {
    Confirm {
        preview: CorrectionPreview,
    },
    ConfirmFieldCorrection {
        indices: Vec<usize>,
        unknown: bool,
        suggested_offset: Option<i64>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_check_panic_retains_request_identity() {
        let (generation, target, completion) =
            run_timeleap_target_check(17, 42_000, || panic!("simulated Target check panic"));

        assert_eq!(generation, 17);
        assert_eq!(target, 42_000);
        assert!(matches!(
            completion,
            TimeleapTargetCheckCompletion::Panicked
        ));
    }

    #[test]
    fn frame1_preview_panic_retains_request_generation() {
        let (generation, completion) =
            run_timeleap_frame1_preview(23, || panic!("simulated Frame1 preview panic"));

        assert_eq!(generation, 23);
        assert!(matches!(
            completion,
            TimeleapFrame1PreviewCompletion::Panicked
        ));
    }
}
