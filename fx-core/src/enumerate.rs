#[cfg(any(test, windows))]
use std::collections::HashMap;
#[cfg(any(test, windows))]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(any(test, windows))]
use std::sync::mpsc::{self, Sender};
use std::sync::mpsc::{Receiver, TryRecvError};
#[cfg(any(test, windows))]
use std::sync::LazyLock;
use std::sync::{Arc, Mutex};
#[cfg(any(test, windows))]
use std::thread;
use std::time::{Duration, Instant};

use crate::cancel::CancellationToken;
#[cfg(any(test, windows))]
use crate::kernel::VolumeKind;
use crate::kernel::{EnumError, FileEntry, Path, VolumeId};
#[cfg(any(test, windows))]
use crate::source::DirectorySource;
#[cfg(test)]
use crate::source::ScriptedSource;

#[cfg(windows)]
use crate::windows::WindowsSource;

/// Implementation batch size. Not a product performance budget.
pub const DEFAULT_BATCH_SIZE: usize = 256;

/// Default deadline for [`crate::VolumeKind::Network`]. Local volumes do not use it.
pub const DEFAULT_NETWORK_TIMEOUT: Duration = Duration::from_secs(2);

/// What to list, and which volume owns the worker.
pub struct ListingOptions {
    pub volume: VolumeId,
    pub directory: Path,
    pub batch_size: usize,
    /// Override for a network volume. `None` uses [`DEFAULT_NETWORK_TIMEOUT`].
    /// Ignored for local volumes.
    pub network_timeout: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListingEvent {
    Batch(Vec<FileEntry>),
    Finished,
    Cancelled,
    TimedOut,
    Failed(EnumError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TryEvent {
    Ready(ListingEvent),
    Pending,
}

/// Non-blocking view of one listing. Dropping it does not join the worker.
pub struct Listing {
    rx: Receiver<ListingEvent>,
    clock: Arc<Clock>,
    terminal: Option<ListingEvent>,
}

impl Listing {
    /// Returns the next event. Never waits.
    ///
    /// A deadline that passes while the worker is inside one blocking call still
    /// reports [`ListingEvent::TimedOut`] from here.
    pub fn try_recv(&mut self) -> TryEvent {
        if let Some(terminal) = &self.terminal {
            return TryEvent::Ready(terminal.clone());
        }
        match self.rx.try_recv() {
            Ok(event) => {
                if is_terminal(&event) {
                    self.terminal = Some(event.clone());
                }
                TryEvent::Ready(event)
            }
            Err(TryRecvError::Empty) => {
                if self.clock.passed() {
                    self.finish(ListingEvent::TimedOut)
                } else {
                    TryEvent::Pending
                }
            }
            Err(TryRecvError::Disconnected) => {
                if self.clock.passed() {
                    self.finish(ListingEvent::TimedOut)
                } else {
                    self.finish(ListingEvent::Failed(EnumError::WorkerStopped))
                }
            }
        }
    }
}

impl Listing {
    fn finish(&mut self, event: ListingEvent) -> TryEvent {
        self.terminal = Some(event.clone());
        TryEvent::Ready(event)
    }
}

fn is_terminal(event: &ListingEvent) -> bool {
    matches!(
        event,
        ListingEvent::Finished
            | ListingEvent::Cancelled
            | ListingEvent::TimedOut
            | ListingEvent::Failed(_)
    )
}

struct Clock {
    timeout: Option<Duration>,
    started: Mutex<Option<Instant>>,
}

impl Clock {
    #[cfg(any(test, windows))]
    fn start(&self) -> Option<Instant> {
        let now = Instant::now();
        let mut started = self.started.lock().unwrap_or_else(|err| err.into_inner());
        *started = Some(now);
        self.timeout.map(|timeout| now + timeout)
    }

    fn passed(&self) -> bool {
        let Some(timeout) = self.timeout else {
            return false;
        };
        let started = self.started.lock().unwrap_or_else(|err| err.into_inner());
        started.is_some_and(|start| Instant::now() >= start + timeout)
    }
}

#[cfg(any(test, windows))]
pub(crate) fn network_timeout_budget(
    kind: VolumeKind,
    network_timeout: Option<Duration>,
) -> Option<Duration> {
    match kind {
        VolumeKind::Local => None,
        VolumeKind::Network => Some(network_timeout.unwrap_or(DEFAULT_NETWORK_TIMEOUT)),
    }
}

#[cfg(any(test, windows))]
enum Backend {
    #[cfg(test)]
    Scripted(ScriptedSource),
    #[cfg(windows)]
    Windows(Path),
}

#[cfg(any(test, windows))]
static REGISTRY: LazyLock<Mutex<HashMap<String, Arc<AtomicBool>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[cfg(any(test, windows))]
struct FinishGuard(Arc<AtomicBool>);

#[cfg(any(test, windows))]
impl Drop for FinishGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[cfg(any(test, windows))]
fn acquire(key: &str) -> Result<Arc<AtomicBool>, EnumError> {
    let mut registry = REGISTRY.lock().unwrap_or_else(|err| err.into_inner());
    if let Some(existing) = registry.get(key) {
        if !existing.load(Ordering::Acquire) {
            return Err(EnumError::VolumeBusy);
        }
    }
    let finished = Arc::new(AtomicBool::new(false));
    registry.insert(key.to_string(), Arc::clone(&finished));
    Ok(finished)
}

/// Enumerate `options.directory` on Windows with `FindFirstFileExW`.
pub fn enumerate(options: ListingOptions) -> Result<(Listing, CancellationToken), EnumError> {
    start_windows(options)
}

#[cfg(windows)]
fn start_windows(options: ListingOptions) -> Result<(Listing, CancellationToken), EnumError> {
    let directory = options.directory.clone();
    spawn_listing(options, Backend::Windows(directory))
}

#[cfg(not(windows))]
fn start_windows(_options: ListingOptions) -> Result<(Listing, CancellationToken), EnumError> {
    Err(EnumError::WindowsOnly)
}

#[cfg(test)]
pub(crate) fn enumerate_scripted(
    options: ListingOptions,
    source: ScriptedSource,
) -> Result<(Listing, CancellationToken), EnumError> {
    spawn_listing(options, Backend::Scripted(source))
}

#[cfg(any(test, windows))]
fn spawn_listing(
    options: ListingOptions,
    backend: Backend,
) -> Result<(Listing, CancellationToken), EnumError> {
    let finished = acquire(options.volume.key())?;
    let timeout = network_timeout_budget(options.volume.kind(), options.network_timeout);
    let batch_size = if options.batch_size == 0 {
        DEFAULT_BATCH_SIZE
    } else {
        options.batch_size
    };
    let clock = Arc::new(Clock {
        timeout,
        started: Mutex::new(None),
    });
    let token = CancellationToken::new();
    let (tx, rx) = mpsc::channel();
    let worker_token = token.clone();
    let worker_clock = Arc::clone(&clock);
    let worker_finished = Arc::clone(&finished);

    // Detach the worker. Joining it would wait on a Win32 call that may not return.
    let spawned = thread::Builder::new()
        .name("fx-core-enum".to_string())
        .spawn(move || {
            let _guard = FinishGuard(worker_finished);
            let deadline = worker_clock.start();
            match open_backend(backend) {
                Ok(mut source) => {
                    run_loop(&mut *source, tx, worker_token, deadline, batch_size);
                }
                Err(err) => {
                    let _ = tx.send(ListingEvent::Failed(err));
                }
            }
        });

    if spawned.is_err() {
        finished.store(true, Ordering::Release);
        return Err(EnumError::SpawnFailed);
    }

    Ok((
        Listing {
            rx,
            clock,
            terminal: None,
        },
        token,
    ))
}

#[cfg(any(test, windows))]
fn open_backend(backend: Backend) -> Result<Box<dyn DirectorySource>, EnumError> {
    match backend {
        #[cfg(test)]
        Backend::Scripted(source) => Ok(Box::new(source)),
        #[cfg(windows)]
        Backend::Windows(path) => {
            WindowsSource::open(&path).map(|source| Box::new(source) as Box<dyn DirectorySource>)
        }
    }
}

#[cfg(any(test, windows))]
fn run_loop(
    source: &mut dyn DirectorySource,
    tx: Sender<ListingEvent>,
    cancel: CancellationToken,
    deadline: Option<Instant>,
    batch_size: usize,
) {
    let mut batch = Vec::new();
    loop {
        if cancel.is_cancelled() {
            flush_then(&tx, &mut batch, ListingEvent::Cancelled);
            return;
        }
        if deadline.is_some_and(|limit| Instant::now() >= limit) {
            flush_then(&tx, &mut batch, ListingEvent::TimedOut);
            return;
        }
        match source.next_entry(&cancel, deadline) {
            Ok(Some(entry)) => {
                batch.push(entry);
                if batch.len() >= batch_size
                    && tx
                        .send(ListingEvent::Batch(std::mem::take(&mut batch)))
                        .is_err()
                {
                    return;
                }
            }
            Ok(None) => {
                flush_then(&tx, &mut batch, ListingEvent::Finished);
                return;
            }
            Err(EnumError::Cancelled) => {
                flush_then(&tx, &mut batch, ListingEvent::Cancelled);
                return;
            }
            Err(EnumError::TimedOut) => {
                flush_then(&tx, &mut batch, ListingEvent::TimedOut);
                return;
            }
            Err(err) => {
                flush_then(&tx, &mut batch, ListingEvent::Failed(err));
                return;
            }
        }
    }
}

#[cfg(any(test, windows))]
fn flush_then(tx: &Sender<ListingEvent>, batch: &mut Vec<FileEntry>, terminal: ListingEvent) {
    if !batch.is_empty() && tx.send(ListingEvent::Batch(std::mem::take(batch))).is_err() {
        return;
    }
    let _ = tx.send(terminal);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::VolumeKind;
    use std::sync::atomic::Ordering;
    use std::time::SystemTime;

    fn sample(name: &str) -> FileEntry {
        FileEntry {
            name: name.to_string(),
            attributes: 0,
            size: 0,
            created: SystemTime::UNIX_EPOCH,
            last_write: SystemTime::UNIX_EPOCH,
            is_directory: false,
            is_reparse_point: false,
        }
    }

    fn options(
        key: &str,
        kind: VolumeKind,
        batch_size: usize,
        network_timeout: Option<Duration>,
    ) -> ListingOptions {
        let volume = match kind {
            VolumeKind::Local => VolumeId::local(key),
            VolumeKind::Network => VolumeId::network(key),
        };
        ListingOptions {
            volume,
            directory: Path::new("C:\\data"),
            batch_size,
            network_timeout,
        }
    }

    fn drive(listing: &mut Listing, limit: Duration) -> Vec<ListingEvent> {
        let start = Instant::now();
        let mut events = Vec::new();
        loop {
            match listing.try_recv() {
                TryEvent::Ready(event) => {
                    let done = is_terminal(&event);
                    events.push(event);
                    if done {
                        return events;
                    }
                }
                TryEvent::Pending => {
                    if start.elapsed() > limit {
                        panic!("listing did not finish within {limit:?}: {events:?}");
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    #[test]
    fn local_has_no_timeout_and_network_defaults_to_two_seconds() {
        assert_eq!(network_timeout_budget(VolumeKind::Local, None), None);
        assert_eq!(
            network_timeout_budget(VolumeKind::Local, Some(Duration::from_millis(1))),
            None
        );
        assert_eq!(
            network_timeout_budget(VolumeKind::Network, None),
            Some(Duration::from_secs(2))
        );
        assert_eq!(
            network_timeout_budget(VolumeKind::Network, Some(Duration::from_millis(40))),
            Some(Duration::from_millis(40))
        );
    }

    #[test]
    fn cancelling_one_volume_does_not_delay_another() {
        let (mut slow, token) = enumerate_scripted(
            options(
                "cancel-slow",
                VolumeKind::Network,
                8,
                Some(Duration::from_secs(2)),
            ),
            ScriptedSource::hang(),
        )
        .unwrap();
        let started = Instant::now();
        let (mut fast, _) = enumerate_scripted(
            options("cancel-fast", VolumeKind::Local, 8, None),
            ScriptedSource::entries(vec![(sample("b"), Duration::ZERO)]),
        )
        .unwrap();
        token.cancel();

        let fast_events = drive(&mut fast, Duration::from_secs(1));
        let fast_at = started.elapsed();
        let slow_events = drive(&mut slow, Duration::from_secs(1));

        assert!(
            fast_at < Duration::from_millis(400),
            "fast volume waited {fast_at:?}"
        );
        assert!(fast_events
            .iter()
            .any(|event| matches!(event, ListingEvent::Finished)));
        assert!(slow_events
            .iter()
            .any(|event| matches!(event, ListingEvent::Cancelled)));
        assert!(!slow_events
            .iter()
            .any(|event| matches!(event, ListingEvent::TimedOut)));
    }

    #[test]
    fn timing_out_one_volume_does_not_delay_another() {
        let timeout = Duration::from_millis(250);
        let started = Instant::now();
        let (mut slow, _) = enumerate_scripted(
            options("timeout-slow", VolumeKind::Network, 8, Some(timeout)),
            ScriptedSource::hang(),
        )
        .unwrap();
        let (mut fast, _) = enumerate_scripted(
            options("timeout-fast", VolumeKind::Local, 8, None),
            ScriptedSource::entries(vec![(sample("b"), Duration::ZERO)]),
        )
        .unwrap();

        let fast_events = drive(&mut fast, Duration::from_secs(1));
        let fast_at = started.elapsed();
        assert!(
            fast_at < timeout,
            "fast volume finished only after the slow timeout ({fast_at:?})"
        );
        assert!(fast_events
            .iter()
            .any(|event| matches!(event, ListingEvent::Finished)));

        let slow_events = drive(&mut slow, Duration::from_secs(2));
        assert!(slow_events
            .iter()
            .any(|event| matches!(event, ListingEvent::TimedOut)));
        assert!(started.elapsed() >= timeout);
    }

    #[test]
    fn local_listing_ignores_the_network_timeout() {
        let (mut listing, _) = enumerate_scripted(
            options(
                "local-ignores-timeout",
                VolumeKind::Local,
                8,
                Some(Duration::from_millis(40)),
            ),
            ScriptedSource::entries(vec![
                (sample("a"), Duration::from_millis(30)),
                (sample("b"), Duration::from_millis(30)),
                (sample("c"), Duration::from_millis(30)),
            ]),
        )
        .unwrap();
        let events = drive(&mut listing, Duration::from_secs(2));
        let names: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                ListingEvent::Batch(entries) => {
                    Some(entries.iter().map(|entry| entry.name.clone()))
                }
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(names, ["a", "b", "c"]);
        assert!(events
            .iter()
            .any(|event| matches!(event, ListingEvent::Finished)));
        assert!(!events
            .iter()
            .any(|event| matches!(event, ListingEvent::TimedOut)));
    }

    #[test]
    fn first_batch_arrives_before_enumeration_finishes() {
        let source = ScriptedSource::entries(vec![
            (sample("a"), Duration::ZERO),
            (sample("b"), Duration::ZERO),
            (sample("c"), Duration::from_millis(400)),
            (sample("d"), Duration::ZERO),
        ]);
        let remaining = source.remaining();
        let (mut listing, _) =
            enumerate_scripted(options("first-batch", VolumeKind::Local, 2, None), source).unwrap();

        let start = Instant::now();
        let first = loop {
            match listing.try_recv() {
                TryEvent::Ready(event) => break event,
                TryEvent::Pending => {
                    if start.elapsed() > Duration::from_millis(200) {
                        panic!("first batch was not sent before the slow entries");
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            }
        };
        match first {
            ListingEvent::Batch(entries) => {
                assert_eq!(entries.len(), 2);
                assert!(
                    remaining.load(Ordering::Acquire) >= 1,
                    "source was already exhausted"
                );
            }
            other => panic!("expected a batch, got {other:?}"),
        }
    }

    #[test]
    fn second_listing_on_a_busy_volume_does_not_start_another_worker() {
        let (mut listing, token) = enumerate_scripted(
            options(
                "busy-volume",
                VolumeKind::Network,
                8,
                Some(Duration::from_secs(2)),
            ),
            ScriptedSource::hang(),
        )
        .unwrap();
        let busy = enumerate_scripted(
            options(
                "busy-volume",
                VolumeKind::Network,
                8,
                Some(Duration::from_millis(50)),
            ),
            ScriptedSource::entries(vec![(sample("nope"), Duration::ZERO)]),
        );
        assert!(matches!(busy, Err(EnumError::VolumeBusy)));

        token.cancel();
        let events = drive(&mut listing, Duration::from_secs(1));
        assert!(events
            .iter()
            .any(|event| matches!(event, ListingEvent::Cancelled)));

        let mut restarted = None;
        for _ in 0..50 {
            match enumerate_scripted(
                options("busy-volume", VolumeKind::Local, 8, None),
                ScriptedSource::entries(vec![(sample("later"), Duration::ZERO)]),
            ) {
                Err(EnumError::VolumeBusy) => thread::sleep(Duration::from_millis(10)),
                other => {
                    restarted = Some(other);
                    break;
                }
            }
        }
        let (mut listing, _) = restarted
            .expect("worker did not exit")
            .expect("restart failed");
        let events = drive(&mut listing, Duration::from_secs(1));
        assert!(events
            .iter()
            .any(|event| matches!(event, ListingEvent::Finished)));
    }

    #[test]
    #[cfg(not(windows))]
    fn windows_enumerate_reports_windows_only() {
        let started = enumerate(options("win-only", VolumeKind::Local, 0, None));
        assert!(matches!(started, Err(EnumError::WindowsOnly)));
    }
}
