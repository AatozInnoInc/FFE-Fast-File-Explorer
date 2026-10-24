//! Directory enumeration for Rapid Explorer.
//!
//! The Win32 shell calls [`shell::Shell`] in-process. That module is the only shell API.
//! Each [`VolumeId`] has at most one worker. Network volumes have a timeout.
//! Local volumes do not. [`shell::Shell::poll`] never blocks.

mod cache;
mod cancel;
pub mod dump;
mod enumerate;
pub mod filename_index;
pub mod helper;
mod kernel;
mod network;
pub mod rank;
pub mod settings;
mod shell;
mod source;
pub mod thumbs;
pub mod watchdog;

#[cfg(windows)]
mod windows;

pub use enumerate::{ListingEvent, TryEvent, DEFAULT_BATCH_SIZE, DEFAULT_NETWORK_TIMEOUT};
pub use kernel::{EnumError, FileEntry, Path, VolumeId, VolumeKind};
pub use network::{BrowseEvent, NetworkBrowse, NetworkPlace, TryBrowse};
pub use shell::{OpenedDirectory, Shell};
