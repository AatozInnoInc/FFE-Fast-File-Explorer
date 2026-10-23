//! Local crash dump.
//!
//! [`write_dump`] writes one text file in a directory the caller chooses.
//! On Windows, [`capture`] also asks `dbghelp` for a minidump beside that file.
//! Nothing in this module opens a socket. There is no telemetry endpoint.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Writes `message` to `rapid-explorer-<unix_ms>.txt` under `directory`.
///
/// The directory is created if it is missing. The file is the dump. It is not
/// uploaded.
pub fn write_dump(directory: &Path, message: &str) -> io::Result<PathBuf> {
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    write_dump_at(directory, message, unix_ms)
}

/// Same as [`write_dump`] with an explicit millisecond stamp.
pub fn write_dump_at(directory: &Path, message: &str, unix_ms: u128) -> io::Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let path = directory.join(format!("rapid-explorer-{unix_ms}.txt"));
    let body = format!("Rapid Explorer crash dump\nunix_ms: {unix_ms}\nmessage:\n{message}\n");
    fs::write(&path, body)?;
    Ok(path)
}

/// Text dump, and on Windows a minidump in the same directory.
pub fn capture(directory: &Path, message: &str) -> io::Result<PathBuf> {
    let path = write_dump(directory, message)?;
    #[cfg(windows)]
    {
        let _ = write_minidump(directory);
    }
    Ok(path)
}

/// Installs a panic hook that writes a dump under `directory`, then calls the
/// previous hook. This does not write a dump until a panic.
pub fn install(directory: impl Into<PathBuf>) {
    let directory = directory.into();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = capture(&directory, &panic_message(info));
        previous(info);
    }));
}

/// Dump directory beside this executable. Does nothing when the path is unknown.
pub fn install_beside_exe() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(parent) = exe.parent() else {
        return;
    };
    install(parent.join("dumps"));
}

fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = info
        .payload()
        .downcast_ref::<&str>()
        .copied()
        .map(str::to_string)
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".to_string());
    match info.location() {
        Some(location) => format!("{payload}\n{}:{}", location.file(), location.line()),
        None => payload,
    }
}

#[cfg(windows)]
fn write_minidump(directory: &Path) -> io::Result<PathBuf> {
    use std::os::windows::io::AsRawHandle;
    use std::ptr;

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::Diagnostics::Debug::{MiniDumpNormal, MiniDumpWriteDump};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId};

    fs::create_dir_all(directory)?;
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    let path = directory.join(format!("rapid-explorer-{unix_ms}.dmp"));
    let file = fs::File::create(&path)?;
    let handle = file.as_raw_handle() as HANDLE;
    let wrote = unsafe {
        MiniDumpWriteDump(
            GetCurrentProcess(),
            GetCurrentProcessId(),
            handle,
            MiniDumpNormal,
            ptr::null(),
            ptr::null(),
            ptr::null(),
        )
    };
    drop(file);
    if wrote == 0 {
        let err = io::Error::last_os_error();
        let _ = fs::remove_file(&path);
        return Err(err);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("fx-core-dump-{name}-{nanos}"));
        let _ = fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn dump_is_a_local_text_file() {
        let dir = temp_dir("one");
        let path = write_dump_at(&dir, "listing worker panicked", 1_762_000_000_000).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("Rapid Explorer crash dump"));
        assert!(text.contains("unix_ms: 1762000000000"));
        assert!(text.contains("listing worker panicked"));
        assert_eq!(path.extension().and_then(|ext| ext.to_str()), Some("txt"));
        assert!(path.starts_with(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_stamps_are_two_files() {
        let dir = temp_dir("two");
        let first = write_dump_at(&dir, "one", 10).unwrap();
        let second = write_dump_at(&dir, "two", 11).unwrap();
        assert_ne!(first, second);
        assert!(first.is_file());
        assert!(second.is_file());
        let _ = fs::remove_dir_all(&dir);
    }
}
