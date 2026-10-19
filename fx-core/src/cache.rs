use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::cancel::CancellationToken;
use crate::enumerate::{enumerate, Listing, ListingEvent, ListingOptions, TryEvent};
use crate::kernel::{EnumError, FileEntry, Path, VolumeId};

/// In-memory directory listings.
///
/// A hit returns the stored entries and then re-enumerates on that volume's worker.
/// A miss enumerates and stores the listing when it finishes.
///
/// Eviction: TODO — define. This cache is unbounded for this slice.
#[derive(Clone, Default)]
pub struct ListingCache {
    entries: Arc<Mutex<HashMap<String, Vec<FileEntry>>>>,
}

/// A cache lookup plus the listing that fills or revalidates it.
pub struct CachedOpen {
    /// Stored entries when this directory was already cached. `None` on a miss.
    pub cached: Option<Vec<FileEntry>>,
    /// The enumeration in progress. `Err` on a hit whose volume worker is already busy.
    pub live: Result<LiveListing, EnumError>,
}

/// The volume worker's listing. Polling it updates the cache when a finished listing differs.
pub struct LiveListing {
    listing: Listing,
    token: CancellationToken,
    entries: Arc<Mutex<HashMap<String, Vec<FileEntry>>>>,
    key: String,
    collected: Vec<FileEntry>,
    stored: bool,
}

impl ListingCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open `options.directory`, using the Windows enumerator.
    pub fn open(&self, options: ListingOptions) -> Result<CachedOpen, EnumError> {
        self.open_with(options, enumerate)
    }

    /// Entries stored for this volume and directory, if a finished listing was cached.
    pub fn cached_entries(&self, volume: &VolumeId, directory: &Path) -> Option<Vec<FileEntry>> {
        let key = cache_key(volume, directory);
        self.entries
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .get(&key)
            .cloned()
    }

    fn open_with<F>(&self, options: ListingOptions, start: F) -> Result<CachedOpen, EnumError>
    where
        F: FnOnce(ListingOptions) -> Result<(Listing, CancellationToken), EnumError>,
    {
        let key = cache_key(&options.volume, &options.directory);
        let cached = self
            .entries
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .get(&key)
            .cloned();
        if cached.is_some() {
            let live = match start(options) {
                Ok((listing, token)) => Ok(LiveListing::new(
                    listing,
                    token,
                    Arc::clone(&self.entries),
                    key,
                )),
                Err(err) => Err(err),
            };
            return Ok(CachedOpen { cached, live });
        }
        let (listing, token) = start(options)?;
        Ok(CachedOpen {
            cached: None,
            live: Ok(LiveListing::new(
                listing,
                token,
                Arc::clone(&self.entries),
                key,
            )),
        })
    }
}

impl LiveListing {
    fn new(
        listing: Listing,
        token: CancellationToken,
        entries: Arc<Mutex<HashMap<String, Vec<FileEntry>>>>,
        key: String,
    ) -> Self {
        Self {
            listing,
            token,
            entries,
            key,
            collected: Vec::new(),
            stored: false,
        }
    }

    pub fn try_recv(&mut self) -> TryEvent {
        let event = self.listing.try_recv();
        if let TryEvent::Ready(ref ready) = event {
            match ready {
                ListingEvent::Batch(batch) => self.collected.extend(batch.iter().cloned()),
                ListingEvent::Finished => self.store_if_changed(),
                ListingEvent::Cancelled | ListingEvent::TimedOut | ListingEvent::Failed(_) => {}
            }
        }
        event
    }

    pub fn cancel(&self) {
        self.token.cancel();
    }

    fn store_if_changed(&mut self) {
        if self.stored {
            return;
        }
        self.stored = true;
        let mut entries = self.entries.lock().unwrap_or_else(|err| err.into_inner());
        if entries.get(&self.key) == Some(&self.collected) {
            return;
        }
        entries.insert(self.key.clone(), self.collected.clone());
    }
}

fn cache_key(volume: &VolumeId, directory: &Path) -> String {
    format!("{}\n{}", volume.key(), directory.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enumerate::enumerate_scripted;
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

    fn options(key: &str, kind: VolumeKind) -> ListingOptions {
        let volume = match kind {
            VolumeKind::Local => VolumeId::local(key),
            VolumeKind::Network => VolumeId::network(key),
        };
        ListingOptions {
            volume,
            directory: Path::new("C:\\data"),
            batch_size: 8,
            network_timeout: None,
        }
    }

    fn options_timeout(key: &str, timeout: Duration) -> ListingOptions {
        let mut options = options(key, VolumeKind::Network);
        options.network_timeout = Some(timeout);
        options
    }

    fn drive(live: &mut LiveListing, limit: Duration) -> Vec<ListingEvent> {
        let start = Instant::now();
        let mut events = Vec::new();
        loop {
            match live.try_recv() {
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

    fn finish(cache: &ListingCache, key: &str, entries: Vec<FileEntry>) {
        let opened = cache
            .open_scripted(
                options(key, VolumeKind::Local),
                ScriptedSource::entries(
                    entries
                        .into_iter()
                        .map(|entry| (entry, Duration::ZERO))
                        .collect(),
                ),
            )
            .unwrap();
        assert!(opened.cached.is_none());
        let mut live = opened.live.unwrap();
        let events = drive(&mut live, Duration::from_secs(1));
        assert!(events
            .iter()
            .any(|event| matches!(event, ListingEvent::Finished)));
    }

    impl ListingCache {
        fn open_scripted(
            &self,
            options: ListingOptions,
            source: ScriptedSource,
        ) -> Result<CachedOpen, EnumError> {
            self.open_with(options, |options| enumerate_scripted(options, source))
        }
    }

    #[test]
    fn second_request_returns_cached_entries_without_waiting() {
        let cache = ListingCache::new();
        finish(&cache, "cache-hit", vec![sample("a"), sample("b")]);

        let started = Instant::now();
        let opened = cache
            .open_scripted(
                options("cache-hit", VolumeKind::Local),
                ScriptedSource::entries(vec![(sample("slow"), Duration::from_millis(400))]),
            )
            .unwrap();
        let elapsed = started.elapsed();
        opened.live.unwrap().cancel();

        assert!(
            elapsed < Duration::from_millis(150),
            "cache hit waited {elapsed:?}"
        );
        assert_eq!(names(&opened.cached.expect("cached entries")), ["a", "b"]);
    }

    #[test]
    fn revalidation_replaces_cache_when_listing_differs() {
        let cache = ListingCache::new();
        finish(&cache, "cache-revalidate", vec![sample("a"), sample("b")]);

        let opened = cache
            .open_scripted(
                options("cache-revalidate", VolumeKind::Local),
                ScriptedSource::entries(vec![
                    (sample("a"), Duration::ZERO),
                    (sample("c"), Duration::ZERO),
                ]),
            )
            .unwrap();
        assert_eq!(names(opened.cached.as_ref().unwrap()), ["a", "b"]);
        let mut live = opened.live.unwrap();
        let events = drive(&mut live, Duration::from_secs(1));
        assert!(events
            .iter()
            .any(|event| matches!(event, ListingEvent::Finished)));

        let updated = cache
            .cached_entries(&VolumeId::local("cache-revalidate"), &Path::new("C:\\data"))
            .unwrap();
        assert_eq!(names(&updated), ["a", "c"]);
    }

    #[test]
    fn slow_volume_does_not_delay_another_cache_hit() {
        let cache = ListingCache::new();
        finish(&cache, "cache-slow-a", vec![sample("a")]);
        finish(&cache, "cache-slow-b", vec![sample("b")]);

        let slow = cache
            .open_scripted(
                options_timeout("cache-slow-a", Duration::from_secs(2)),
                ScriptedSource::hang(),
            )
            .unwrap();
        assert_eq!(names(slow.cached.as_ref().unwrap()), ["a"]);

        let started = Instant::now();
        let other = cache
            .open_scripted(
                options("cache-slow-b", VolumeKind::Local),
                ScriptedSource::entries(vec![(sample("later"), Duration::from_millis(400))]),
            )
            .unwrap();
        let elapsed = started.elapsed();
        other.live.unwrap().cancel();
        slow.live.unwrap().cancel();

        assert!(
            elapsed < Duration::from_millis(200),
            "other volume waited on the slow revalidation ({elapsed:?})"
        );
        assert_eq!(names(other.cached.as_ref().unwrap()), ["b"]);
    }

    #[test]
    fn timed_out_revalidation_keeps_the_cached_listing() {
        let cache = ListingCache::new();
        finish(&cache, "cache-timeout", vec![sample("kept")]);

        let opened = cache
            .open_scripted(
                options_timeout("cache-timeout", Duration::from_millis(80)),
                ScriptedSource::hang(),
            )
            .unwrap();
        let mut live = opened.live.unwrap();
        let events = drive(&mut live, Duration::from_secs(1));
        assert!(events
            .iter()
            .any(|event| matches!(event, ListingEvent::TimedOut)));

        let stored = cache
            .cached_entries(&VolumeId::network("cache-timeout"), &Path::new("C:\\data"))
            .unwrap();
        assert_eq!(names(&stored), ["kept"]);
    }
}
