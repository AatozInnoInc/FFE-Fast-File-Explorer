#[cfg(test)]
use std::collections::VecDeque;
#[cfg(any(test, windows))]
use std::time::Instant;

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use std::thread;
#[cfg(test)]
use std::time::Duration;

#[cfg(any(test, windows))]
use crate::cancel::CancellationToken;
#[cfg(any(test, windows))]
use crate::kernel::EnumError;
#[cfg(any(test, windows))]
use crate::kernel::FileEntry;

#[cfg(any(test, windows))]
pub(crate) trait DirectorySource {
    fn next_entry(
        &mut self,
        cancel: &CancellationToken,
        deadline: Option<Instant>,
    ) -> Result<Option<FileEntry>, EnumError>;
}

#[cfg(test)]
#[derive(Debug)]
enum ScriptStep {
    Delay(Duration),
    Hang,
    Entry(FileEntry),
}

/// Test double for [`DirectorySource`]. Not a second enumeration design.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct ScriptedSource {
    steps: VecDeque<ScriptStep>,
    remaining_entries: Arc<AtomicUsize>,
}

#[cfg(test)]
impl ScriptedSource {
    pub(crate) fn entries(items: Vec<(FileEntry, Duration)>) -> Self {
        let mut steps = VecDeque::new();
        for (entry, delay) in items {
            if !delay.is_zero() {
                steps.push_back(ScriptStep::Delay(delay));
            }
            steps.push_back(ScriptStep::Entry(entry));
        }
        Self::from_steps(steps)
    }

    pub(crate) fn hang() -> Self {
        Self::from_steps(VecDeque::from([ScriptStep::Hang]))
    }

    fn from_steps(steps: VecDeque<ScriptStep>) -> Self {
        let remaining = steps
            .iter()
            .filter(|step| matches!(step, ScriptStep::Entry(_)))
            .count();
        Self {
            steps,
            remaining_entries: Arc::new(AtomicUsize::new(remaining)),
        }
    }

    pub(crate) fn remaining(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.remaining_entries)
    }

    fn check(
        &self,
        cancel: &CancellationToken,
        deadline: Option<Instant>,
    ) -> Result<(), EnumError> {
        if cancel.is_cancelled() {
            return Err(EnumError::Cancelled);
        }
        if deadline.is_some_and(|limit| Instant::now() >= limit) {
            return Err(EnumError::TimedOut);
        }
        Ok(())
    }

    fn pause(
        &self,
        total: Duration,
        cancel: &CancellationToken,
        deadline: Option<Instant>,
    ) -> Result<(), EnumError> {
        let start = Instant::now();
        while start.elapsed() < total {
            self.check(cancel, deadline)?;
            let slice = Duration::from_millis(5).min(total.saturating_sub(start.elapsed()));
            if slice.is_zero() {
                break;
            }
            thread::sleep(slice);
        }
        self.check(cancel, deadline)
    }

    fn park(&self, cancel: &CancellationToken, deadline: Option<Instant>) -> Result<(), EnumError> {
        loop {
            self.check(cancel, deadline)?;
            thread::sleep(Duration::from_millis(5));
        }
    }
}

#[cfg(test)]
impl DirectorySource for ScriptedSource {
    fn next_entry(
        &mut self,
        cancel: &CancellationToken,
        deadline: Option<Instant>,
    ) -> Result<Option<FileEntry>, EnumError> {
        loop {
            let Some(step) = self.steps.front() else {
                return Ok(None);
            };
            match step {
                ScriptStep::Delay(delay) => {
                    let delay = *delay;
                    self.pause(delay, cancel, deadline)?;
                    self.steps.pop_front();
                }
                ScriptStep::Hang => {
                    self.park(cancel, deadline)?;
                }
                ScriptStep::Entry(_) => {
                    let Some(ScriptStep::Entry(entry)) = self.steps.pop_front() else {
                        return Ok(None);
                    };
                    self.remaining_entries.fetch_sub(1, Ordering::AcqRel);
                    return Ok(Some(entry));
                }
            }
        }
    }
}
