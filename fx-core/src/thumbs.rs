//! Thumbnail decodes for Rapid Explorer.
//!
//! Loading this module does not start work and does not read a file. [`ThumbCache::new`] is the
//! same: it does not spawn a worker and it does not open a path. [`ThumbCache::request`] is what
//! starts a detached worker, and only when that full path is not cached and not already in flight.
//!
//! [`ThumbCache::new`] fails a decode without opening the path. [`ThumbCache::with_decoder`]
//! runs a caller-supplied decoder on the pool. Finished values are raw RGBA8 buffers keyed by
//! the full path string.
//! Eviction: TODO — define
//!
//! [`ThumbTicket::try_recv`] never blocks. Dropping a ticket does not join the worker;
//! [`ThumbTicket::cancel`] is explicit. The pool size is `min(4, logical core count)`, the
//! thumbnail ceiling in `docs/ARCHITECTURE.md`. This module does not paint, call Win32, or cross
//! a process.

// The shell does not call this module yet. A private `mod thumbs` would otherwise trip
// dead_code on the lib build; tests still run with the lint on.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

#[cfg(test)]
use std::sync::atomic::AtomicUsize;
#[cfg(test)]
use std::time::{Duration, Instant};

/// Upper bound from `docs/ARCHITECTURE.md`: `min(4, logical core count)`.
const MAX_THUMB_WORKERS: usize = 4;

/// Thumbnail worker count: `min(4, logical core count)`.
///
/// This is the thread cap from `docs/ARCHITECTURE.md`.
pub fn thumb_pool_size() -> usize {
    let parallelism = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1);
    parallelism.min(MAX_THUMB_WORKERS)
}

/// Raw RGBA8 pixels. No header and no file wrapper.
///
/// `pixels` is whatever the decoder returned, stored unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// One non-blocking poll of a [`ThumbTicket`].
///
/// [`TryThumb::Ready`], [`TryThumb::Failed`], and [`TryThumb::Cancelled`] are terminal: later
/// polls repeat that same event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TryThumb {
    /// The decode has not finished.
    Pending,
    /// Finished RGBA8 buffer.
    Ready(Rgba),
    /// The decoder failed. The path is not cached.
    Failed,
    /// [`ThumbTicket::cancel`] won before a buffer was published.
    Cancelled,
}

/// Finished RGBA8 thumbnails keyed by the full path string.
///
/// A hit returns the stored buffer and does not start a worker. A miss starts a worker, or
/// returns the ticket for the decode already in flight for that path.
///
/// Eviction: TODO — define. This cache is unbounded for this slice.
pub struct ThumbCache {
    inner: Arc<Inner>,
}

/// Handle for one path's thumbnail.
///
/// Dropping a ticket does not join the worker and does not cancel it. [`ThumbTicket::cancel`]
/// is explicit. Two tickets from [`ThumbCache::request`] for the same in-flight path share one
/// job: cancelling either one cancels that path only.
#[derive(Debug)]
pub struct ThumbTicket {
    job: Arc<Job>,
}

impl ThumbCache {
    /// An empty cache.
    ///
    /// Does not start a worker and does not read a file. The default decoder fails without
    /// opening the path: this slice has no image codec and no Win32 thumbnail provider.
    pub fn new() -> Self {
        let decode: DecodeFn = Arc::new(decode_unavailable);
        Self::from_decode(decode)
    }

    /// Start a decode for `path`, or attach to work that is already running.
    ///
    /// The key is the full path string, with no canonicalization. A cached path returns a ticket
    /// that is already [`TryThumb::Ready`] and does not start a worker. A second request for a
    /// path that is queued or running returns a ticket for that same job and does not start
    /// another worker. A cancelled job is not reused: the next request starts another decode.
    /// At [`thumb_pool_size`], the path is queued and [`ThumbTicket::try_recv`] stays
    /// [`TryThumb::Pending`] until a slot frees. This method does not sleep and does not
    /// read `path`.
    pub fn request(&self, path: &str) -> ThumbTicket {
        let mut sched = lock(&self.inner.sched);
        // A cancelled job stays in the map until its worker leaves the imaging call.
        // Returning that ticket would make the next visit look finished. Replace it.
        let replaced_cancelled = match sched.jobs.get(path) {
            Some(job) if job.is_cancelled() => true,
            Some(job) => {
                let job = Arc::clone(job);
                drop(sched);
                return ThumbTicket::from_job(job);
            }
            None => false,
        };
        if !replaced_cancelled {
            if let Some(rgba) = sched.finished.get(path) {
                let rgba = rgba.clone();
                drop(sched);
                return ThumbTicket::ready(path, rgba);
            }
        }
        let job = Job::new(path.to_string());
        sched.jobs.insert(path.to_string(), Arc::clone(&job));
        // The worker that is still inside the cancelled decode holds its pool slot.
        // Queue so that worker picks this path up when it returns, instead of
        // starting a second decode over the cap.
        let start_now =
            sched.active < self.inner.pool_size && !(replaced_cancelled && sched.active > 0);
        if start_now {
            sched.active += 1;
        } else {
            sched.queue.push_back(Arc::clone(&job));
        }
        drop(sched);
        if start_now {
            spawn_worker(Arc::clone(&self.inner), Arc::clone(&job));
        }
        ThumbTicket::from_job(job)
    }

    #[cfg(test)]
    fn pool_size(&self) -> usize {
        self.inner.pool_size
    }

    /// Test-only cache. `decode` is [`decode_scripted`] in the unit tests.
    #[cfg(test)]
    fn scripted(book: Arc<ScriptBook>, decode: ScriptedDecode) -> Self {
        let decode: DecodeFn =
            Arc::new(move |path: &str, cancel: &CancelFlag| decode(path, &book, cancel));
        Self::from_decode(decode)
    }

    /// `decode` runs on a pool worker. [`ThumbCache::request`] does not call it.
    ///
    /// Cancellation is checked before `decode` and again before the buffer is stored.
    /// A decode that blocks inside `decode` does not observe cancel until it returns.
    pub fn with_decoder<F>(decode: F) -> Self
    where
        F: Fn(&str) -> Result<Rgba, ()> + Send + Sync + 'static,
    {
        let decode: DecodeFn = Arc::new(move |path, cancel| {
            if cancel.is_cancelled() {
                return Err(DecodeFailed);
            }
            match decode(path) {
                Ok(rgba) if !cancel.is_cancelled() => Ok(rgba),
                _ => Err(DecodeFailed),
            }
        });
        Self::from_decode(decode)
    }

    fn from_decode(decode: DecodeFn) -> Self {
        let pool_size = thumb_pool_size();
        debug_assert!(pool_size >= 1);
        Self {
            inner: Arc::new(Inner {
                sched: Mutex::new(Scheduler::default()),
                pool_size,
                decode,
            }),
        }
    }
}

impl Default for ThumbCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ThumbTicket {
    /// The latest event. Never blocks and never sleeps.
    ///
    /// After [`TryThumb::Ready`], [`TryThumb::Failed`], or [`TryThumb::Cancelled`], later calls
    /// repeat that event.
    pub fn try_recv(&self) -> TryThumb {
        self.job.poll()
    }

    /// Cancel this path.
    ///
    /// A queued job is not started. A running job stops publishing a buffer. Another path's
    /// ticket is not affected. A ticket that is already ready or failed stays in that state.
    pub fn cancel(&self) {
        self.job.cancel();
    }

    #[cfg(test)]
    fn same_job(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.job, &other.job)
    }

    fn from_job(job: Arc<Job>) -> Self {
        Self { job }
    }

    fn ready(path: &str, rgba: Rgba) -> Self {
        let job = Job::new(path.to_string());
        let cached = job.settle(Phase::Ready(rgba));
        debug_assert!(cached.is_some());
        Self { job }
    }
}

type DecodeFn = Arc<dyn Fn(&str, &CancelFlag) -> Result<Rgba, DecodeFailed> + Send + Sync>;

#[cfg(test)]
type ScriptedDecode = fn(&str, &ScriptBook, &CancelFlag) -> Result<Rgba, DecodeFailed>;

struct Inner {
    sched: Mutex<Scheduler>,
    pool_size: usize,
    decode: DecodeFn,
}

#[derive(Default)]
struct Scheduler {
    /// Eviction: TODO — define
    finished: HashMap<String, Rgba>,
    jobs: HashMap<String, Arc<Job>>,
    queue: VecDeque<Arc<Job>>,
    active: usize,
}

struct CancelFlag {
    cancelled: AtomicBool,
}

impl CancelFlag {
    fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
        }
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

#[derive(Debug)]
struct Job {
    path: String,
    flag: CancelFlag,
    state: Mutex<Phase>,
}

#[derive(Debug)]
enum Phase {
    Pending,
    Ready(Rgba),
    Failed,
    Cancelled,
}

impl Job {
    fn new(path: String) -> Arc<Self> {
        Arc::new(Self {
            path,
            flag: CancelFlag::new(),
            state: Mutex::new(Phase::Pending),
        })
    }

    fn cancel(&self) {
        self.flag.cancel();
        self.force_cancelled();
    }

    fn force_cancelled(&self) {
        let mut state = lock(&self.state);
        if matches!(*state, Phase::Pending) {
            *state = Phase::Cancelled;
        }
    }

    /// Publishes `phase` unless cancel already won. Returns the buffer when the stored state is
    /// [`Phase::Ready`] so the caller can insert it outside this lock.
    fn settle(&self, phase: Phase) -> Option<Rgba> {
        let mut state = lock(&self.state);
        if self.flag.is_cancelled() {
            if matches!(*state, Phase::Pending) {
                *state = Phase::Cancelled;
            }
            return None;
        }
        if !matches!(*state, Phase::Pending) {
            return None;
        }
        match phase {
            Phase::Ready(rgba) => {
                let cached = rgba.clone();
                *state = Phase::Ready(rgba);
                Some(cached)
            }
            other => {
                *state = other;
                None
            }
        }
    }

    fn poll(&self) -> TryThumb {
        let state = lock(&self.state);
        match &*state {
            Phase::Pending => TryThumb::Pending,
            Phase::Ready(rgba) => TryThumb::Ready(rgba.clone()),
            Phase::Failed => TryThumb::Failed,
            Phase::Cancelled => TryThumb::Cancelled,
        }
    }

    fn is_cancelled(&self) -> bool {
        matches!(*lock(&self.state), Phase::Cancelled)
    }
}

impl std::fmt::Debug for CancelFlag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancelFlag")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

/// Does not open `path`. This slice has no thumbnail codec.
fn decode_unavailable(_path: &str, _cancel: &CancelFlag) -> Result<Rgba, DecodeFailed> {
    Err(DecodeFailed)
}

#[derive(Debug)]
struct DecodeFailed;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

fn spawn_worker(inner: Arc<Inner>, mut job: Arc<Job>) {
    loop {
        let worker_inner = Arc::clone(&inner);
        let worker_job = Arc::clone(&job);
        // Dropping the `JoinHandle` detaches the worker. Tickets do not join it.
        match thread::Builder::new()
            .name("fx-core-thumb".to_string())
            .spawn(move || worker_loop(worker_inner, worker_job))
        {
            Ok(handle) => {
                drop(handle);
                return;
            }
            Err(_) => {
                let _ = job.settle(Phase::Failed);
                match next_job(&inner, &job) {
                    Some(next) => job = next,
                    None => return,
                }
            }
        }
    }
}

fn worker_loop(inner: Arc<Inner>, mut job: Arc<Job>) {
    loop {
        decode_job(&inner, &job);
        match next_job(&inner, &job) {
            Some(next) => job = next,
            None => break,
        }
    }
}

/// The decoder runs outside the scheduler lock, so one slow path cannot stall `request` or
/// `try_recv` for another path.
fn decode_job(inner: &Inner, job: &Job) {
    if job.flag.is_cancelled() {
        job.force_cancelled();
        return;
    }
    let phase = match (inner.decode)(&job.path, &job.flag) {
        Ok(rgba) => Phase::Ready(rgba),
        Err(DecodeFailed) => Phase::Failed,
    };
    if let Some(rgba) = job.settle(phase) {
        lock(&inner.sched).finished.insert(job.path.clone(), rgba);
    }
}

fn next_job(inner: &Inner, finished: &Arc<Job>) -> Option<Arc<Job>> {
    let mut sched = lock(&inner.sched);
    // A newer request may already have replaced a cancelled job for this path.
    if sched
        .jobs
        .get(&finished.path)
        .is_some_and(|current| Arc::ptr_eq(current, finished))
    {
        sched.jobs.remove(&finished.path);
    }
    while let Some(next) = sched.queue.pop_front() {
        if next.flag.is_cancelled() {
            if sched
                .jobs
                .get(&next.path)
                .is_some_and(|current| Arc::ptr_eq(current, &next))
            {
                sched.jobs.remove(&next.path);
            }
            next.force_cancelled();
            continue;
        }
        return Some(next);
    }
    sched.active = sched.active.saturating_sub(1);
    None
}

/// One step of [`decode_scripted`]. Bytes, a delay, a hang, or a failure. Not an image file.
#[cfg(test)]
#[derive(Clone)]
enum ScriptStep {
    Bytes(Rgba),
    Delay(Duration),
    Hang,
    Fail,
}

/// Paths to script steps. `started` counts decoder entries. `max_current` is the most decodes
/// that were inside the script at once.
#[cfg(test)]
struct ScriptBook {
    steps: HashMap<String, Vec<ScriptStep>>,
    started: AtomicUsize,
    current: AtomicUsize,
    max_current: AtomicUsize,
}

#[cfg(test)]
impl ScriptBook {
    fn new(steps: HashMap<String, Vec<ScriptStep>>) -> Self {
        Self {
            steps,
            started: AtomicUsize::new(0),
            current: AtomicUsize::new(0),
            max_current: AtomicUsize::new(0),
        }
    }

    fn started(&self) -> usize {
        self.started.load(Ordering::Acquire)
    }

    fn max_current(&self) -> usize {
        self.max_current.load(Ordering::Acquire)
    }

    fn note_started(&self) {
        self.started.fetch_add(1, Ordering::AcqRel);
    }

    fn enter(&self) -> InFlight<'_> {
        let now = self.current.fetch_add(1, Ordering::AcqRel) + 1;
        self.max_current.fetch_max(now, Ordering::AcqRel);
        InFlight(self)
    }

    fn steps_for(&self, path: &str) -> Vec<ScriptStep> {
        self.steps.get(path).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
struct InFlight<'a>(&'a ScriptBook);

#[cfg(test)]
impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.current.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Scripted decoder used by the unit tests. It does not open `path`.
///
/// [`ScriptStep::Bytes`] returns those pixels. [`ScriptStep::Delay`] waits, then continues.
/// [`ScriptStep::Hang`] waits until `cancel` is set. [`ScriptStep::Fail`] and a cancelled wait
/// return [`DecodeFailed`].
#[cfg(test)]
fn decode_scripted(
    path: &str,
    book: &ScriptBook,
    cancel: &CancelFlag,
) -> Result<Rgba, DecodeFailed> {
    book.note_started();
    let _inflight = book.enter();
    let steps = book.steps_for(path);
    for step in &steps {
        if cancel.is_cancelled() {
            return Err(DecodeFailed);
        }
        match step {
            ScriptStep::Bytes(rgba) => return Ok(rgba.clone()),
            ScriptStep::Delay(duration) => pause(*duration, cancel)?,
            ScriptStep::Hang => {
                while !cancel.is_cancelled() {
                    thread::sleep(Duration::from_millis(5));
                }
                return Err(DecodeFailed);
            }
            ScriptStep::Fail => return Err(DecodeFailed),
        }
    }
    Err(DecodeFailed)
}

#[cfg(test)]
fn pause(total: Duration, cancel: &CancelFlag) -> Result<(), DecodeFailed> {
    let start = Instant::now();
    while start.elapsed() < total {
        if cancel.is_cancelled() {
            return Err(DecodeFailed);
        }
        let remaining = total.saturating_sub(start.elapsed());
        let slice = Duration::from_millis(5).min(remaining);
        if slice.is_zero() {
            break;
        }
        thread::sleep(slice);
    }
    if cancel.is_cancelled() {
        Err(DecodeFailed)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_scripted, thumb_pool_size, Rgba, ScriptBook, ScriptStep, ThumbCache, ThumbTicket,
        TryThumb,
    };
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    fn rgba(pixels: [u8; 4]) -> Rgba {
        Rgba {
            width: 1,
            height: 1,
            pixels: pixels.to_vec(),
        }
    }

    fn scripted_cache<I, S>(entries: I) -> (Arc<ScriptBook>, ThumbCache)
    where
        I: IntoIterator<Item = (S, Vec<ScriptStep>)>,
        S: Into<String>,
    {
        let steps = entries
            .into_iter()
            .map(|(path, steps)| (path.into(), steps))
            .collect();
        let book = Arc::new(ScriptBook::new(steps));
        let cache = ThumbCache::scripted(Arc::clone(&book), decode_scripted);
        (book, cache)
    }

    fn drive(ticket: &ThumbTicket, limit: Duration) -> TryThumb {
        let start = Instant::now();
        loop {
            let event = ticket.try_recv();
            if !matches!(event, TryThumb::Pending) {
                return event;
            }
            if start.elapsed() > limit {
                panic!("thumbnail stayed pending for {limit:?}");
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_until(mut pred: impl FnMut() -> bool, limit: Duration, what: &str) {
        let start = Instant::now();
        while !pred() {
            if start.elapsed() > limit {
                panic!("{what} did not happen within {limit:?}");
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    struct CancelOnDrop<'a>(&'a ThumbTicket);

    impl Drop for CancelOnDrop<'_> {
        fn drop(&mut self) {
            self.0.cancel();
        }
    }

    struct CancelSlice<'a>(&'a [ThumbTicket]);

    impl Drop for CancelSlice<'_> {
        fn drop(&mut self) {
            for ticket in self.0 {
                ticket.cancel();
            }
        }
    }

    #[test]
    fn new_does_not_start_work_or_read_a_file() {
        let constructed = Instant::now();
        drop(ThumbCache::new());
        let construct_elapsed = constructed.elapsed();
        assert!(
            construct_elapsed < Duration::from_millis(50),
            "constructing a cache did work ({construct_elapsed:?})"
        );

        let (book, _scripted) = scripted_cache([(
            r"C:\photos\a.png",
            vec![ScriptStep::Bytes(rgba([1, 2, 3, 4]))],
        )]);
        assert_eq!(book.started(), 0);

        let cache = ThumbCache::new();
        let started = Instant::now();
        let ticket = cache.request(r"C:\does-not-exist\thumb.png");
        let request_elapsed = started.elapsed();
        assert!(
            request_elapsed < Duration::from_millis(200),
            "request waited {request_elapsed:?}"
        );
        assert_eq!(drive(&ticket, Duration::from_secs(1)), TryThumb::Failed);
        assert_eq!(ticket.try_recv(), TryThumb::Failed);
        assert_eq!(book.started(), 0);
    }

    #[test]
    fn with_decoder_publishes_on_a_worker() {
        let cache = ThumbCache::with_decoder(|path| {
            assert_eq!(path, r"C:\photos\a.png");
            Ok(rgba([9, 8, 7, 6]))
        });
        let ticket = cache.request(r"C:\photos\a.png");
        assert_eq!(
            drive(&ticket, Duration::from_secs(1)),
            TryThumb::Ready(rgba([9, 8, 7, 6]))
        );
        assert_eq!(ticket.try_recv(), TryThumb::Ready(rgba([9, 8, 7, 6])));
    }

    #[test]
    fn second_request_for_an_inflight_path_returns_the_existing_ticket() {
        let (book, cache) = scripted_cache([(r"C:\photos\a.png", vec![ScriptStep::Hang])]);
        let first = cache.request(r"C:\photos\a.png");
        let _guard = CancelOnDrop(&first);
        wait_until(
            || book.started() == 1,
            Duration::from_secs(2),
            "first decode",
        );

        let started = Instant::now();
        let second = cache.request(r"C:\photos\a.png");
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(50),
            "second request waited {elapsed:?}"
        );
        assert!(first.same_job(&second));
        assert_eq!(book.started(), 1);
        thread::sleep(Duration::from_millis(50));
        assert_eq!(book.started(), 1);
        assert_eq!(first.try_recv(), TryThumb::Pending);
        assert_eq!(second.try_recv(), TryThumb::Pending);

        second.cancel();
        assert_eq!(first.try_recv(), TryThumb::Cancelled);
        assert_eq!(second.try_recv(), TryThumb::Cancelled);
        assert_eq!(first.try_recv(), TryThumb::Cancelled);
        assert_eq!(book.started(), 1);
    }

    #[test]
    fn request_after_cancel_decodes_the_path_again() {
        let (book, cache) = scripted_cache([(r"C:\photos\a.png", vec![ScriptStep::Hang])]);
        let first = cache.request(r"C:\photos\a.png");
        wait_until(
            || book.started() == 1,
            Duration::from_secs(2),
            "first decode",
        );
        first.cancel();
        assert_eq!(first.try_recv(), TryThumb::Cancelled);

        let second = cache.request(r"C:\photos\a.png");
        let _guard = CancelOnDrop(&second);
        assert!(!first.same_job(&second));
        assert_eq!(second.try_recv(), TryThumb::Pending);
        assert_eq!(first.try_recv(), TryThumb::Cancelled);
        wait_until(
            || book.started() == 2,
            Duration::from_secs(2),
            "decode after cancel",
        );
    }

    #[test]
    fn try_recv_never_blocks_and_repeats_terminal_event() {
        let ready = rgba([1, 2, 3, 4]);
        let (_book, cache) = scripted_cache([
            (
                "delay",
                vec![
                    ScriptStep::Delay(Duration::from_millis(200)),
                    ScriptStep::Bytes(ready.clone()),
                ],
            ),
            ("fail", vec![ScriptStep::Fail]),
            ("hang", vec![ScriptStep::Hang]),
        ]);
        let delayed = cache.request("delay");
        let failed = cache.request("fail");
        let hung = cache.request("hang");
        let _guard = CancelOnDrop(&hung);

        let started = Instant::now();
        assert_eq!(delayed.try_recv(), TryThumb::Pending);
        let poll_elapsed = started.elapsed();
        assert!(
            poll_elapsed < Duration::from_millis(50),
            "try_recv waited {poll_elapsed:?}"
        );

        assert_eq!(
            drive(&delayed, Duration::from_secs(2)),
            TryThumb::Ready(ready.clone())
        );
        assert_eq!(delayed.try_recv(), TryThumb::Ready(ready.clone()));
        delayed.cancel();
        assert_eq!(delayed.try_recv(), TryThumb::Ready(ready));
        assert!(started.elapsed() >= Duration::from_millis(150));

        assert_eq!(drive(&failed, Duration::from_secs(2)), TryThumb::Failed);
        assert_eq!(failed.try_recv(), TryThumb::Failed);
        failed.cancel();
        assert_eq!(failed.try_recv(), TryThumb::Failed);

        let cancel_at = Instant::now();
        hung.cancel();
        assert_eq!(hung.try_recv(), TryThumb::Cancelled);
        assert_eq!(hung.try_recv(), TryThumb::Cancelled);
        let cancel_elapsed = cancel_at.elapsed();
        assert!(
            cancel_elapsed < Duration::from_millis(50),
            "cancel or try_recv waited {cancel_elapsed:?}"
        );
    }

    #[test]
    fn dropping_a_ticket_does_not_join_the_worker() {
        let (book, cache) = scripted_cache([(r"C:\photos\slow.png", vec![ScriptStep::Hang])]);
        let ticket = cache.request(r"C:\photos\slow.png");
        wait_until(
            || book.started() == 1,
            Duration::from_secs(2),
            "worker entered the hang",
        );
        let started = Instant::now();
        drop(ticket);
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(100),
            "drop joined the worker ({elapsed:?})"
        );

        let again = cache.request(r"C:\photos\slow.png");
        let _guard = CancelOnDrop(&again);
        assert_eq!(again.try_recv(), TryThumb::Pending);
        thread::sleep(Duration::from_millis(40));
        assert_eq!(book.started(), 1);
        assert_eq!(again.try_recv(), TryThumb::Pending);
    }

    #[test]
    fn slow_path_does_not_delay_try_recv_or_another_request() {
        let fast = rgba([8, 8, 8, 8]);
        let (_book, cache) = scripted_cache([
            ("slow", vec![ScriptStep::Hang]),
            ("fast", vec![ScriptStep::Bytes(fast.clone())]),
        ]);
        let slow = cache.request("slow");
        let _slow_guard = CancelOnDrop(&slow);

        let poll_at = Instant::now();
        assert_eq!(slow.try_recv(), TryThumb::Pending);
        let poll_elapsed = poll_at.elapsed();
        let request_at = Instant::now();
        let other = cache.request("fast");
        let _other_guard = CancelOnDrop(&other);
        let request_elapsed = request_at.elapsed();
        assert!(
            poll_elapsed < Duration::from_millis(50),
            "try_recv waited {poll_elapsed:?}"
        );
        assert!(
            request_elapsed < Duration::from_millis(200),
            "request waited {request_elapsed:?}"
        );

        if thumb_pool_size() > 1 {
            assert_eq!(drive(&other, Duration::from_secs(1)), TryThumb::Ready(fast));
            assert_eq!(slow.try_recv(), TryThumb::Pending);
        } else {
            assert_eq!(other.try_recv(), TryThumb::Pending);
        }
    }

    #[test]
    fn cancelling_one_path_does_not_cancel_another() {
        let kept = rgba([4, 5, 6, 7]);
        let (_book, cache) = scripted_cache([
            ("a", vec![ScriptStep::Hang]),
            ("b", vec![ScriptStep::Bytes(kept.clone())]),
        ]);
        let a = cache.request("a");
        let b = cache.request("b");
        let _guard_a = CancelOnDrop(&a);
        let _guard_b = CancelOnDrop(&b);

        if thumb_pool_size() > 1 {
            assert_eq!(
                drive(&b, Duration::from_secs(1)),
                TryThumb::Ready(kept.clone())
            );
            a.cancel();
            assert_eq!(a.try_recv(), TryThumb::Cancelled);
            assert_eq!(b.try_recv(), TryThumb::Ready(kept));
        } else {
            assert_eq!(b.try_recv(), TryThumb::Pending);
            a.cancel();
            assert_eq!(a.try_recv(), TryThumb::Cancelled);
            assert_eq!(drive(&b, Duration::from_secs(1)), TryThumb::Ready(kept));
            assert_eq!(a.try_recv(), TryThumb::Cancelled);
        }
        assert_eq!(a.try_recv(), TryThumb::Cancelled);
    }

    #[test]
    fn queued_path_waits_for_a_free_slot_not_an_unrelated_completion() {
        let slots = thumb_pool_size();
        assert!(slots >= 1);
        let fast_px = rgba([7, 7, 7, 7]);
        let mut entries = Vec::new();
        for i in 0..slots {
            entries.push((format!("hang-{i}"), vec![ScriptStep::Hang]));
        }
        entries.push(("fast".to_string(), vec![ScriptStep::Bytes(fast_px.clone())]));
        let (book, cache) = scripted_cache(entries);
        assert_eq!(cache.pool_size(), slots);

        let mut hangs = Vec::new();
        for i in 0..slots {
            hangs.push(cache.request(&format!("hang-{i}")));
        }
        let _hang_guard = CancelSlice(&hangs);
        wait_until(
            || book.started() == slots && book.max_current() == slots,
            Duration::from_secs(2),
            "hangs occupy every pool slot",
        );

        let requested = Instant::now();
        let fast = cache.request("fast");
        let _fast_guard = CancelOnDrop(&fast);
        let request_elapsed = requested.elapsed();
        assert!(
            request_elapsed < Duration::from_millis(50),
            "queued request waited {request_elapsed:?}"
        );
        let hold = Instant::now();
        while hold.elapsed() < Duration::from_millis(50) {
            assert_eq!(fast.try_recv(), TryThumb::Pending);
            assert_eq!(book.started(), slots);
            thread::sleep(Duration::from_millis(5));
        }

        let release = hangs.len() - 1;
        hangs[release].cancel();
        assert_eq!(
            drive(&fast, Duration::from_secs(2)),
            TryThumb::Ready(fast_px.clone())
        );
        assert_eq!(book.started(), slots + 1);
        assert_eq!(book.max_current(), slots);
        if hangs.len() > 1 {
            assert_eq!(hangs[0].try_recv(), TryThumb::Pending);
        }
        assert_eq!(hangs[release].try_recv(), TryThumb::Cancelled);
        assert_eq!(fast.try_recv(), TryThumb::Ready(fast_px));
    }

    #[test]
    fn pool_cap_is_min_of_4_and_available_parallelism() {
        let parallelism = std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(1);
        assert_eq!(thumb_pool_size(), parallelism.min(4));
        let cache = ThumbCache::new();
        assert_eq!(cache.pool_size(), parallelism.min(4));
    }

    #[test]
    fn finished_rgba_is_cached_by_the_full_path_string() {
        let mut entries = Vec::new();
        for i in 0..8u8 {
            let path = format!(r"C:\thumbs\file-{i}.png");
            entries.push((path, vec![ScriptStep::Bytes(rgba([i, 0, 0, 255]))]));
        }
        entries.push((
            r"C:/thumbs/file-0.png".to_string(),
            vec![ScriptStep::Bytes(rgba([9, 9, 9, 9]))],
        ));
        let (book, cache) = scripted_cache(entries);

        let mut tickets = Vec::new();
        for i in 0..8u8 {
            tickets.push(cache.request(&format!(r"C:\thumbs\file-{i}.png")));
        }
        for (i, ticket) in tickets.iter().enumerate() {
            let byte = u8::try_from(i).unwrap();
            assert_eq!(
                drive(ticket, Duration::from_secs(2)),
                TryThumb::Ready(rgba([byte, 0, 0, 255]))
            );
        }
        assert_eq!(book.started(), 8);

        let started = Instant::now();
        let again = cache.request(r"C:\thumbs\file-0.png");
        let first = again.try_recv();
        let elapsed = started.elapsed();
        assert_eq!(first, TryThumb::Ready(rgba([0, 0, 0, 255])));
        assert!(
            elapsed < Duration::from_millis(50),
            "cached request waited {elapsed:?}"
        );
        let last = cache.request(r"C:\thumbs\file-7.png");
        assert_eq!(last.try_recv(), TryThumb::Ready(rgba([7, 0, 0, 255])));
        thread::sleep(Duration::from_millis(30));
        assert_eq!(book.started(), 8);

        let other = cache.request(r"C:/thumbs/file-0.png");
        assert_eq!(
            drive(&other, Duration::from_secs(1)),
            TryThumb::Ready(rgba([9, 9, 9, 9]))
        );
        assert_eq!(book.started(), 9);
        assert_eq!(
            cache.request(r"C:\thumbs\file-0.png").try_recv(),
            TryThumb::Ready(rgba([0, 0, 0, 255]))
        );
        assert_eq!(book.started(), 9);
    }
}
