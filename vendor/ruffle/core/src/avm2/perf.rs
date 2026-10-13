//! Bounded, opt-in observations of long synchronous AVM2 callbacks.
//!
//! Samples are taken at existing interpreter timeout checkpoints, not at every
//! opcode. They show where execution was observed, not per-method CPU totals.
//! Only code metadata is recorded; no arguments, object values or URLs.

use super::{Avm2, method::Method};
use crate::string::WString;
use std::cell::Cell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::time::Duration;
use web_time::Instant;

const SAMPLE_INTERVAL: Duration = Duration::from_millis(100);
const MAX_SAMPLES: u32 = 8;
const MAX_STACK_DEPTH: usize = 16;

#[derive(Clone, Copy)]
struct CallbackTrace {
    started: Instant,
    last_sample: Duration,
    samples: u32,
    native_reports: u32,
    text_layouts: u32,
    hidden_text_layouts: u32,
    text_layout_wall: Duration,
}

impl CallbackTrace {
    fn take_sample(&mut self, elapsed: Duration) -> Option<u32> {
        if self.samples >= MAX_SAMPLES || elapsed.saturating_sub(self.last_sample) < SAMPLE_INTERVAL
        {
            return None;
        }
        self.last_sample = elapsed;
        self.samples += 1;
        Some(self.samples)
    }
}

thread_local! {
    static CALLBACK_TRACE: Cell<Option<CallbackTrace>> = const { Cell::new(None) };
}

/// Nested event dispatch shares the outer callback's sample budget. The guard
/// also clears the state on early errors or unwinding, and stays on its thread.
pub(crate) struct CallbackGuard {
    outermost: bool,
    _same_thread: PhantomData<Rc<()>>,
}

impl CallbackGuard {
    pub(crate) fn enter() -> Self {
        let outermost = CALLBACK_TRACE.with(|slot| {
            if slot.get().is_some() {
                return false;
            }
            slot.set(Some(CallbackTrace {
                started: Instant::now(),
                last_sample: Duration::ZERO,
                samples: 0,
                native_reports: 0,
                text_layouts: 0,
                hidden_text_layouts: 0,
                text_layout_wall: Duration::ZERO,
            }));
            true
        });
        Self {
            outermost,
            _same_thread: PhantomData,
        }
    }
}

impl Drop for CallbackGuard {
    fn drop(&mut self) {
        if self.outermost
            && let Some(trace) = CALLBACK_TRACE.with(Cell::take)
            && trace.text_layout_wall >= Duration::from_millis(25)
        {
            tracing::info!(target: "zm_perf",
                    count = trace.text_layouts,
                    hidden_count = trace.hidden_text_layouts,
                    elapsed_ms = trace.text_layout_wall.as_secs_f64() * 1000.0,
                    "AVM2 callback cumulative text layout (CPU wall; includes short calls)");
        }
    }
}

/// Called only at the existing, batched interpreter timeout check.
pub(crate) fn sample_checkpoint(avm2: &Avm2<'_>) {
    let sample = CALLBACK_TRACE.with(|slot| {
        let mut trace = slot.get()?;
        let elapsed = trace.started.elapsed();
        let index = trace.take_sample(elapsed)?;
        slot.set(Some(trace));
        Some((index, elapsed))
    });
    if let Some((index, elapsed)) = sample {
        tracing::info!(
            target: "zm_perf",
            sample = index,
            callback_elapsed_ms = elapsed.as_secs_f64() * 1000.0,
            stack = ?avm2.call_stack().borrow().performance_sample(MAX_STACK_DEPTH),
            "AVM2 callback stack sample (innermost first; checkpoint observation, not CPU attribution)"
        );
    }
}

/// Native work does not pass through interpreter checkpoints. Time it only
/// inside explicitly traced callbacks so a single blocking builtin is visible.
#[inline]
pub(crate) fn start_native() -> Option<Instant> {
    CALLBACK_TRACE.with(|slot| slot.get().map(|_| Instant::now()))
}

// Individual layout calls can stay below the native-call reporting threshold
// while hundreds of them stall one callback. Keep only counts and wall time.
pub(crate) fn finish_text_layout(started: Option<Instant>, hidden: bool) {
    let Some(started) = started else {
        return;
    };
    let elapsed = started.elapsed();
    CALLBACK_TRACE.with(|slot| {
        if let Some(mut trace) = slot.get() {
            trace.text_layouts = trace.text_layouts.saturating_add(1);
            trace.hidden_text_layouts = trace.hidden_text_layouts.saturating_add(u32::from(hidden));
            trace.text_layout_wall += elapsed;
            slot.set(Some(trace));
        }
    });
}

fn take_native_report() -> bool {
    CALLBACK_TRACE.with(|slot| {
        let Some(mut trace) = slot.get() else {
            return false;
        };
        if trace.native_reports >= MAX_SAMPLES {
            return false;
        }
        trace.native_reports += 1;
        slot.set(Some(trace));
        true
    })
}

pub(crate) fn finish_native(started: Option<Instant>, avm2: &Avm2<'_>) {
    if let Some(started) = started {
        let elapsed = started.elapsed();
        if elapsed >= Duration::from_millis(25) && take_native_report() {
            tracing::info!(
                target: "zm_perf",
                elapsed_ms = elapsed.as_secs_f64() * 1000.0,
                stack = ?avm2.call_stack().borrow().performance_sample(MAX_STACK_DEPTH),
                "Slow AVM2 native call (CPU wall time; nested script calls may overlap)"
            );
        }
    }
}

pub(crate) fn measure_verification<T>(method: Method<'_>, operation: impl FnOnce() -> T) -> T {
    let started = tracing::enabled!(target: "zm_perf", tracing::Level::INFO).then(Instant::now);
    let result = operation();
    if let Some(started) = started {
        let elapsed = started.elapsed();
        if elapsed >= Duration::from_millis(25) {
            let mut name = WString::new();
            super::function::display_function(&mut name, method);
            tracing::info!(
                target: "zm_perf",
                method = %name.to_utf8_lossy(),
                elapsed_ms = elapsed.as_secs_f64() * 1000.0,
                "Slow AVM2 method verification (first use; CPU wall time)"
            );
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_samples_are_spaced_and_bounded() {
        let mut trace = CallbackTrace {
            started: Instant::now(),
            last_sample: Duration::ZERO,
            samples: 0,
            native_reports: 0,
            text_layouts: 0,
            hidden_text_layouts: 0,
            text_layout_wall: Duration::ZERO,
        };
        assert_eq!(trace.take_sample(Duration::from_millis(99)), None);
        assert_eq!(trace.take_sample(Duration::from_millis(100)), Some(1));
        assert_eq!(trace.take_sample(Duration::from_millis(199)), None);
        // A delayed checkpoint yields one observation, not invented catch-up samples.
        assert_eq!(trace.take_sample(Duration::from_millis(750)), Some(2));
        for index in 3..=MAX_SAMPLES {
            assert_eq!(
                trace.take_sample(Duration::from_millis(750 + u64::from(index - 2) * 100)),
                Some(index)
            );
        }
        assert_eq!(trace.take_sample(Duration::from_secs(30)), None);
    }

    #[test]
    fn cumulative_layout_includes_short_calls_and_is_cleared_per_callback() {
        assert!(start_native().is_none());
        {
            let _guard = CallbackGuard::enter();
            finish_text_layout(start_native(), true);
            {
                let _nested = CallbackGuard::enter();
                finish_text_layout(start_native(), false);
            }
            let trace = CALLBACK_TRACE.with(|slot| slot.get().unwrap());
            assert_eq!(trace.text_layouts, 2);
            assert_eq!(trace.hidden_text_layouts, 1);
        }
        assert!(start_native().is_none());
        let _guard = CallbackGuard::enter();
        assert_eq!(
            CALLBACK_TRACE.with(|slot| slot.get().unwrap().text_layouts),
            0
        );
    }

    #[test]
    fn nested_callbacks_share_budget_and_errors_clear_state() {
        assert!(start_native().is_none());
        {
            let _outer = CallbackGuard::enter();
            for _ in 0..MAX_SAMPLES {
                assert!(take_native_report());
            }
            assert!(!take_native_report());
            CALLBACK_TRACE.with(|slot| {
                let mut trace = slot.get().unwrap();
                trace.samples = MAX_SAMPLES;
                slot.set(Some(trace));
            });
            {
                let _nested = CallbackGuard::enter();
                assert!(!take_native_report());
                assert_eq!(
                    CALLBACK_TRACE.with(|slot| slot.get().unwrap().samples),
                    MAX_SAMPLES
                );
            }
            assert!(start_native().is_some());
        }
        assert!(start_native().is_none());
        let _ = std::panic::catch_unwind(|| {
            let _guard = CallbackGuard::enter();
            panic!("fixture");
        });
        assert!(start_native().is_none());
        let _next = CallbackGuard::enter();
        assert_eq!(CALLBACK_TRACE.with(|slot| slot.get().unwrap().samples), 0);
    }
}
