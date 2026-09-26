use crate::application::field::{
    FieldIdxInference, FieldSearchPool, FieldState, PreparedTimelineIndex,
};
use crate::application::state::CandidateReachabilityRequest;
use crate::domain::rng::ObservationCache;
use crate::errors::AppError;
use crate::ui::app_types::{
    CandidateReachabilityOutput, FieldSearchOutput, FieldTargetOutput, GuidanceGenerationOutput,
    ProductionPreparationOutput,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::JoinHandle;

/// ワーカーの実行・キャンセル・完了取得を一体で管理する。
///
/// ハンドルとキャンセルトークンを別々のOptionで持つと、片方だけ残る状態を
/// 作りやすいため、同じワーカーのライフサイクルとしてまとめる。
pub(super) struct WorkerTask<T> {
    handle: Option<JoinHandle<T>>,
    cancel: Option<Arc<AtomicBool>>,
}

impl<T> Default for WorkerTask<T> {
    fn default() -> Self {
        Self {
            handle: None,
            cancel: None,
        }
    }
}

impl<T: Send + 'static> WorkerTask<T> {
    pub(super) fn is_running(&self) -> bool {
        self.handle.is_some()
    }

    pub(super) fn is_finished(&self) -> bool {
        self.handle.as_ref().is_some_and(JoinHandle::is_finished)
    }

    #[cfg(test)]
    pub(super) fn thread_id(&self) -> Option<std::thread::ThreadId> {
        self.handle.as_ref().map(|handle| handle.thread().id())
    }

    pub(super) fn spawn_cancellable(
        &mut self,
        job: impl FnOnce(Arc<AtomicBool>) -> T + Send + 'static,
    ) -> bool {
        if self.handle.is_some() {
            return false;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        self.spawn_with_token(cancel.clone(), move || job(worker_cancel));
        true
    }

    pub(super) fn spawn(&mut self, job: impl FnOnce() -> T + Send + 'static) -> bool {
        if self.handle.is_some() {
            return false;
        }
        self.cancel = None;
        self.handle = Some(std::thread::spawn(job));
        true
    }

    pub(super) fn spawn_with_token(
        &mut self,
        cancel: Arc<AtomicBool>,
        job: impl FnOnce() -> T + Send + 'static,
    ) -> bool {
        if self.handle.is_some() {
            return false;
        }
        self.cancel = Some(cancel);
        self.handle = Some(std::thread::spawn(job));
        true
    }

    pub(super) fn cancel(&self) {
        if let Some(token) = self.cancel.as_ref() {
            token.store(true, Ordering::Release);
        }
    }

    pub(super) fn take_finished(&mut self) -> Option<(bool, Result<T, ()>)> {
        if !self.is_finished() {
            return None;
        }
        let cancelled = self
            .cancel
            .take()
            .is_some_and(|token| token.load(Ordering::Acquire));
        let result = self
            .handle
            .take()
            .expect("finished worker has a handle")
            .join()
            .map_err(|_| ());
        Some((cancelled, result))
    }

    pub(super) fn take(&mut self) -> Option<(JoinHandle<T>, Option<Arc<AtomicBool>>)> {
        self.handle
            .take()
            .map(|handle| (handle, self.cancel.take()))
    }

    pub(super) fn cancel_and_discard(&mut self) {
        self.cancel();
        self.handle.take();
        self.cancel.take();
    }
}

/// 検索・Timeline関連ワーカーのライフサイクルを管理する。
///
/// `WorkerTask`がスレッドハンドルとキャンセル通知を一緒に所有し、`UiApp`は
/// ワーカーの開始条件と、完了結果を画面状態へ適用する処理を調停する。
#[derive(Default)]
pub(super) struct SearchJobs {
    pub(super) generation: WorkerTask<Result<ObservationCache, AppError>>,
    pub(super) correction_generation: WorkerTask<Result<ObservationCache, AppError>>,
    pub(super) correction_generation_key: Option<(u32, i64, i64, u64)>,
    pub(super) field_generation: WorkerTask<Result<FieldSearchOutput, AppError>>,
    pub(super) field_target_generation: WorkerTask<FieldTargetOutput>,
    pub(super) field_pool_generation: WorkerTask<Result<FieldSearchPool, AppError>>,
    pub(super) field_timeline_generation: WorkerTask<Result<PreparedTimelineIndex, AppError>>,
    pub(super) field_timeline_extension: WorkerTask<(FieldState, Result<bool, AppError>)>,
    pub(super) field_timeline_extension_key: Option<(usize, i64, i64)>,
    pub(super) field_timeline_extension_blocked: bool,
    pub(super) field_timeline_extension_error: Option<Arc<AppError>>,
    pub(super) field_pool: Option<FieldSearchPool>,
    pub(super) field_idx_inference: WorkerTask<Result<FieldIdxInference, AppError>>,
    pub(super) field_search_needs_restart: bool,
    pub(super) guidance_generation: WorkerTask<GuidanceGenerationOutput>,
    pub(super) candidate_reachability: WorkerTask<CandidateReachabilityOutput>,
    pub(super) candidate_reachability_key: Option<CandidateReachabilityRequest>,
    pub(super) production_preparation: WorkerTask<ProductionPreparationOutput>,
}

impl SearchJobs {
    /// Field検索セッションに属するワーカーをすべてキャンセルし、結果を破棄する。
    ///
    /// ワーカーの寿命は画面表示やタイマー状態ではなく検索セッションに属するため、
    /// キャンセル通知とハンドル破棄を同じ場所で管理する。
    pub(super) fn cancel_field_session(&mut self) {
        self.field_generation.cancel_and_discard();
        self.field_target_generation.cancel_and_discard();
        self.field_search_needs_restart = false;
        self.field_pool_generation.cancel_and_discard();
        self.field_timeline_generation.cancel_and_discard();
        self.field_timeline_extension.cancel_and_discard();
        self.field_timeline_extension_key.take();
        self.field_timeline_extension_blocked = false;
        self.field_timeline_extension_error = None;
        self.field_idx_inference.cancel_and_discard();
        self.field_pool = None;
    }

    pub(super) fn cancel_field_idx_inference(&self) {
        self.field_idx_inference.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::{SearchJobs, WorkerTask};
    use crate::errors::AppError;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    #[test]
    fn cancelling_field_session_signals_every_field_worker_and_resets_session_jobs() {
        let mut jobs = SearchJobs::default();
        let tokens = (0..5)
            .map(|_| Arc::new(AtomicBool::new(false)))
            .collect::<Vec<_>>();
        jobs.field_generation.cancel = Some(tokens[0].clone());
        jobs.field_pool_generation.cancel = Some(tokens[1].clone());
        jobs.field_timeline_generation.cancel = Some(tokens[2].clone());
        jobs.field_target_generation.cancel = Some(tokens[3].clone());
        jobs.field_idx_inference.cancel = Some(tokens[4].clone());
        jobs.field_search_needs_restart = true;
        jobs.field_timeline_extension_blocked = true;
        jobs.field_timeline_extension_error = Some(Arc::new(AppError::WorkerPanicked));

        jobs.cancel_field_session();

        assert!(tokens.iter().all(|token| token.load(Ordering::Acquire)));
        assert!(!jobs.field_search_needs_restart);
        assert!(!jobs.field_timeline_extension_blocked);
        assert!(jobs.field_timeline_extension_error.is_none());
        assert!(jobs.field_generation.cancel.is_none());
        assert!(jobs.field_pool_generation.cancel.is_none());
        assert!(jobs.field_timeline_generation.cancel.is_none());
        assert!(jobs.field_target_generation.cancel.is_none());
        assert!(jobs.field_idx_inference.cancel.is_none());
    }

    #[test]
    fn worker_task_cancellation_and_completion_share_one_lifecycle() {
        let mut task = WorkerTask::default();
        task.spawn_cancellable(|cancel| {
            while !cancel.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            42
        });

        task.cancel();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while !task.is_finished() && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        let Some((cancelled, result)) = task.take_finished() else {
            panic!("cancelled worker should finish within the test deadline");
        };

        assert!(cancelled);
        assert_eq!(result, Ok(42));
        assert!(!task.is_running());
    }

    #[test]
    fn worker_task_does_not_replace_an_active_worker() {
        let mut task = WorkerTask::default();
        let (release_sender, release_receiver) = std::sync::mpsc::channel();
        assert!(task.spawn(move || release_receiver.recv().unwrap()));
        let thread_id = task.thread_id().unwrap();

        assert!(!task.spawn(|| panic!("a second worker must not start")));
        assert_eq!(task.thread_id(), Some(thread_id));

        release_sender.send(()).unwrap();
        while !task.is_finished() {
            std::thread::yield_now();
        }
        let Some((cancelled, result)) = task.take_finished() else {
            panic!("released worker should be available");
        };
        assert!(!cancelled);
        assert_eq!(result, Ok(()));
    }
}
