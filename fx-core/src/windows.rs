use std::mem;
use std::ptr;

use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW,
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FIND_FIRST_EX_LARGE_FETCH,
    WIN32_FIND_DATAW,
};

use crate::cancel::CancellationToken;
use crate::kernel::{filetime_to_systemtime, windows_search_pattern, EnumError, FileEntry, Path};
use crate::source::DirectorySource;

/// Production directory reader. A single `FindNextFileW` call cannot be preempted;
/// the caller observes the deadline through [`crate::Listing::try_recv`] instead.
pub(crate) struct WindowsSource {
    handle: windows_sys::Win32::Foundation::HANDLE,
    pending: Option<FileEntry>,
    exhausted: bool,
}

impl WindowsSource {
    pub(crate) fn open(directory: &Path) -> Result<Self, EnumError> {
        let pattern = wide_pattern(&windows_search_pattern(directory.as_str()));
        let mut data: WIN32_FIND_DATAW = unsafe { mem::zeroed() };
        let handle = unsafe {
            FindFirstFileExW(
                pattern.as_ptr(),
                FindExInfoBasic,
                &mut data as *mut WIN32_FIND_DATAW as *mut _,
                FindExSearchNameMatch,
                ptr::null(),
                FIND_FIRST_EX_LARGE_FETCH,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            let code = unsafe { GetLastError() };
            if code == ERROR_FILE_NOT_FOUND || code == ERROR_NO_MORE_FILES {
                return Ok(Self {
                    handle: INVALID_HANDLE_VALUE,
                    pending: None,
                    exhausted: true,
                });
            }
            return Err(EnumError::Io { code });
        }
        Ok(Self {
            handle,
            pending: Some(entry_from_data(&data)),
            exhausted: false,
        })
    }

    fn pull(&mut self) -> Result<Option<FileEntry>, EnumError> {
        if let Some(entry) = self.pending.take() {
            return Ok(Some(entry));
        }
        if self.exhausted || self.handle == INVALID_HANDLE_VALUE {
            return Ok(None);
        }
        let mut data: WIN32_FIND_DATAW = unsafe { mem::zeroed() };
        let ok = unsafe { FindNextFileW(self.handle, &mut data) };
        if ok == 0 {
            let code = unsafe { GetLastError() };
            self.exhausted = true;
            if code == ERROR_NO_MORE_FILES {
                return Ok(None);
            }
            return Err(EnumError::Io { code });
        }
        Ok(Some(entry_from_data(&data)))
    }
}

impl DirectorySource for WindowsSource {
    fn next_entry(
        &mut self,
        cancel: &CancellationToken,
        deadline: Option<std::time::Instant>,
    ) -> Result<Option<FileEntry>, EnumError> {
        loop {
            if cancel.is_cancelled() {
                return Err(EnumError::Cancelled);
            }
            if deadline.is_some_and(|limit| std::time::Instant::now() >= limit) {
                return Err(EnumError::TimedOut);
            }
            match self.pull()? {
                Some(entry) if entry.name == "." || entry.name == ".." => continue,
                other => return Ok(other),
            }
        }
    }
}

impl Drop for WindowsSource {
    fn drop(&mut self) {
        if !self.handle.is_null() && self.handle != INVALID_HANDLE_VALUE {
            unsafe {
                FindClose(self.handle);
            }
            self.handle = INVALID_HANDLE_VALUE;
        }
    }
}

fn entry_from_data(data: &WIN32_FIND_DATAW) -> FileEntry {
    let attributes = data.dwFileAttributes;
    FileEntry {
        name: decode_name(&data.cFileName),
        attributes,
        size: (u64::from(data.nFileSizeHigh) << 32) | u64::from(data.nFileSizeLow),
        created: filetime_to_systemtime(
            data.ftCreationTime.dwLowDateTime,
            data.ftCreationTime.dwHighDateTime,
        ),
        last_write: filetime_to_systemtime(
            data.ftLastWriteTime.dwLowDateTime,
            data.ftLastWriteTime.dwHighDateTime,
        ),
        is_directory: attributes & FILE_ATTRIBUTE_DIRECTORY != 0,
        is_reparse_point: attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0,
    }
}

fn decode_name(buf: &[u16]) -> String {
    let end = buf.iter().position(|unit| *unit == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

fn wide_pattern(pattern: &str) -> Vec<u16> {
    pattern.encode_utf16().chain(std::iter::once(0)).collect()
}
