//! Lazy Windows network neighborhood.
//!
//! Loading this module does not enumerate anything. [`browse`] starts one
//! detached worker for a parent. The root key is `"Network"`. A parent UNC uses
//! that UNC as its key. A second browse of a busy key returns
//! [`crate::EnumError::VolumeBusy`] and does not start another worker.
//!
//! Every browse has a [`crate::DEFAULT_NETWORK_TIMEOUT`] deadline starting when
//! its worker begins. [`NetworkBrowse::try_recv`] uses that same instant, so it
//! does not report a timeout before the worker runs. Local disks are not
//! involved. [`NetworkBrowse::try_recv`] never blocks. Dropping a browse does
//! not join the worker. Results are not stored in the listing cache.
//!
//! A place is either a container, which expands through another browse, or a
//! connectable share with a UNC the shell can open as a directory.

#[cfg(any(test, windows))]
use std::collections::HashMap;
#[cfg(any(test, windows))]
use std::collections::VecDeque;
#[cfg(any(test, windows))]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(any(test, windows))]
use std::sync::mpsc::Sender;
use std::sync::mpsc::{Receiver, TryRecvError};
#[cfg(any(test, windows))]
use std::sync::LazyLock;
use std::sync::{Arc, Mutex};
#[cfg(any(test, windows))]
use std::thread;
use std::time::Instant;

use crate::cancel::CancellationToken;
use crate::kernel::EnumError;
use crate::DEFAULT_NETWORK_TIMEOUT;

#[cfg(any(test, windows))]
use crate::DEFAULT_BATCH_SIZE;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{
    ERROR_INVALID_DATA, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, NO_ERROR,
};
#[cfg(windows)]
use windows_sys::Win32::NetworkManagement::WNet::{
    WNetCloseEnum, WNetEnumResourceW, WNetOpenEnumW, NETRESOURCEW, RESOURCETYPE_DISK,
    RESOURCEUSAGE_CONNECTABLE, RESOURCEUSAGE_CONTAINER, RESOURCE_GLOBALNET,
};

/// One network place.
///
/// A container expands through another browse and is not listed as a directory.
/// A connectable share has a UNC the shell can open as a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkPlace {
    /// Text shown for this place.
    pub name: String,
    /// Remote name, when the provider supplied one.
    pub unc: Option<String>,
    /// This place expands to more network places.
    pub container: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseEvent {
    Batch(Vec<NetworkPlace>),
    Finished,
    Cancelled,
    TimedOut,
    Failed(EnumError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TryBrowse {
    Ready(BrowseEvent),
    Pending,
}

/// Shared with the worker. `started` stays unset until the worker begins, so
/// [`NetworkBrowse::try_recv`] cannot synthesize a timeout before then.
struct Clock {
    started: Mutex<Option<Instant>>,
}

impl Clock {
    #[cfg(any(test, windows))]
    fn start(&self) -> Instant {
        let now = Instant::now();
        let mut started = self.started.lock().unwrap_or_else(|err| err.into_inner());
        *started = Some(now);
        now + DEFAULT_NETWORK_TIMEOUT
    }

    fn passed(&self) -> bool {
        let started = self.started.lock().unwrap_or_else(|err| err.into_inner());
        started.is_some_and(|start| Instant::now() >= start + DEFAULT_NETWORK_TIMEOUT)
    }
}

/// Non-blocking view of one network browse. Dropping it does not join the worker.
pub struct NetworkBrowse {
    rx: Receiver<BrowseEvent>,
    clock: Arc<Clock>,
    terminal: Option<BrowseEvent>,
    token: CancellationToken,
}

impl NetworkBrowse {
    /// Returns the next event. Never waits.
    ///
    /// The deadline starts when the worker does. A deadline that passes while
    /// the worker is inside one blocking call still reports
    /// [`BrowseEvent::TimedOut`] from here, and later calls keep reporting it.
    pub fn try_recv(&mut self) -> TryBrowse {
        if let Some(terminal) = &self.terminal {
            return TryBrowse::Ready(terminal.clone());
        }
        match self.rx.try_recv() {
            Ok(event) => {
                if is_terminal(&event) {
                    self.terminal = Some(event.clone());
                }
                TryBrowse::Ready(event)
            }
            Err(TryRecvError::Empty) => {
                if self.clock.passed() {
                    self.finish(BrowseEvent::TimedOut)
                } else {
                    TryBrowse::Pending
                }
            }
            Err(TryRecvError::Disconnected) => {
                if self.clock.passed() {
                    self.finish(BrowseEvent::TimedOut)
                } else {
                    self.finish(BrowseEvent::Failed(EnumError::WorkerStopped))
                }
            }
        }
    }

    /// Cancel this browse. The worker observes it between places.
    pub fn cancel(&self) {
        self.token.cancel();
    }

    fn finish(&mut self, event: BrowseEvent) -> TryBrowse {
        self.terminal = Some(event.clone());
        TryBrowse::Ready(event)
    }
}

fn is_terminal(event: &BrowseEvent) -> bool {
    matches!(
        event,
        BrowseEvent::Finished
            | BrowseEvent::Cancelled
            | BrowseEvent::TimedOut
            | BrowseEvent::Failed(_)
    )
}

/// Start a browse. `parent` `None` is the Network root.
///
/// This does not wait. On Windows the calling thread does not call WNet;
/// only the detached worker does.
pub(crate) fn browse(parent: Option<&str>) -> Result<NetworkBrowse, EnumError> {
    start_browse(parent)
}

#[cfg(windows)]
fn start_browse(parent: Option<&str>) -> Result<NetworkBrowse, EnumError> {
    spawn(
        parent_key(parent),
        Backend::Windows(parent.map(str::to_string)),
    )
}

#[cfg(not(windows))]
fn start_browse(_parent: Option<&str>) -> Result<NetworkBrowse, EnumError> {
    Err(EnumError::WindowsOnly)
}

#[cfg(test)]
pub(crate) fn browse_scripted(
    parent: Option<&str>,
    source: ScriptedPlaces,
) -> Result<NetworkBrowse, EnumError> {
    spawn(parent_key(parent), Backend::Scripted(source))
}

#[cfg(any(test, windows))]
fn parent_key(parent: Option<&str>) -> String {
    parent.unwrap_or("Network").to_string()
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

#[cfg(any(test, windows))]
enum Backend {
    #[cfg(test)]
    Scripted(ScriptedPlaces),
    #[cfg(windows)]
    Windows(Option<String>),
}

#[cfg(any(test, windows))]
fn spawn(key: String, backend: Backend) -> Result<NetworkBrowse, EnumError> {
    let finished = acquire(&key)?;
    let clock = Arc::new(Clock {
        started: Mutex::new(None),
    });
    let token = CancellationToken::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let worker_token = token.clone();
    let worker_clock = Arc::clone(&clock);
    let worker_finished = Arc::clone(&finished);

    // Detach the worker. Joining it would wait on a WNet call that may not return.
    let spawned = thread::Builder::new()
        .name("fx-core-net".to_string())
        .spawn(move || {
            let _guard = FinishGuard(worker_finished);
            // Same instant `try_recv` observes. Time spent waiting to be scheduled
            // is not part of the budget, and a result produced once the worker
            // runs is not discarded as already timed out.
            let deadline = worker_clock.start();
            match open_backend(backend) {
                Ok(mut source) => {
                    run_loop(&mut *source, tx, worker_token, deadline, DEFAULT_BATCH_SIZE);
                }
                Err(err) => {
                    let _ = tx.send(BrowseEvent::Failed(err));
                }
            }
        });

    if spawned.is_err() {
        finished.store(true, Ordering::Release);
        return Err(EnumError::SpawnFailed);
    }

    Ok(NetworkBrowse {
        rx,
        clock,
        terminal: None,
        token,
    })
}

#[cfg(any(test, windows))]
trait NetworkSource {
    fn next_place(
        &mut self,
        cancel: &CancellationToken,
        deadline: Instant,
    ) -> Result<Option<NetworkPlace>, EnumError>;
}

#[cfg(any(test, windows))]
fn open_backend(backend: Backend) -> Result<Box<dyn NetworkSource>, EnumError> {
    match backend {
        #[cfg(test)]
        Backend::Scripted(source) => Ok(Box::new(source)),
        #[cfg(windows)]
        Backend::Windows(parent) => WindowsNetwork::open(parent.as_deref())
            .map(|source| Box::new(source) as Box<dyn NetworkSource>),
    }
}

#[cfg(any(test, windows))]
fn run_loop(
    source: &mut dyn NetworkSource,
    tx: Sender<BrowseEvent>,
    cancel: CancellationToken,
    deadline: Instant,
    batch_size: usize,
) {
    let mut batch = Vec::new();
    loop {
        if cancel.is_cancelled() {
            flush_then(&tx, &mut batch, BrowseEvent::Cancelled);
            return;
        }
        // The source applies the deadline. Places already copied out of an enum
        // buffer must be pulled before TimedOut, or that successful read is dropped.
        match source.next_place(&cancel, deadline) {
            Ok(Some(place)) => {
                batch.push(place);
                if batch.len() >= batch_size
                    && tx
                        .send(BrowseEvent::Batch(std::mem::take(&mut batch)))
                        .is_err()
                {
                    return;
                }
            }
            Ok(None) => {
                flush_then(&tx, &mut batch, BrowseEvent::Finished);
                return;
            }
            Err(EnumError::Cancelled) => {
                flush_then(&tx, &mut batch, BrowseEvent::Cancelled);
                return;
            }
            Err(EnumError::TimedOut) => {
                flush_then(&tx, &mut batch, BrowseEvent::TimedOut);
                return;
            }
            Err(err) => {
                flush_then(&tx, &mut batch, BrowseEvent::Failed(err));
                return;
            }
        }
    }
}

#[cfg(any(test, windows))]
fn flush_then(tx: &Sender<BrowseEvent>, batch: &mut Vec<NetworkPlace>, terminal: BrowseEvent) {
    if !batch.is_empty() && tx.send(BrowseEvent::Batch(std::mem::take(batch))).is_err() {
        return;
    }
    let _ = tx.send(terminal);
}

#[cfg(windows)]
const ENUM_BUFFER_BYTES: usize = 16 * 1024;

#[cfg(windows)]
const MAX_ENUM_BUFFER_BYTES: usize = 16 * 1024 * 1024;

/// Production neighborhood reader. `WNetOpenEnumW` and `WNetEnumResourceW` run
/// on the worker only. A single call cannot be preempted; the caller observes
/// the deadline through [`NetworkBrowse::try_recv`].
#[cfg(windows)]
struct WindowsNetwork {
    handle: windows_sys::Win32::Foundation::HANDLE,
    storage: Vec<u64>,
    pending: VecDeque<NetworkPlace>,
    exhausted: bool,
}

#[cfg(windows)]
impl WindowsNetwork {
    fn open(parent: Option<&str>) -> Result<Self, EnumError> {
        let mut remote = Vec::<u16>::new();
        let mut resource = NETRESOURCEW {
            dwScope: RESOURCE_GLOBALNET,
            dwType: RESOURCETYPE_DISK,
            dwDisplayType: 0,
            dwUsage: RESOURCEUSAGE_CONTAINER,
            lpLocalName: std::ptr::null_mut(),
            lpRemoteName: std::ptr::null_mut(),
            lpComment: std::ptr::null_mut(),
            lpProvider: std::ptr::null_mut(),
        };
        let parent_ptr: *const NETRESOURCEW = if let Some(unc) = parent {
            remote.extend(unc.encode_utf16());
            remote.push(0);
            resource.lpRemoteName = remote.as_mut_ptr();
            &resource
        } else {
            std::ptr::null()
        };

        let mut handle = std::ptr::null_mut();
        let status = unsafe {
            WNetOpenEnumW(
                RESOURCE_GLOBALNET,
                RESOURCETYPE_DISK,
                RESOURCEUSAGE_CONTAINER | RESOURCEUSAGE_CONNECTABLE,
                parent_ptr,
                &mut handle,
            )
        };
        if status != NO_ERROR {
            return Err(EnumError::Io { code: status });
        }
        let mut storage = Vec::new();
        grow_storage(&mut storage, ENUM_BUFFER_BYTES);
        Ok(Self {
            handle,
            storage,
            pending: VecDeque::new(),
            exhausted: false,
        })
    }

    fn fill(&mut self, cancel: &CancellationToken, deadline: Instant) -> Result<(), EnumError> {
        loop {
            if cancel.is_cancelled() {
                return Err(EnumError::Cancelled);
            }
            // Each retry is another blocking call.
            if Instant::now() >= deadline {
                return Err(EnumError::TimedOut);
            }
            let mut count = u32::MAX;
            let mut bytes = u32::try_from(
                self.storage
                    .len()
                    .saturating_mul(std::mem::size_of::<u64>()),
            )
            .unwrap_or(u32::MAX);
            let handle = self.handle;
            let buffer = self.storage.as_mut_ptr().cast::<std::ffi::c_void>();
            let status = unsafe { WNetEnumResourceW(handle, &mut count, buffer, &mut bytes) };
            if status == ERROR_MORE_DATA {
                let required = usize::try_from(bytes).unwrap_or(usize::MAX);
                let current = self
                    .storage
                    .len()
                    .saturating_mul(std::mem::size_of::<u64>());
                let next = if required > current {
                    required
                } else {
                    current.saturating_mul(2).max(current.saturating_add(4096))
                };
                if next > MAX_ENUM_BUFFER_BYTES || next <= current {
                    self.exhausted = true;
                    return Err(EnumError::Io { code: status });
                }
                grow_storage(&mut self.storage, next);
                continue;
            }
            if status == ERROR_NO_MORE_ITEMS {
                self.exhausted = true;
                return Ok(());
            }
            if status != NO_ERROR {
                self.exhausted = true;
                return Err(EnumError::Io { code: status });
            }
            if count == 0 {
                self.exhausted = true;
                return Ok(());
            }
            self.capture(count)?;
            return Ok(());
        }
    }

    fn capture(&mut self, count: u32) -> Result<(), EnumError> {
        let bytes = self
            .storage
            .len()
            .saturating_mul(std::mem::size_of::<u64>());
        let entry = std::mem::size_of::<NETRESOURCEW>();
        let count = usize::try_from(count).unwrap_or(usize::MAX);
        if count > bytes / entry {
            return Err(EnumError::Io {
                code: ERROR_INVALID_DATA,
            });
        }
        let base = self.storage.as_ptr().cast::<NETRESOURCEW>();
        for index in 0..count {
            // Strings live in `storage` until the next `WNetEnumResourceW`. Copy them now.
            let resource = unsafe { base.add(index).read_unaligned() };
            if let Some(place) = place_from(&resource) {
                self.pending.push_back(place);
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
impl NetworkSource for WindowsNetwork {
    fn next_place(
        &mut self,
        cancel: &CancellationToken,
        deadline: Instant,
    ) -> Result<Option<NetworkPlace>, EnumError> {
        loop {
            if cancel.is_cancelled() {
                return Err(EnumError::Cancelled);
            }
            // Already copied out of the enum buffer. Returning them does not
            // block, and dropping them would discard a successful read.
            if let Some(place) = self.pending.pop_front() {
                return Ok(Some(place));
            }
            // A completed enum is Finished even if the deadline passed during
            // the call that observed the end.
            if self.exhausted {
                return Ok(None);
            }
            if Instant::now() >= deadline {
                return Err(EnumError::TimedOut);
            }
            self.fill(cancel, deadline)?;
        }
    }
}

#[cfg(windows)]
impl Drop for WindowsNetwork {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                WNetCloseEnum(self.handle);
            }
            self.handle = std::ptr::null_mut();
        }
    }
}

#[cfg(windows)]
fn grow_storage(storage: &mut Vec<u64>, min_bytes: usize) {
    let words = min_bytes.div_ceil(std::mem::size_of::<u64>());
    if storage.len() < words {
        storage.resize(words, 0);
    }
}

#[cfg(windows)]
fn place_from(resource: &NETRESOURCEW) -> Option<NetworkPlace> {
    let unc = decode_pwstr(resource.lpRemoteName);
    let name = unc
        .clone()
        .or_else(|| decode_pwstr(resource.lpProvider))
        .or_else(|| decode_pwstr(resource.lpComment))?;
    let container = (resource.dwUsage & RESOURCEUSAGE_CONTAINER) != 0;
    Some(NetworkPlace {
        name,
        unc,
        container,
    })
}

/// `NETRESOURCEW` has no display-name field. A missing or empty string is skipped.
#[cfg(windows)]
fn decode_pwstr(ptr: *const u16) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    const MAX_UNITS: usize = 32_768;
    let mut len = 0usize;
    unsafe {
        while len < MAX_UNITS && *ptr.add(len) != 0 {
            len += 1;
        }
    }
    if len == 0 {
        return None;
    }
    let units = unsafe { std::slice::from_raw_parts(ptr, len) };
    Some(String::from_utf16_lossy(units))
}

#[cfg(test)]
#[derive(Debug)]
enum ScriptStep {
    Place(NetworkPlace),
    Delay(std::time::Duration),
    Hang,
}

/// Test double for [`NetworkSource`]. Not a second browse design.
#[cfg(test)]
pub(crate) struct ScriptedPlaces {
    steps: VecDeque<ScriptStep>,
}

#[cfg(test)]
impl ScriptedPlaces {
    pub(crate) fn places(items: Vec<NetworkPlace>) -> Self {
        Self {
            steps: items.into_iter().map(ScriptStep::Place).collect(),
        }
    }

    pub(crate) fn hang() -> Self {
        Self {
            steps: VecDeque::from([ScriptStep::Hang]),
        }
    }

    /// Insert `pause` immediately before `index`, then the remaining places.
    fn pause_before(index: usize, pause: std::time::Duration, items: Vec<NetworkPlace>) -> Self {
        let mut steps = VecDeque::new();
        for (at, place) in items.into_iter().enumerate() {
            if at == index {
                steps.push_back(ScriptStep::Delay(pause));
            }
            steps.push_back(ScriptStep::Place(place));
        }
        Self { steps }
    }

    fn check(cancel: &CancellationToken, deadline: Instant) -> Result<(), EnumError> {
        if cancel.is_cancelled() {
            return Err(EnumError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(EnumError::TimedOut);
        }
        Ok(())
    }

    fn pause(
        &self,
        total: std::time::Duration,
        cancel: &CancellationToken,
        deadline: Instant,
    ) -> Result<(), EnumError> {
        let start = Instant::now();
        while start.elapsed() < total {
            Self::check(cancel, deadline)?;
            let slice =
                std::time::Duration::from_millis(5).min(total.saturating_sub(start.elapsed()));
            if slice.is_zero() {
                break;
            }
            thread::sleep(slice);
        }
        Self::check(cancel, deadline)
    }
}

#[cfg(test)]
impl NetworkSource for ScriptedPlaces {
    fn next_place(
        &mut self,
        cancel: &CancellationToken,
        deadline: Instant,
    ) -> Result<Option<NetworkPlace>, EnumError> {
        loop {
            Self::check(cancel, deadline)?;
            match self.steps.pop_front() {
                None => return Ok(None),
                Some(ScriptStep::Hang) => {
                    self.steps.push_front(ScriptStep::Hang);
                    thread::sleep(std::time::Duration::from_millis(5));
                }
                Some(ScriptStep::Delay(total)) => self.pause(total, cancel, deadline)?,
                Some(ScriptStep::Place(place)) => return Ok(Some(place)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    fn sample(name: &str, unc: Option<&str>, container: bool) -> NetworkPlace {
        NetworkPlace {
            name: name.to_string(),
            unc: unc.map(str::to_string),
            container,
        }
    }

    fn drive(browse: &mut NetworkBrowse, limit: Duration) -> Vec<BrowseEvent> {
        let start = Instant::now();
        let mut events = Vec::new();
        loop {
            match browse.try_recv() {
                TryBrowse::Ready(event) => {
                    let done = is_terminal(&event);
                    events.push(event);
                    if done {
                        return events;
                    }
                }
                TryBrowse::Pending => {
                    if start.elapsed() > limit {
                        panic!("browse did not finish within {limit:?}: {events:?}");
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    #[test]
    fn browse_is_not_started_by_helpers() {
        let prepared = ScriptedPlaces::places(vec![sample("server", Some(r"\\server"), true)]);
        #[cfg(not(windows))]
        {
            let started = Instant::now();
            assert!(matches!(browse(None), Err(EnumError::WindowsOnly)));
            assert!(matches!(
                browse(Some(r"\\server\share")),
                Err(EnumError::WindowsOnly)
            ));
            assert!(
                started.elapsed() < Duration::from_millis(50),
                "browse blocked for {:?}",
                started.elapsed()
            );
        }
        let mut listing = browse_scripted(Some(r"\\helper-does-not-start"), prepared).unwrap();
        let started = Instant::now();
        let polled = listing.try_recv();
        assert!(
            started.elapsed() < Duration::from_millis(50),
            "try_recv blocked for {:?}",
            started.elapsed()
        );
        let events = match polled {
            TryBrowse::Ready(event) => {
                let mut events = vec![event];
                if !is_terminal(events.last().expect("event")) {
                    events.extend(drive(&mut listing, Duration::from_secs(1)));
                }
                events
            }
            TryBrowse::Pending => drive(&mut listing, Duration::from_secs(1)),
        };
        assert_eq!(
            events,
            vec![
                BrowseEvent::Batch(vec![sample("server", Some(r"\\server"), true)]),
                BrowseEvent::Finished,
            ]
        );
        let again = Instant::now();
        assert_eq!(listing.try_recv(), TryBrowse::Ready(BrowseEvent::Finished));
        assert!(
            again.elapsed() < Duration::from_millis(50),
            "try_recv blocked for {:?}",
            again.elapsed()
        );
    }

    #[test]
    fn scripted_listing_returns_a_batch_then_finished() {
        let mut listing = browse_scripted(
            Some(r"\\scripted-parent"),
            ScriptedPlaces::places(vec![
                sample("share", Some(r"\\scripted-parent\share"), false),
                sample("workgroup", None, true),
            ]),
        )
        .unwrap();
        let started = Instant::now();
        let events = drive(&mut listing, Duration::from_secs(1));
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "try_recv blocked for {:?}",
            started.elapsed()
        );
        assert_eq!(
            events,
            vec![
                BrowseEvent::Batch(vec![
                    sample("share", Some(r"\\scripted-parent\share"), false),
                    sample("workgroup", None, true),
                ]),
                BrowseEvent::Finished,
            ]
        );
    }

    #[test]
    fn full_batch_is_sent_before_the_rest() {
        let batch = crate::DEFAULT_BATCH_SIZE;
        let items: Vec<_> = (0..(batch + 1))
            .map(|index| sample(&format!("p{index}"), None, true))
            .collect();
        let mut listing = browse_scripted(
            Some(r"\\stream-batch"),
            ScriptedPlaces::pause_before(batch, Duration::from_millis(300), items),
        )
        .unwrap();
        let started = Instant::now();
        let first = loop {
            match listing.try_recv() {
                TryBrowse::Ready(event) => break event,
                TryBrowse::Pending => {
                    if started.elapsed() > Duration::from_millis(200) {
                        panic!("first batch was not sent before the rest");
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            }
        };
        match first {
            BrowseEvent::Batch(places) => assert_eq!(places.len(), batch),
            other => panic!("expected a batch, got {other:?}"),
        }
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "first batch waited {:?}",
            started.elapsed()
        );
        listing.cancel();
    }

    #[test]
    fn second_browse_of_a_busy_parent_returns_volume_busy() {
        let mut root = browse_scripted(None, ScriptedPlaces::hang()).unwrap();
        let again = browse_scripted(
            None,
            ScriptedPlaces::places(vec![sample("nope", None, true)]),
        );
        assert!(matches!(again, Err(EnumError::VolumeBusy)));
        let same_key = browse_scripted(
            Some("Network"),
            ScriptedPlaces::places(vec![sample("nope", None, true)]),
        );
        assert!(
            matches!(same_key, Err(EnumError::VolumeBusy)),
            "root key is the string Network"
        );

        let mut other = browse_scripted(
            Some(r"\\busy-other"),
            ScriptedPlaces::places(vec![sample("ok", Some(r"\\busy-other\share"), false)]),
        )
        .unwrap();
        let other_events = drive(&mut other, Duration::from_secs(1));
        assert_eq!(
            other_events,
            vec![
                BrowseEvent::Batch(vec![sample("ok", Some(r"\\busy-other\share"), false)]),
                BrowseEvent::Finished,
            ]
        );

        root.cancel();
        let root_events = drive(&mut root, Duration::from_secs(1));
        assert!(root_events
            .iter()
            .any(|event| matches!(event, BrowseEvent::Cancelled)));

        let mut restarted = None;
        for _ in 0..50 {
            match browse_scripted(
                None,
                ScriptedPlaces::places(vec![sample("later", None, true)]),
            ) {
                Err(EnumError::VolumeBusy) => thread::sleep(Duration::from_millis(10)),
                other => {
                    restarted = Some(other);
                    break;
                }
            }
        }
        let mut listing = restarted
            .expect("worker did not exit")
            .expect("restart failed");
        let events = drive(&mut listing, Duration::from_secs(1));
        assert_eq!(
            events,
            vec![
                BrowseEvent::Batch(vec![sample("later", None, true)]),
                BrowseEvent::Finished,
            ]
        );
    }

    #[test]
    fn slow_parent_does_not_delay_a_different_parent() {
        let mut slow = browse_scripted(Some(r"\\slow-parent"), ScriptedPlaces::hang()).unwrap();
        let started = Instant::now();
        let mut fast = browse_scripted(
            Some(r"\\fast-parent"),
            ScriptedPlaces::places(vec![sample("share", Some(r"\\fast-parent\share"), false)]),
        )
        .unwrap();
        let fast_events = drive(&mut fast, Duration::from_secs(1));
        let fast_at = started.elapsed();
        slow.cancel();
        let slow_events = drive(&mut slow, Duration::from_secs(1));

        assert!(
            fast_at < Duration::from_millis(400),
            "fast parent waited {fast_at:?}"
        );
        assert_eq!(
            fast_events,
            vec![
                BrowseEvent::Batch(vec![sample("share", Some(r"\\fast-parent\share"), false,)]),
                BrowseEvent::Finished,
            ]
        );
        assert!(slow_events
            .iter()
            .any(|event| matches!(event, BrowseEvent::Cancelled)));
        assert!(!slow_events
            .iter()
            .any(|event| matches!(event, BrowseEvent::TimedOut)));
    }

    #[test]
    fn cancelling_one_parent_does_not_cancel_the_other() {
        let mut slow = browse_scripted(Some(r"\\cancel-slow"), ScriptedPlaces::hang()).unwrap();
        let mut other = browse_scripted(
            Some(r"\\cancel-other"),
            ScriptedPlaces::places(vec![sample("kept", Some(r"\\cancel-other\share"), false)]),
        )
        .unwrap();
        slow.cancel();
        let other_events = drive(&mut other, Duration::from_secs(1));
        let slow_events = drive(&mut slow, Duration::from_secs(1));

        assert_eq!(
            other_events,
            vec![
                BrowseEvent::Batch(vec![sample("kept", Some(r"\\cancel-other\share"), false,)]),
                BrowseEvent::Finished,
            ]
        );
        assert!(slow_events
            .iter()
            .any(|event| matches!(event, BrowseEvent::Cancelled)));
        assert!(!other_events
            .iter()
            .any(|event| matches!(event, BrowseEvent::Cancelled)));
    }

    #[test]
    fn hang_past_the_deadline_returns_timed_out() {
        let started = Instant::now();
        let mut listing =
            browse_scripted(Some(r"\\hang-deadline"), ScriptedPlaces::hang()).unwrap();
        let polled = Instant::now();
        assert!(matches!(listing.try_recv(), TryBrowse::Pending));
        assert!(
            polled.elapsed() < Duration::from_millis(50),
            "try_recv blocked for {:?}",
            polled.elapsed()
        );

        let events = drive(
            &mut listing,
            DEFAULT_NETWORK_TIMEOUT + Duration::from_secs(1),
        );
        let seen = started.elapsed();
        assert_eq!(events, vec![BrowseEvent::TimedOut]);
        assert!(
            seen >= DEFAULT_NETWORK_TIMEOUT,
            "timed out early at {seen:?}"
        );
        assert!(
            seen < DEFAULT_NETWORK_TIMEOUT + Duration::from_millis(500),
            "try_recv waited {seen:?} past the deadline"
        );
        let again = Instant::now();
        assert_eq!(listing.try_recv(), TryBrowse::Ready(BrowseEvent::TimedOut));
        assert!(
            again.elapsed() < Duration::from_millis(50),
            "repeated try_recv blocked for {:?}",
            again.elapsed()
        );
    }

    #[test]
    fn dropping_a_browse_does_not_join_the_worker() {
        let listing = browse_scripted(Some(r"\\drop-hang"), ScriptedPlaces::hang()).unwrap();
        let started = Instant::now();
        drop(listing);
        assert!(
            started.elapsed() < Duration::from_millis(50),
            "drop joined the worker after {:?}",
            started.elapsed()
        );
    }
}
