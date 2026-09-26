use crate::application::field::{FieldResult, FieldSearchOutcome, FieldSearchPool};
use crate::application::state::CorrectionPreview;
use crate::errors::AppError;

pub(super) type GuidanceGenerationOutput = (
    crate::application::state::GuidanceSearchRequest,
    Result<Vec<(i64, bool)>, AppError>,
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
