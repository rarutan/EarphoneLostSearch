use crate::application::field::{
    FieldIdxInference, FieldSearchPool, FieldState, PreparedTimelineIndex,
};
use crate::application::state::CandidateReachabilityRequest;
use crate::domain::rng::ObservationCache;
use crate::ui::app_types::{
    CandidateReachabilityOutput, FieldSearchOutput, GuidanceGenerationOutput,
};
use std::sync::{atomic::AtomicBool, Arc};
use std::thread::JoinHandle;

/// UIから起動する検索・Timeline関連ワーカーをまとめて保持する。
///
/// ジョブの開始・キャンセル・完了結果の適用は`UiApp`が調停するが、個々の
/// ハンドルを画面状態と同列に並べないことで、検索ライフサイクルの境界を明確にする。
#[derive(Default)]
pub(super) struct SearchJobs {
    pub(super) generation: Option<JoinHandle<Result<ObservationCache, String>>>,
    pub(super) correction_generation: Option<JoinHandle<Result<ObservationCache, String>>>,
    pub(super) correction_generation_key: Option<(u32, i64, i64, u64)>,
    pub(super) field_generation: Option<JoinHandle<Result<FieldSearchOutput, String>>>,
    pub(super) field_generation_cancel: Option<Arc<AtomicBool>>,
    pub(super) field_pool_generation: Option<JoinHandle<Result<FieldSearchPool, String>>>,
    pub(super) field_pool_generation_cancel: Option<Arc<AtomicBool>>,
    pub(super) field_timeline_generation: Option<JoinHandle<Result<PreparedTimelineIndex, String>>>,
    pub(super) field_timeline_generation_cancel: Option<Arc<AtomicBool>>,
    pub(super) field_timeline_extension: Option<JoinHandle<(FieldState, bool)>>,
    pub(super) field_timeline_extension_key: Option<(usize, i64, i64)>,
    pub(super) field_timeline_extension_blocked: bool,
    pub(super) field_pool: Option<FieldSearchPool>,
    pub(super) field_idx_inference: Option<JoinHandle<Result<FieldIdxInference, String>>>,
    pub(super) field_idx_inference_cancel: Option<Arc<AtomicBool>>,
    pub(super) field_search_needs_restart: bool,
    pub(super) guidance_generation: Option<JoinHandle<GuidanceGenerationOutput>>,
    pub(super) candidate_reachability: Option<JoinHandle<CandidateReachabilityOutput>>,
    pub(super) candidate_reachability_key: Option<CandidateReachabilityRequest>,
}
