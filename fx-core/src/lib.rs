//! Directory enumeration for Rapid Explorer.
//!
//! Each [`VolumeId`] has at most one worker. Network volumes have a timeout.
//! Local volumes do not. [`Listing::try_recv`] never blocks.
//! [`ListingCache`] returns a cached directory immediately, then re-enumerates on that volume's worker.

mod cache;
mod cancel;
mod enumerate;
mod kernel;
mod source;

#[cfg(windows)]
mod windows;

pub use cache::{CachedOpen, ListingCache, LiveListing};
pub use cancel::CancellationToken;
pub use enumerate::{
    enumerate, Listing, ListingEvent, ListingOptions, TryEvent, DEFAULT_BATCH_SIZE,
    DEFAULT_NETWORK_TIMEOUT,
};
pub use kernel::{EnumError, FileEntry, Path, VolumeId, VolumeKind};
