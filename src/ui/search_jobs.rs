use crate::application::field::{
    FieldIdxInference, FieldSearchPool, PreparedTimelineIndex, TimelineExtensionResult,
};
use crate::domain::rng::ObservationCache;
use crate::errors::AppError;
use crate::ui::app_types::{
    CandidateReachabilityOutput, ConfirmedTargetSearchOutput, FieldSearchOutput, FieldTargetOutput,
    GuidanceGenerationOutput, PreparedTimeleapTarget, PreparedTimeleapTargetOutput,
    ProductionPreparationOutput, TimeleapCandidateOutput, TimeleapFrame1Output,
    TimeleapFrame1PreviewOutput, TimeleapReachabilityOutput, TimeleapShiftResolutionOutput,
    TimeleapTargetCheckOutput,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::JoinHandle;

pub(super) type PreparedTimeleapTargetJob =
    Box<dyn FnOnce(Arc<AtomicBool>) -> PreparedTimeleapTargetOutput + Send + 'static>;

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
    pub(super) field_timeline_extension: WorkerTask<Result<TimelineExtensionResult, AppError>>,
    pub(super) field_timeline_extension_key: Option<u64>,
    pub(super) field_timeline_extension_generation: u64,
    pub(super) field_timeline_extension_blocked: bool,
    pub(super) field_timeline_extension_error: Option<Arc<AppError>>,
    pub(super) field_pool: Option<FieldSearchPool>,
    pub(super) field_idx_inference: WorkerTask<Result<FieldIdxInference, AppError>>,
    pub(super) field_timeleap_candidates: WorkerTask<TimeleapCandidateOutput>,
    pub(super) field_timeleap_frame1: WorkerTask<TimeleapFrame1Output>,
    pub(super) field_timeleap_prepared_target: WorkerTask<PreparedTimeleapTargetOutput>,
    pub(super) prepared_timeleap_target: Option<PreparedTimeleapTarget>,
    pending_prepared_timeleap_target: Option<PreparedTimeleapTargetJob>,
    pub(super) field_timeleap_frame1_preview: WorkerTask<TimeleapFrame1PreviewOutput>,
    pub(super) field_timeleap_target_check: WorkerTask<TimeleapTargetCheckOutput>,
    pub(super) field_timeleap_reachability: WorkerTask<TimeleapReachabilityOutput>,
    pub(super) field_timeleap_shift_resolution: WorkerTask<TimeleapShiftResolutionOutput>,
    pub(super) field_confirmed_target_search: WorkerTask<ConfirmedTargetSearchOutput>,
    pub(super) field_confirmed_target_search_key: Option<(u64, u64)>,
    pub(super) field_search_needs_restart: bool,
    pub(super) guidance_generation: WorkerTask<GuidanceGenerationOutput>,
    pub(super) candidate_reachability: WorkerTask<CandidateReachabilityOutput>,
    pub(super) candidate_reachability_key:
        Option<crate::application::state::CandidateReachabilityKey>,
    pub(super) normal_target_wait: WorkerTask<crate::ui::app_types::NormalTargetWaitOutput>,
    pub(super) normal_target_wait_key: Option<crate::application::state::NormalTargetWaitKey>,
    pub(super) production_preparation: WorkerTask<ProductionPreparationOutput>,
}

impl SearchJobs {
    /// Field観測セッションに属するワーカーをキャンセルし、結果を破棄する。
    ///
    /// ワーカーの寿命は画面表示やタイマー状態ではなく検索セッションに属するため、
    /// キャンセル通知とハンドル破棄を同じ場所で管理する。
    pub(super) fn cancel_field_session(&mut self) {
        self.field_generation.cancel_and_discard();
        self.field_target_generation.cancel_and_discard();
        self.field_search_needs_restart = false;
        self.field_pool_generation.cancel_and_discard();
        self.field_timeline_generation.cancel_and_discard();
        self.cancel_field_timeline_extension();
        self.field_idx_inference.cancel_and_discard();
        // Frame1候補検索はメイン観測から独立した検索ウィンドウのworker。
        self.field_timeleap_frame1.cancel_and_discard();
        self.cancel_prepared_timeleap_target();
        self.field_timeleap_frame1_preview.cancel_and_discard();
        self.field_timeleap_target_check.cancel_and_discard();
        self.field_timeleap_reachability.cancel_and_discard();
        self.field_timeleap_shift_resolution.cancel_and_discard();
        self.field_confirmed_target_search.cancel_and_discard();
        self.field_confirmed_target_search_key = None;
        self.field_pool = None;
    }

    pub(super) fn cancel_prepared_timeleap_target(&mut self) {
        self.field_timeleap_prepared_target.cancel();
        self.pending_prepared_timeleap_target = None;
        self.prepared_timeleap_target = None;
    }

    pub(super) fn queue_prepared_timeleap_target(&mut self, job: PreparedTimeleapTargetJob) {
        self.prepared_timeleap_target = None;
        if self.field_timeleap_prepared_target.is_running() {
            self.pending_prepared_timeleap_target = Some(job);
            self.field_timeleap_prepared_target.cancel();
            return;
        }
        self.spawn_prepared_timeleap_target(job);
    }

    pub(super) fn poll_prepared_timeleap_target(&mut self) -> bool {
        if let Some((cancelled, joined)) = self.field_timeleap_prepared_target.take_finished() {
            if !cancelled && self.pending_prepared_timeleap_target.is_none() {
                if let Ok(Ok(prepared)) = joined {
                    self.prepared_timeleap_target = Some(prepared);
                }
            }
            if let Some(job) = self.pending_prepared_timeleap_target.take() {
                self.spawn_prepared_timeleap_target(job);
            }
        }
        self.field_timeleap_prepared_target.is_running()
    }

    fn spawn_prepared_timeleap_target(&mut self, job: PreparedTimeleapTargetJob) {
        let started = self
            .field_timeleap_prepared_target
            .spawn_cancellable(move |cancel| job(cancel));
        debug_assert!(
            started,
            "prepared target worker slot must be idle before spawn"
        );
    }

    pub(super) fn cancel_field_timeline_extension(&mut self) {
        self.field_timeline_extension.cancel_and_discard();
        self.field_timeline_extension_key = None;
        self.field_timeline_extension_generation =
            self.field_timeline_extension_generation.wrapping_add(1);
        self.field_timeline_extension_blocked = false;
        self.field_timeline_extension_error = None;
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
    fn cancelling_field_session_preserves_independent_frame1_search_worker() {
        let mut jobs = SearchJobs::default();
        let tokens = (0..14)
            .map(|_| Arc::new(AtomicBool::new(false)))
            .collect::<Vec<_>>();
        jobs.field_generation.cancel = Some(tokens[0].clone());
        jobs.field_pool_generation.cancel = Some(tokens[1].clone());
        jobs.field_timeline_generation.cancel = Some(tokens[2].clone());
        jobs.field_target_generation.cancel = Some(tokens[3].clone());
        jobs.field_idx_inference.cancel = Some(tokens[4].clone());
        jobs.field_timeleap_candidates.cancel = Some(tokens[5].clone());
        jobs.field_timeleap_frame1.cancel = Some(tokens[6].clone());
        jobs.field_timeleap_frame1_preview.cancel = Some(tokens[7].clone());
        jobs.field_timeleap_reachability.cancel = Some(tokens[8].clone());
        jobs.field_timeleap_target_check.cancel = Some(tokens[9].clone());
        jobs.field_confirmed_target_search.cancel = Some(tokens[10].clone());
        jobs.field_timeleap_shift_resolution.cancel = Some(tokens[11].clone());
        jobs.field_timeline_extension.cancel = Some(tokens[12].clone());
        jobs.field_timeleap_prepared_target.cancel = Some(tokens[13].clone());
        jobs.field_confirmed_target_search_key = Some((3, 2));
        jobs.field_search_needs_restart = true;
        jobs.field_timeline_extension_blocked = true;
        jobs.field_timeline_extension_error = Some(Arc::new(AppError::WorkerPanicked));

        jobs.cancel_field_session();

        assert!(tokens
            .iter()
            .enumerate()
            .all(|(index, token)| token.load(Ordering::Acquire) == (index != 5)));
        assert!(!jobs.field_search_needs_restart);
        assert!(!jobs.field_timeline_extension_blocked);
        assert!(jobs.field_timeline_extension_error.is_none());
        assert!(jobs.field_timeline_extension_key.is_none());
        assert_eq!(jobs.field_timeline_extension_generation, 1);
        assert!(jobs.field_generation.cancel.is_none());
        assert!(jobs.field_pool_generation.cancel.is_none());
        assert!(jobs.field_timeline_generation.cancel.is_none());
        assert!(jobs.field_target_generation.cancel.is_none());
        assert!(jobs.field_idx_inference.cancel.is_none());
        assert!(jobs.field_timeleap_candidates.cancel.is_some());
        assert!(jobs.field_timeleap_frame1.cancel.is_none());
        assert!(jobs.field_timeleap_prepared_target.cancel.is_some());
        assert!(jobs.pending_prepared_timeleap_target.is_none());
        assert!(jobs.field_timeleap_frame1_preview.cancel.is_none());
        assert!(jobs.field_timeleap_target_check.cancel.is_none());
        assert!(jobs.field_confirmed_target_search.cancel.is_none());
        assert!(jobs.field_timeleap_shift_resolution.cancel.is_none());
        assert!(jobs.field_timeline_extension.cancel.is_none());
        assert!(jobs.field_confirmed_target_search_key.is_none());
        assert!(jobs.field_timeleap_reachability.cancel.is_none());
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
        assert!(
            task.is_running(),
            "requesting cancellation must retain its worker handle"
        );
        assert!(
            !task.spawn(|| panic!("a retry must wait until cancellation is collected")),
            "cancellation must not detach the old worker and overlap a retry"
        );
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

        assert!(
            task.spawn(|| 7),
            "retry is allowed after the old worker is joined"
        );
        while !task.is_finished() {
            std::thread::yield_now();
        }
        let Some((cancelled, result)) = task.take_finished() else {
            panic!("retry worker should be collected");
        };
        assert!(!cancelled);
        assert_eq!(result, Ok(7));
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

    #[test]
    fn prepared_target_requests_coalesce_while_cancelled_worker_is_reaped() {
        let mut jobs = SearchJobs::default();
        let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let maximum_active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let ran_old = Arc::new(AtomicBool::new(false));
        let ran_middle = Arc::new(AtomicBool::new(false));
        let ran_latest = Arc::new(AtomicBool::new(false));
        let (started_tx, started_rx) = std::sync::mpsc::channel();

        let worker_active = active.clone();
        let worker_maximum = maximum_active.clone();
        let worker_ran_old = ran_old.clone();
        assert!(jobs
            .field_timeleap_prepared_target
            .spawn_cancellable(move |cancel| {
                let count = worker_active.fetch_add(1, Ordering::SeqCst) + 1;
                worker_maximum.fetch_max(count, Ordering::SeqCst);
                worker_ran_old.store(true, Ordering::SeqCst);
                started_tx.send(()).unwrap();
                while !cancel.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                worker_active.fetch_sub(1, Ordering::SeqCst);
                Err(AppError::Cancelled)
            }));
        started_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();

        let worker_active = active.clone();
        let worker_maximum = maximum_active.clone();
        let worker_ran_middle = ran_middle.clone();
        jobs.queue_prepared_timeleap_target(Box::new(move |_| {
            let count = worker_active.fetch_add(1, Ordering::SeqCst) + 1;
            worker_maximum.fetch_max(count, Ordering::SeqCst);
            worker_ran_middle.store(true, Ordering::SeqCst);
            worker_active.fetch_sub(1, Ordering::SeqCst);
            Err(AppError::Cancelled)
        }));

        let worker_active = active.clone();
        let worker_maximum = maximum_active.clone();
        let worker_ran_latest = ran_latest.clone();
        jobs.queue_prepared_timeleap_target(Box::new(move |_| {
            let count = worker_active.fetch_add(1, Ordering::SeqCst) + 1;
            worker_maximum.fetch_max(count, Ordering::SeqCst);
            worker_ran_latest.store(true, Ordering::SeqCst);
            worker_active.fetch_sub(1, Ordering::SeqCst);
            Err(AppError::Cancelled)
        }));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while jobs.field_timeleap_prepared_target.is_running()
            && std::time::Instant::now() < deadline
        {
            jobs.poll_prepared_timeleap_target();
            std::thread::yield_now();
        }
        jobs.poll_prepared_timeleap_target();

        assert!(ran_old.load(Ordering::SeqCst));
        assert!(!ran_middle.load(Ordering::SeqCst));
        assert!(ran_latest.load(Ordering::SeqCst));
        assert_eq!(maximum_active.load(Ordering::SeqCst), 1);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert!(!jobs.field_timeleap_prepared_target.is_running());
    }

    #[test]
    fn cancelling_prepared_target_clears_pending_request_without_detaching_worker() {
        let mut jobs = SearchJobs::default();
        let ran_pending = Arc::new(AtomicBool::new(false));
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        assert!(jobs
            .field_timeleap_prepared_target
            .spawn_cancellable(move |cancel| {
                started_tx.send(()).unwrap();
                while !cancel.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                Err(AppError::Cancelled)
            }));
        started_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();

        let pending_ran = ran_pending.clone();
        jobs.queue_prepared_timeleap_target(Box::new(move |_| {
            pending_ran.store(true, Ordering::SeqCst);
            Err(AppError::Cancelled)
        }));
        jobs.cancel_prepared_timeleap_target();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while jobs.field_timeleap_prepared_target.is_running()
            && std::time::Instant::now() < deadline
        {
            jobs.poll_prepared_timeleap_target();
            std::thread::yield_now();
        }
        jobs.poll_prepared_timeleap_target();

        assert!(!ran_pending.load(Ordering::SeqCst));
        assert!(!jobs.field_timeleap_prepared_target.is_running());
        assert!(jobs.pending_prepared_timeleap_target.is_none());
    }
}
