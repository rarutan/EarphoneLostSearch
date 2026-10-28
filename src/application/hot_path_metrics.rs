//! 明示的に有効化した開発ビルドだけで使う、入力値を含まない処理時間集計。

#[cfg(any(debug_assertions, test))]
use std::time::Instant;

#[derive(Clone, Copy)]
pub(crate) enum HotPath {
    HatchGuidance,
    CandidateSnapshot,
    TargetFallback,
}

impl HotPath {
    #[cfg(any(debug_assertions, test))]
    fn label(self) -> &'static str {
        match self {
            Self::HatchGuidance => "hatch_guidance",
            Self::CandidateSnapshot => "candidate_snapshot",
            Self::TargetFallback => "target_fallback",
        }
    }
}

pub(crate) struct HotPathTimer {
    #[cfg(debug_assertions)]
    operation: HotPath,
    #[cfg(any(debug_assertions, test))]
    started: Option<Instant>,
    #[cfg(any(debug_assertions, test))]
    items: u64,
}

impl HotPathTimer {
    pub(crate) fn start(operation: HotPath) -> Self {
        #[cfg(any(debug_assertions, test))]
        {
            Self::with_enabled(operation, profiling_enabled())
        }
        #[cfg(not(any(debug_assertions, test)))]
        {
            let _ = operation;
            Self {}
        }
    }

    #[cfg(any(debug_assertions, test))]
    fn with_enabled(operation: HotPath, enabled: bool) -> Self {
        #[cfg(not(debug_assertions))]
        let _ = operation;
        Self {
            #[cfg(debug_assertions)]
            operation,
            started: enabled.then(Instant::now),
            items: 0,
        }
    }

    pub(crate) fn with_items(self, items: usize) -> Self {
        #[cfg(any(debug_assertions, test))]
        {
            let mut timer = self;
            timer.items = items as u64;
            timer
        }
        #[cfg(not(any(debug_assertions, test)))]
        {
            let _ = items;
            self
        }
    }

    pub(crate) fn set_items(&mut self, items: usize) {
        #[cfg(any(debug_assertions, test))]
        {
            self.items = items as u64;
        }
        #[cfg(not(any(debug_assertions, test)))]
        {
            let _ = items;
        }
    }
}

#[cfg(any(debug_assertions, test))]
fn format_metric(
    operation: HotPath,
    calls: u64,
    total: std::time::Duration,
    max: std::time::Duration,
    items: u64,
) -> String {
    format!(
        "[hot-path] {} calls={} total_ms={:.3} avg_ms={:.3} max_ms={:.3} items={}",
        operation.label(),
        calls,
        total.as_secs_f64() * 1_000.0,
        total.as_secs_f64() * 1_000.0 / calls.max(1) as f64,
        max.as_secs_f64() * 1_000.0,
        items,
    )
}

#[cfg(any(debug_assertions, test))]
fn profiling_enabled() -> bool {
    #[cfg(debug_assertions)]
    {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ENABLED
            .get_or_init(|| enabled_from_value(std::env::var_os("HATCH_BLINK_PROFILE").as_deref()))
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

#[cfg(any(debug_assertions, test))]
fn enabled_from_value(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|value| value == "1")
}

#[cfg(debug_assertions)]
impl Drop for HotPathTimer {
    fn drop(&mut self) {
        use std::sync::{Mutex, OnceLock};
        use std::time::Duration;

        let Some(started) = self.started else {
            return;
        };
        #[derive(Default)]
        struct Metric {
            calls: u64,
            elapsed: Duration,
            max: Duration,
            items: u64,
        }
        struct Window {
            started: Instant,
            metrics: [Metric; 3],
        }
        static WINDOW: OnceLock<Mutex<Window>> = OnceLock::new();
        let elapsed = started.elapsed();
        let index = match self.operation {
            HotPath::HatchGuidance => 0,
            HotPath::CandidateSnapshot => 1,
            HotPath::TargetFallback => 2,
        };
        let window = WINDOW.get_or_init(|| {
            Mutex::new(Window {
                started: Instant::now(),
                metrics: std::array::from_fn(|_| Metric::default()),
            })
        });
        let Ok(mut window) = window.lock() else {
            return;
        };
        let metric = &mut window.metrics[index];
        metric.calls += 1;
        metric.elapsed += elapsed;
        metric.max = metric.max.max(elapsed);
        metric.items = metric.items.saturating_add(self.items);
        if window.started.elapsed() >= Duration::from_secs(5) {
            for (operation, metric) in [
                HotPath::HatchGuidance,
                HotPath::CandidateSnapshot,
                HotPath::TargetFallback,
            ]
            .into_iter()
            .zip(window.metrics.iter())
            {
                if metric.calls == 0 {
                    continue;
                }
                eprintln!(
                    "{}",
                    format_metric(
                        operation,
                        metric.calls,
                        metric.elapsed,
                        metric.max,
                        metric.items,
                    )
                );
            }
            window.started = Instant::now();
            window.metrics = std::array::from_fn(|_| Metric::default());
        }
    }
}

#[cfg(not(debug_assertions))]
impl Drop for HotPathTimer {
    fn drop(&mut self) {}
}

#[cfg(test)]
mod tests {
    #[test]
    fn opt_in_requires_the_exact_debug_environment_switch() {
        use std::ffi::OsStr;

        assert!(!super::enabled_from_value(None));
        assert!(!super::enabled_from_value(Some(OsStr::new("true"))));
        assert!(!super::enabled_from_value(Some(OsStr::new("0"))));
        assert!(super::enabled_from_value(Some(OsStr::new("1"))));
    }

    #[test]
    fn aggregate_report_contains_only_operation_and_measurements() {
        let report = super::format_metric(
            super::HotPath::HatchGuidance,
            2,
            std::time::Duration::from_millis(1),
            std::time::Duration::from_micros(800),
            4096,
        );
        assert!(report.contains("hatch_guidance"));
        assert!(report.contains("items=4096"));
        assert!(!report.contains("seed"));
        assert!(!report.contains("target="));
    }

    #[test]
    fn disabled_sampler_does_not_start_a_clock_or_emit_on_drop() {
        let timer = super::HotPathTimer::with_enabled(super::HotPath::TargetFallback, false);
        assert!(timer.started.is_none());
        drop(timer);
    }
}
