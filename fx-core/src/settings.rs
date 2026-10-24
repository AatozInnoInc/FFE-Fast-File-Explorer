//! The network enumeration timeout.
//!
//! One setting, one line: `network_timeout_ms=<integer>`. The default is
//! [`crate::DEFAULT_NETWORK_TIMEOUT`], which is 2 seconds. A missing file stays
//! missing and reads as that default.
//!
//! This type does not decide whether a timed-out volume auto-retries.
//! [`MAX_TIMEOUT_MS`] is a validation bound, not a performance budget.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::DEFAULT_NETWORK_TIMEOUT;

/// Longest accepted `network_timeout_ms`. One hour.
///
/// Validation bound, not a performance budget.
pub const MAX_TIMEOUT_MS: u64 = 3_600_000;

/// Network enumeration timeout, in milliseconds.
///
/// Parsed values are from 1 through [`MAX_TIMEOUT_MS`]. The default comes from
/// [`DEFAULT_NETWORK_TIMEOUT`]. Retry policy is not stored here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeoutSetting {
    /// Timeout in milliseconds.
    pub millis: u64,
}

impl Default for TimeoutSetting {
    fn default() -> Self {
        Self {
            millis: default_millis(),
        }
    }
}

impl TimeoutSetting {
    /// Reads `network_timeout_ms=<integer>`.
    ///
    /// The line may have surrounding whitespace. No other key is accepted.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let line = text.trim();
        if line.is_empty() {
            return Err(ParseError::Empty);
        }
        let Some(raw) = line.strip_prefix("network_timeout_ms=") else {
            return Err(ParseError::Invalid);
        };
        if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ParseError::Invalid);
        }
        let millis = raw.parse::<u64>().map_err(|_| ParseError::Invalid)?;
        if millis == 0 {
            return Err(ParseError::Zero);
        }
        if millis > MAX_TIMEOUT_MS {
            return Err(ParseError::TooLarge);
        }
        Ok(Self { millis })
    }

    /// Reads the setting at `path`.
    ///
    /// A missing file returns [`TimeoutSetting::default`] and does not create
    /// the file. Any other I/O failure, or a file that does not parse, is
    /// returned.
    pub fn load(path: &Path) -> Result<Self, LoadError> {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(LoadError::Parse),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(LoadError::Io(err)),
        }
    }

    /// Writes the canonical line and a trailing newline.
    pub fn store(&self, path: &Path) -> io::Result<()> {
        fs::write(path, format!("{self}\n"))
    }
}

impl fmt::Display for TimeoutSetting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "network_timeout_ms={}", self.millis)
    }
}

/// `network_timeout_ms` could not be read from text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// Empty, or only whitespace.
    Empty,
    /// Not the single key `network_timeout_ms` with an integer value.
    Invalid,
    /// The integer was zero.
    Zero,
    /// The integer was above [`MAX_TIMEOUT_MS`].
    TooLarge,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "network timeout setting is empty"),
            Self::Invalid => {
                write!(
                    f,
                    "network timeout setting must be network_timeout_ms=<integer>"
                )
            }
            Self::Zero => write!(f, "network timeout must be greater than zero"),
            Self::TooLarge => write!(f, "network timeout is above {MAX_TIMEOUT_MS} milliseconds"),
        }
    }
}

impl std::error::Error for ParseError {}

/// [`TimeoutSetting::load`] failed.
#[derive(Debug)]
pub enum LoadError {
    Io(io::Error),
    Parse(ParseError),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "failed to read network timeout setting: {err}"),
            Self::Parse(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Parse(err) => Some(err),
        }
    }
}

/// `rapid-explorer-settings.txt` in the same directory as `exe`.
pub fn path_beside(exe: &Path) -> PathBuf {
    let parent = exe.parent().unwrap_or_else(|| Path::new(""));
    parent.join("rapid-explorer-settings.txt")
}

/// Settings file beside this executable, when that path can be resolved.
pub fn beside_exe() -> Option<PathBuf> {
    std::env::current_exe().ok().map(|exe| path_beside(&exe))
}

fn default_millis() -> u64 {
    let millis = u64::try_from(DEFAULT_NETWORK_TIMEOUT.as_millis())
        .expect("DEFAULT_NETWORK_TIMEOUT fits in u64 milliseconds");
    debug_assert!(
        (1..=MAX_TIMEOUT_MS).contains(&millis),
        "DEFAULT_NETWORK_TIMEOUT is outside the setting bounds"
    );
    millis
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "fx-core-settings-{}-{nanos}-{n}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn path_beside_uses_the_executable_directory() {
        let exe = Path::new("Program").join("list_shell.exe");
        let path = path_beside(&exe);
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("rapid-explorer-settings.txt")
        );
        assert_eq!(path.parent(), Some(Path::new("Program")));
    }

    #[test]
    fn default_is_the_product_network_timeout() {
        let setting = TimeoutSetting::default();
        assert_eq!(setting.millis, 2_000);
        assert_eq!(
            Duration::from_millis(setting.millis),
            crate::DEFAULT_NETWORK_TIMEOUT
        );
        assert_eq!(setting, TimeoutSetting { millis: 2_000 });
    }

    #[test]
    fn parses_the_single_line_and_rejects_everything_else() {
        assert_eq!(
            TimeoutSetting::parse("network_timeout_ms=2000").unwrap(),
            TimeoutSetting { millis: 2_000 }
        );
        assert_eq!(
            TimeoutSetting::parse("  network_timeout_ms=1500 \r\n").unwrap(),
            TimeoutSetting { millis: 1_500 }
        );
        assert_eq!(
            TimeoutSetting::parse("\nnetwork_timeout_ms=1\n")
                .unwrap()
                .millis,
            1
        );
        assert_eq!(
            TimeoutSetting::parse("network_timeout_ms=3600000")
                .unwrap()
                .millis,
            MAX_TIMEOUT_MS
        );
        assert_eq!(
            TimeoutSetting::parse("network_timeout_ms=0002")
                .unwrap()
                .millis,
            2
        );

        assert_eq!(TimeoutSetting::parse("").unwrap_err(), ParseError::Empty);
        assert_eq!(
            TimeoutSetting::parse(" \n\t").unwrap_err(),
            ParseError::Empty
        );
        assert_eq!(
            TimeoutSetting::parse("network_timeout_ms=abc").unwrap_err(),
            ParseError::Invalid
        );
        assert_eq!(
            TimeoutSetting::parse("network_timeout_ms=1.5").unwrap_err(),
            ParseError::Invalid
        );
        assert_eq!(
            TimeoutSetting::parse("network_timeout_ms=-1").unwrap_err(),
            ParseError::Invalid
        );
        assert_eq!(
            TimeoutSetting::parse("network_timeout_ms=").unwrap_err(),
            ParseError::Invalid
        );
        assert_eq!(
            TimeoutSetting::parse("timeout_ms=2000").unwrap_err(),
            ParseError::Invalid
        );
        assert_eq!(
            TimeoutSetting::parse("network_timeout_ms=2000\nauto_retry=true").unwrap_err(),
            ParseError::Invalid
        );
        assert_eq!(
            TimeoutSetting::parse("auto_retry=true").unwrap_err(),
            ParseError::Invalid
        );
        assert_eq!(
            TimeoutSetting::parse("network_timeout_ms=0").unwrap_err(),
            ParseError::Zero
        );
        assert_eq!(
            TimeoutSetting::parse("network_timeout_ms=00").unwrap_err(),
            ParseError::Zero
        );
        assert_eq!(
            TimeoutSetting::parse("network_timeout_ms=3600001").unwrap_err(),
            ParseError::TooLarge
        );
        assert_eq!(
            TimeoutSetting::parse("network_timeout_ms=999999999999999999999").unwrap_err(),
            ParseError::Invalid
        );
    }

    #[test]
    fn display_writes_the_canonical_line() {
        let setting = TimeoutSetting::parse("network_timeout_ms=0002").unwrap();
        assert_eq!(setting.to_string(), "network_timeout_ms=2");
        assert_eq!(
            TimeoutSetting { millis: 2_000 }.to_string(),
            "network_timeout_ms=2000"
        );
    }

    #[test]
    fn missing_file_returns_the_default_and_does_not_create_it() {
        let dir = TempDir::new();
        let path = dir.0.join("timeout.txt");
        let loaded = TimeoutSetting::load(&path).unwrap();
        assert_eq!(loaded, TimeoutSetting::default());
        assert!(!path.exists());
    }

    #[test]
    fn store_writes_the_canonical_line_and_a_trailing_newline() {
        let dir = TempDir::new();
        let path = dir.0.join("timeout.txt");
        let setting = TimeoutSetting { millis: 1_500 };
        setting.store(&path).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "network_timeout_ms=1500\n"
        );
        assert_eq!(TimeoutSetting::load(&path).unwrap(), setting);

        TimeoutSetting {
            millis: MAX_TIMEOUT_MS,
        }
        .store(&path)
        .unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("network_timeout_ms={MAX_TIMEOUT_MS}\n")
        );
    }

    #[test]
    fn present_but_unreadable_text_is_a_parse_error() {
        let dir = TempDir::new();
        let path = dir.0.join("timeout.txt");
        fs::write(&path, " \n").unwrap();
        assert!(matches!(
            TimeoutSetting::load(&path),
            Err(LoadError::Parse(ParseError::Empty))
        ));

        fs::write(&path, "auto_retry=true\n").unwrap();
        assert!(matches!(
            TimeoutSetting::load(&path),
            Err(LoadError::Parse(ParseError::Invalid))
        ));

        let nested = dir.0.join("nested");
        fs::create_dir(&nested).unwrap();
        assert!(matches!(
            TimeoutSetting::load(&nested),
            Err(LoadError::Io(_))
        ));
    }
}
