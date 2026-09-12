use crate::application::field::{FieldSearchOutcome, FieldSearchPool};
use crate::application::state::CorrectionPreview;

pub(super) type GuidanceGenerationOutput = (
    crate::application::state::GuidanceSearchRequest,
    Vec<(i64, bool)>,
);
pub(super) type CandidateReachabilityOutput = (
    crate::application::state::CandidateReachabilityRequest,
    Vec<(usize, bool)>,
);
pub(super) type FieldSearchOutput = (FieldSearchOutcome, FieldSearchPool);

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
    pub(super) status: String,
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
