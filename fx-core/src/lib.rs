//! Directory enumeration for Rapid Explorer.
//!
//! The Win32 shell calls [`shell::Shell`] in-process. That module is the only shell API.
//! Each [`VolumeId`] has at most one worker. Network volumes have a timeout.
//! Local volumes do not. [`shell::Shell::poll`] never blocks.

mod cache;
mod cancel;
mod enumerate;
mod kernel;
mod network;
pub mod shell;
mod source;

#[cfg(windows)]
mod windows;

pub use enumerate::{ListingEvent, TryEvent, DEFAULT_BATCH_SIZE, DEFAULT_NETWORK_TIMEOUT};
pub use kernel::{EnumError, FileEntry, Path, VolumeId, VolumeKind};
pub use network::{BrowseEvent, NetworkBrowse, NetworkPlace, TryBrowse};
pub use shell::{OpenedDirectory, Shell};
