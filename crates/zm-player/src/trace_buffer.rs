use std::collections::VecDeque;

const HISTORY_LIMIT: usize = 160;

struct Entry {
    line: String,
    occurrences: u64,
}

/// Consecutive identical traces occupy one history slot. The exact count stays
/// in diagnostics; disk output samples repeats at powers of two to avoid spam.
/// Warnings are never folded, and callers still process every compatibility event.
#[derive(Default)]
pub(crate) struct TraceBuffer {
    entries: VecDeque<Entry>,
}

impl TraceBuffer {
    pub(crate) fn record(&mut self, line: String, is_trace: bool) -> Option<String> {
        if is_trace
            && let Some(previous) = self.entries.back_mut()
            && previous.line == line
        {
            previous.occurrences = previous.occurrences.saturating_add(1);
            return previous.occurrences.is_power_of_two().then(|| {
                format!(
                    "{} [consecutive occurrences={}]",
                    previous.line, previous.occurrences
                )
            });
        }
        if self.entries.len() == HISTORY_LIMIT {
            self.entries.pop_front();
        }
        self.entries.push_back(Entry {
            line: line.clone(),
            occurrences: 1,
        });
        Some(line)
    }

    pub(crate) fn summary(&self, limit: usize) -> String {
        let mut output = String::new();
        for entry in self.entries.iter().rev().take(limit).rev() {
            output.push_str(&entry.line);
            if entry.occurrences > 1 {
                use std::fmt::Write;
                let _ = write!(output, " [consecutive occurrences={}]", entry.occurrences);
            }
            output.push('\n');
        }
        output
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noisy_trace_preserves_earlier_context_and_exact_count() {
        let mut history = TraceBuffer::default();
        history.record("trace: useful context".into(), true);
        let emitted = (0..10_000)
            .filter(|_| history.record("trace: repeated".into(), true).is_some())
            .count();
        assert_eq!(emitted, 14);
        assert_eq!(
            history.summary(40),
            "trace: useful context\ntrace: repeated [consecutive occurrences=10000]\n"
        );
        history.record("trace: another event".into(), true);
        history.record("trace: repeated".into(), true);
        assert_eq!(history.entries.len(), 4);
    }

    #[test]
    fn warnings_remain_separate_and_history_is_bounded() {
        let mut history = TraceBuffer::default();
        for _ in 0..200 {
            assert!(
                history
                    .record("warning: repeated warning".into(), false)
                    .is_some()
            );
        }
        assert_eq!(history.entries.len(), HISTORY_LIMIT);
        assert_eq!(history.summary(40).lines().count(), 40);
        assert!(!history.summary(40).contains("occurrences"));
        history.clear();
        assert!(history.summary(40).is_empty());
    }
}
