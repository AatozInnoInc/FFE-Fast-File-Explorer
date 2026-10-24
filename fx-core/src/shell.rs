use std::time::Duration;

use crate::cache::ListingCache;
use crate::enumerate::{ListingOptions, TryEvent};
use crate::kernel::{EnumError, FileEntry, Path, VolumeId, VolumeKind};
use crate::settings::TimeoutSetting;
use crate::DEFAULT_NETWORK_TIMEOUT;

/// In-process door for the Win32 shell.
///
/// This is the only API the shell calls. Enumeration and the listing cache stay behind it.
#[derive(Clone)]
pub struct Shell {
    cache: ListingCache,
    /// Used only for [`VolumeKind::Network`]. Local volumes ignore it.
    network_timeout: Duration,
}

/// One open directory: cached entries, if any, and the live listing behind [`Shell::poll`].
pub struct OpenedDirectory {
    /// Entries already stored for this directory. `None` on a cache miss.
    pub cached: Option<Vec<FileEntry>>,
    live: Result<crate::cache::LiveListing, EnumError>,
}

impl Default for Shell {
    fn default() -> Self {
        Self::new()
    }
}

impl Shell {
    pub fn new() -> Self {
        Self {
            cache: ListingCache::new(),
            network_timeout: DEFAULT_NETWORK_TIMEOUT,
        }
    }

    /// Stores the network-volume deadline. Local volumes still have none.
    ///
    /// This does not enable auto-retry.
    pub fn set_network_timeout(&mut self, setting: TimeoutSetting) {
        self.network_timeout = Duration::from_millis(setting.millis);
    }

    /// Stored network-volume deadline. Local listings do not use it.
    pub fn network_timeout(&self) -> Duration {
        self.network_timeout
    }

    /// Deadline passed to a network volume. `None` for a local volume.
    pub fn network_timeout_for(&self, volume: &VolumeId) -> Option<Duration> {
        match volume.kind() {
            VolumeKind::Network => Some(self.network_timeout),
            VolumeKind::Local => None,
        }
    }

    /// Open `directory` on `volume`.
    ///
    /// When the cache holds this directory, `cached` is those entries and the call returns
    /// without waiting for disk. A live listing still starts on that volume's worker.
    pub fn open(&self, volume: VolumeId, directory: Path) -> Result<OpenedDirectory, EnumError> {
        let network_timeout = self.network_timeout_for(&volume);
        self.open_with(ListingOptions {
            volume,
            directory,
            batch_size: 0,
            network_timeout,
        })
    }

    /// Next listing event. Never blocks. `Err` when this volume's worker was already busy.
    pub fn poll(opened: &mut OpenedDirectory) -> Result<TryEvent, EnumError> {
        match &mut opened.live {
            Ok(live) => Ok(live.try_recv()),
            Err(err) => Err(err.clone()),
        }
    }

    /// Cancel the live listing. A busy volume has no listing to cancel.
    pub fn cancel(opened: &OpenedDirectory) {
        if let Ok(live) = &opened.live {
            live.cancel();
        }
    }

    /// Start a network-neighborhood browse.
    ///
    /// `parent` `None` is the Network root. A UNC browses that container.
    /// This does not wait, does not touch the listing cache, and does not call
    /// WNet on the caller's thread.
    pub fn browse_network(parent: Option<&str>) -> Result<crate::NetworkBrowse, EnumError> {
        crate::network::browse(parent)
    }

    fn open_with(&self, options: ListingOptions) -> Result<OpenedDirectory, EnumError> {
        let opened = self.cache.open(options)?;
        Ok(OpenedDirectory {
            cached: opened.cached,
            live: opened.live,
        })
    }

    #[cfg(test)]
    pub(crate) fn open_scripted(
        &self,
        volume: VolumeId,
        directory: Path,
        source: crate::source::ScriptedSource,
    ) -> Result<OpenedDirectory, EnumError> {
        let opened = self.cache.open_scripted(
            ListingOptions {
                volume,
                directory,
                batch_size: 0,
                network_timeout: None,
            },
            source,
        )?;
        Ok(OpenedDirectory {
            cached: opened.cached,
            live: opened.live,
        })
    }
}

impl OpenedDirectory {
    pub fn poll(&mut self) -> Result<TryEvent, EnumError> {
        Shell::poll(self)
    }

    pub fn cancel(&self) {
        Shell::cancel(self);
    }

    /// The cache returned entries and this volume's worker was already taken.
    ///
    /// [`Self::poll`] repeats [`EnumError::VolumeBusy`]. The cached names are in
    /// [`Self::cached`]. A later `open` starts the revalidation once the worker is free.
    pub fn worker_busy(&self) -> bool {
        matches!(&self.live, Err(EnumError::VolumeBusy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enumerate::ListingEvent;
    use crate::kernel::VolumeKind;
    use crate::source::ScriptedSource;
    use std::thread;
    use std::time::{Duration, Instant, SystemTime};

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

    fn names(entries: &[FileEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.name.as_str()).collect()
    }

    fn volume(key: &str, kind: VolumeKind) -> VolumeId {
        match kind {
            VolumeKind::Local => VolumeId::local(key),
            VolumeKind::Network => VolumeId::network(key),
        }
    }

    fn drive(opened: &mut OpenedDirectory, limit: Duration) -> Vec<ListingEvent> {
        let start = Instant::now();
        let mut events = Vec::new();
        loop {
            match opened.poll().unwrap() {
                TryEvent::Ready(event) => {
                    let done = matches!(
                        event,
                        ListingEvent::Finished
                            | ListingEvent::Cancelled
                            | ListingEvent::TimedOut
                            | ListingEvent::Failed(_)
                    );
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
    fn open_polls_a_miss_until_finished() {
        let shell = Shell::new();
        let mut opened = shell
            .open_scripted(
                volume("shell-miss", VolumeKind::Local),
                Path::new("C:\\data"),
                ScriptedSource::entries(vec![
                    (sample("a"), Duration::ZERO),
                    (sample("b"), Duration::ZERO),
                ]),
            )
            .unwrap();
        assert!(opened.cached.is_none());
        let events = drive(&mut opened, Duration::from_secs(1));
        let got: Vec<&str> = events
            .iter()
            .filter_map(|event| match event {
                ListingEvent::Batch(entries) => {
                    Some(entries.iter().map(|entry| entry.name.as_str()))
                }
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(got, ["a", "b"]);
        assert!(events
            .iter()
            .any(|event| matches!(event, ListingEvent::Finished)));
    }

    #[test]
    fn open_returns_cached_entries_without_waiting() {
        let shell = Shell::new();
        let mut first = shell
            .open_scripted(
                volume("shell-hit", VolumeKind::Local),
                Path::new("C:\\docs"),
                ScriptedSource::entries(vec![
                    (sample("a"), Duration::ZERO),
                    (sample("b"), Duration::ZERO),
                ]),
            )
            .unwrap();
        drive(&mut first, Duration::from_secs(1));

        let started = Instant::now();
        let second = shell
            .open_scripted(
                volume("shell-hit", VolumeKind::Local),
                Path::new("C:\\docs"),
                ScriptedSource::entries(vec![(sample("slow"), Duration::from_millis(400))]),
            )
            .unwrap();
        let elapsed = started.elapsed();
        second.cancel();

        assert!(
            elapsed < Duration::from_millis(150),
            "open waited {elapsed:?}"
        );
        assert_eq!(names(second.cached.as_ref().unwrap()), ["a", "b"]);
    }

    #[test]
    fn cached_hit_on_a_busy_volume_keeps_entries_and_reports_busy() {
        let shell = Shell::new();
        let mut first = shell
            .open_scripted(
                volume("shell-busy-hit", VolumeKind::Local),
                Path::new("C:\\docs"),
                ScriptedSource::entries(vec![(sample("a"), Duration::ZERO)]),
            )
            .unwrap();
        drive(&mut first, Duration::from_secs(1));

        let slow = shell
            .open_scripted(
                volume("shell-busy-hit", VolumeKind::Local),
                Path::new("C:\\docs"),
                ScriptedSource::hang(),
            )
            .unwrap();
        assert!(!slow.worker_busy());
        assert_eq!(names(slow.cached.as_ref().unwrap()), ["a"]);

        let missed = shell.open_scripted(
            volume("shell-busy-hit", VolumeKind::Local),
            Path::new("C:\\other"),
            ScriptedSource::entries(vec![(sample("b"), Duration::ZERO)]),
        );
        assert!(matches!(missed, Err(EnumError::VolumeBusy)));

        let mut hit = shell
            .open_scripted(
                volume("shell-busy-hit", VolumeKind::Local),
                Path::new("C:\\docs"),
                ScriptedSource::entries(vec![(sample("c"), Duration::ZERO)]),
            )
            .unwrap();
        assert!(hit.worker_busy());
        assert_eq!(names(hit.cached.as_ref().unwrap()), ["a"]);
        assert!(matches!(hit.poll(), Err(EnumError::VolumeBusy)));
        slow.cancel();
    }

    #[test]
    fn cancel_reaches_the_live_listing() {
        let shell = Shell::new();
        let mut opened = shell
            .open_scripted(
                volume("shell-cancel", VolumeKind::Network),
                Path::new("\\\\server\\share"),
                ScriptedSource::hang(),
            )
            .unwrap();
        opened.cancel();
        let events = drive(&mut opened, Duration::from_secs(1));
        assert!(events
            .iter()
            .any(|event| matches!(event, ListingEvent::Cancelled)));
    }

    #[cfg(not(windows))]
    #[test]
    fn browse_network_reports_windows_only() {
        let started = Instant::now();
        let result = Shell::browse_network(None);
        assert!(
            started.elapsed() < Duration::from_millis(50),
            "browse_network blocked for {:?}",
            started.elapsed()
        );
        assert!(matches!(result, Err(EnumError::WindowsOnly)));
        assert!(matches!(
            Shell::browse_network(Some(r"\\server\share")),
            Err(EnumError::WindowsOnly)
        ));
    }

    #[test]
    fn network_volume_uses_the_stored_timeout_and_local_does_not() {
        let mut shell = Shell::new();
        assert_eq!(shell.network_timeout_for(&VolumeId::local("C:")), None);
        assert_eq!(
            shell.network_timeout_for(&VolumeId::network(r"\\server\share")),
            Some(crate::DEFAULT_NETWORK_TIMEOUT)
        );
        shell.set_network_timeout(TimeoutSetting { millis: 1_500 });
        assert_eq!(
            shell.network_timeout_for(&VolumeId::network(r"\\server\share")),
            Some(Duration::from_millis(1_500))
        );
        assert_eq!(shell.network_timeout_for(&VolumeId::local("C:")), None);
    }
}
