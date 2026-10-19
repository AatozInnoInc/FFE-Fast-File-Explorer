use std::fmt;
use std::io;
#[cfg(any(test, windows))]
use std::time::Duration;
use std::time::SystemTime;

/// Owned filesystem path. This is the shared-kernel path, not `std::path::Path`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Path(String);

impl Path {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Path {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for Path {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

/// Whether a volume uses the network timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VolumeKind {
    /// No enumeration deadline. Cancellation still applies.
    Local,
    /// Configurable deadline. The default is [`crate::DEFAULT_NETWORK_TIMEOUT`].
    Network,
}

/// Identity of one isolated volume. The worker key is `key`, not the kind.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VolumeId {
    key: String,
    kind: VolumeKind,
}

impl VolumeId {
    pub fn new(key: impl Into<String>, kind: VolumeKind) -> Self {
        Self {
            key: key.into(),
            kind,
        }
    }

    pub fn local(key: impl Into<String>) -> Self {
        Self::new(key, VolumeKind::Local)
    }

    pub fn network(key: impl Into<String>) -> Self {
        Self::new(key, VolumeKind::Network)
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn kind(&self) -> VolumeKind {
        self.kind
    }

    /// Drive-letter paths are local. UNC paths are network.
    pub fn from_directory(directory: &Path) -> Self {
        let normalized = directory.as_str().replace('/', "\\");
        let trimmed = normalized.trim_end_matches('\\');
        if let Some(body) = trimmed.strip_prefix("\\\\?\\UNC\\") {
            return VolumeId::network(unc_volume_key(body));
        }
        if let Some(body) = trimmed.strip_prefix("\\\\?\\") {
            return classify_rooted(body);
        }
        if let Some(body) = trimmed.strip_prefix("\\\\") {
            return VolumeId::network(unc_volume_key(body));
        }
        classify_rooted(trimmed)
    }
}

fn classify_rooted(path: &str) -> VolumeId {
    if path.len() >= 2 && path.as_bytes().get(1) == Some(&b':') {
        VolumeId::local(&path[..2])
    } else {
        VolumeId::local(path)
    }
}

fn unc_volume_key(body: &str) -> String {
    let mut parts = body.split('\\').filter(|part| !part.is_empty());
    let server = parts.next().unwrap_or("");
    let share = parts.next().unwrap_or("");
    format!("\\\\{server}\\{share}")
}

/// `directory` as the wide search pattern passed to `FindFirstFileExW`, without the UTF-16 encoding.
#[cfg(any(test, windows))]
pub(crate) fn windows_search_pattern(directory: &str) -> String {
    let normalized = directory.replace('/', "\\");
    let trimmed = normalized.trim_end_matches('\\');
    let prefixed = if trimmed.starts_with("\\\\?\\") {
        trimmed.to_string()
    } else if let Some(unc) = trimmed.strip_prefix("\\\\") {
        format!("\\\\?\\UNC\\{unc}")
    } else {
        format!("\\\\?\\{trimmed}")
    };
    format!("{prefixed}\\*")
}

/// One directory entry. Fields match what `WIN32_FIND_DATAW` already returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub attributes: u32,
    pub size: u64,
    pub created: SystemTime,
    pub last_write: SystemTime,
    pub is_directory: bool,
    pub is_reparse_point: bool,
}

#[cfg(any(test, windows))]
pub(crate) fn filetime_to_systemtime(low: u32, high: u32) -> SystemTime {
    let ticks = ((high as u64) << 32) | u64::from(low);
    const EPOCH_DIFF_100NS: u64 = 11_644_473_600_000_000;
    if ticks <= EPOCH_DIFF_100NS {
        return SystemTime::UNIX_EPOCH;
    }
    let hundred_ns = ticks - EPOCH_DIFF_100NS;
    let secs = hundred_ns / 10_000_000;
    let sub = hundred_ns % 10_000_000;
    let nanos = u32::try_from(sub * 100).unwrap_or(0);
    SystemTime::UNIX_EPOCH + Duration::new(secs, nanos)
}

/// Failure of a listing. Cancellation and timeout are per volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnumError {
    VolumeBusy,
    Cancelled,
    TimedOut,
    WindowsOnly,
    WorkerStopped,
    SpawnFailed,
    Io { code: u32 },
}

impl fmt::Display for EnumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VolumeBusy => write!(f, "volume already has an enumeration worker"),
            Self::Cancelled => write!(f, "enumeration cancelled"),
            Self::TimedOut => write!(f, "enumeration timed out"),
            Self::WindowsOnly => write!(f, "directory enumeration requires Windows"),
            Self::WorkerStopped => write!(f, "enumeration worker stopped"),
            Self::SpawnFailed => write!(f, "failed to spawn enumeration worker"),
            Self::Io { code } => write!(f, "enumeration io error {code}"),
        }
    }
}

impl std::error::Error for EnumError {}

impl From<io::Error> for EnumError {
    fn from(value: io::Error) -> Self {
        let code = value.raw_os_error().unwrap_or(0);
        Self::Io {
            code: u32::try_from(code).unwrap_or(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_drive_letters_and_unc_paths() {
        let drive = VolumeId::from_directory(&Path::new("C:\\Users\\a"));
        assert_eq!(drive.key(), "C:");
        assert_eq!(drive.kind(), VolumeKind::Local);

        let slash = VolumeId::from_directory(&Path::new("D:/data"));
        assert_eq!(slash.key(), "D:");
        assert_eq!(slash.kind(), VolumeKind::Local);

        let unc = VolumeId::from_directory(&Path::new("\\\\server\\share\\docs"));
        assert_eq!(unc.key(), "\\\\server\\share");
        assert_eq!(unc.kind(), VolumeKind::Network);

        let long_unc = VolumeId::from_directory(&Path::new("\\\\?\\UNC\\server\\share\\docs"));
        assert_eq!(long_unc.key(), "\\\\server\\share");
        assert_eq!(long_unc.kind(), VolumeKind::Network);
    }

    #[test]
    fn search_pattern_uses_large_fetch_form() {
        assert_eq!(windows_search_pattern("C:\\Users"), "\\\\?\\C:\\Users\\*");
        assert_eq!(windows_search_pattern("C:\\Users\\"), "\\\\?\\C:\\Users\\*");
        assert_eq!(
            windows_search_pattern("\\\\server\\share\\docs"),
            "\\\\?\\UNC\\server\\share\\docs\\*"
        );
        assert_eq!(
            windows_search_pattern("\\\\?\\C:\\Users"),
            "\\\\?\\C:\\Users\\*"
        );
    }

    #[test]
    fn filetime_converts_unix_epoch_plus_one_second() {
        let ticks = 11_644_473_600_000_000u64 + 10_000_000;
        let low = u32::try_from(ticks & 0xFFFF_FFFF).unwrap();
        let high = u32::try_from(ticks >> 32).unwrap();
        assert_eq!(
            filetime_to_systemtime(low, high),
            SystemTime::UNIX_EPOCH + Duration::from_secs(1)
        );
        assert_eq!(filetime_to_systemtime(0, 0), SystemTime::UNIX_EPOCH);
    }
}
