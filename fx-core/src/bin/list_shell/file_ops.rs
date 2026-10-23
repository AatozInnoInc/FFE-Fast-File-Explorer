//! Clipboard paths and file operations for the context menu.
//!
//! Copy and cut only write path lists to the clipboard. Paste, delete, and
//! rename run on a detached thread. [`FileOp::try_recv`] does not wait, and
//! dropping a [`FileOp`] does not join that thread. This module does not load
//! a third-party shell extension.

use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use windows_sys::Win32::Foundation::{GlobalFree, BOOL, HGLOBAL, HWND, POINT};
use windows_sys::Win32::Storage::FileSystem::MoveFileW;
use windows_sys::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW,
    SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::UI::Shell::{
    DragQueryFileW, SHFileOperationW, SHObjectProperties, FOF_ALLOWUNDO, FOF_NOCONFIRMATION,
    FOF_NOERRORUI, FOF_RENAMEONCOLLISION, FOF_SILENT, FO_COPY, FO_DELETE, FO_MOVE, HDROP,
    SHFILEOPSTRUCTW, SHOP_FILEPATH,
};

use crate::commands::double_null;

/// `CF_HDROP`.
const CF_HDROP: u32 = 15;
/// `DROPEFFECT_COPY`.
const DROP_COPY: u32 = 1;
/// `DROPEFFECT_MOVE`.
const DROP_MOVE: u32 = 2;
const GMEM_ZEROINIT: u32 = 64;

/// Paths from the clipboard, and whether the drop effect was Cut.
pub struct Paste {
    pub sources: Vec<String>,
    pub cut: bool,
}

enum Phase {
    Pending,
    Done(bool),
}

/// One file operation on a detached thread.
pub struct FileOp {
    directory: String,
    state: Arc<Mutex<Phase>>,
}

pub enum Op {
    CopyInto { sources: Vec<String>, dest: String },
    MoveInto { sources: Vec<String>, dest: String },
    Delete { paths: Vec<String> },
    Rename { from: String, to: String },
}

impl FileOp {
    /// Starts `op` on a new thread. `None` means the thread was not created.
    /// This does not wait.
    pub fn start(op: Op, directory: String) -> Option<Self> {
        let state = Arc::new(Mutex::new(Phase::Pending));
        let worker_state = Arc::clone(&state);
        let spawned = thread::Builder::new()
            .name("fx-core-file-op".to_string())
            .spawn(move || {
                let ok = perform(op);
                *lock(&worker_state) = Phase::Done(ok);
            })
            .is_ok();
        if spawned {
            Some(Self { directory, state })
        } else {
            None
        }
    }

    pub fn directory(&self) -> &str {
        &self.directory
    }

    /// `None` while the worker is still running. Later calls repeat the result.
    pub fn try_recv(&self) -> Option<bool> {
        match *lock(&self.state) {
            Phase::Pending => None,
            Phase::Done(ok) => Some(ok),
        }
    }
}

pub fn clipboard_has_files() -> bool {
    read_paste().is_some_and(|paste| !paste.sources.is_empty())
}

pub fn copy_paths(paths: &[String], cut: bool) -> bool {
    if paths.is_empty() {
        return false;
    }
    let effect = if cut { DROP_MOVE } else { DROP_COPY };
    let files = clipboard_files(paths);
    let effect_bytes = effect.to_le_bytes();
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return false;
        }
        EmptyClipboard();
        let files_ok = set_global(CF_HDROP, &files);
        let format = RegisterClipboardFormatW(wide("Preferred DropEffect").as_ptr());
        let effect_ok = format != 0 && set_global(format, &effect_bytes);
        CloseClipboard();
        files_ok && effect_ok
    }
}

pub fn paste_request() -> Option<Paste> {
    read_paste().filter(|paste| !paste.sources.is_empty())
}

/// The shell property sheet. It is a dialog on this thread.
pub fn show_properties(hwnd: HWND, path: &str) {
    let wide_path = wide(path);
    unsafe {
        SHObjectProperties(
            hwnd,
            SHOP_FILEPATH as u32,
            wide_path.as_ptr(),
            std::ptr::null(),
        );
    }
}

fn read_paste() -> Option<Paste> {
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return None;
        }
        let handle = GetClipboardData(CF_HDROP);
        let sources = if handle.is_null() {
            Vec::new()
        } else {
            drop_paths(handle as HDROP)
        };
        let format = RegisterClipboardFormatW(wide("Preferred DropEffect").as_ptr());
        let cut = if format == 0 {
            false
        } else {
            drop_effect(GetClipboardData(format)) == DROP_MOVE
        };
        CloseClipboard();
        Some(Paste { sources, cut })
    }
}

fn drop_paths(drop: HDROP) -> Vec<String> {
    let count = unsafe { DragQueryFileW(drop, 0xFFFF_FFFF, std::ptr::null_mut(), 0) };
    let mut paths = Vec::with_capacity(count as usize);
    for index in 0..count {
        let chars = unsafe { DragQueryFileW(drop, index, std::ptr::null_mut(), 0) };
        if chars == 0 {
            continue;
        }
        let mut buf = vec![0u16; chars as usize + 1];
        let written = unsafe { DragQueryFileW(drop, index, buf.as_mut_ptr(), buf.len() as u32) };
        if written == 0 {
            continue;
        }
        paths.push(String::from_utf16_lossy(&buf[..written as usize]));
    }
    paths
}

fn drop_effect(handle: HGLOBAL) -> u32 {
    if handle.is_null() {
        return DROP_COPY;
    }
    let locked = unsafe { GlobalLock(handle) };
    if locked.is_null() {
        return DROP_COPY;
    }
    let effect = unsafe { std::ptr::read_unaligned(locked as *const u32) };
    unsafe { GlobalUnlock(handle) };
    effect
}

fn clipboard_files(paths: &[String]) -> Vec<u8> {
    let names = double_null(paths);
    let header = DropFiles {
        p_files: std::mem::size_of::<DropFiles>() as u32,
        pt: POINT { x: 0, y: 0 },
        f_nc: 0,
        f_wide: 1,
    };
    let mut bytes = Vec::with_capacity(std::mem::size_of::<DropFiles>() + names.len() * 2);
    bytes.extend_from_slice(unsafe {
        std::slice::from_raw_parts(
            &header as *const DropFiles as *const u8,
            std::mem::size_of::<DropFiles>(),
        )
    });
    for unit in names {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

#[repr(C)]
struct DropFiles {
    p_files: u32,
    pt: POINT,
    f_nc: BOOL,
    f_wide: BOOL,
}

fn set_global(format: u32, bytes: &[u8]) -> bool {
    let flags = GMEM_MOVEABLE | GMEM_ZEROINIT;
    let handle = unsafe { GlobalAlloc(flags, bytes.len()) };
    if handle.is_null() {
        return false;
    }
    let locked = unsafe { GlobalLock(handle) };
    if locked.is_null() {
        unsafe { GlobalFree(handle) };
        return false;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), locked as *mut u8, bytes.len());
        GlobalUnlock(handle);
    }
    if unsafe { SetClipboardData(format, handle) }.is_null() {
        unsafe { GlobalFree(handle) };
        false
    } else {
        true
    }
}

fn perform(op: Op) -> bool {
    let started = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    let ok = match op {
        Op::CopyInto { sources, dest } => shell_op(FO_COPY, &sources, Some(&dest)),
        Op::MoveInto { sources, dest } => shell_op(FO_MOVE, &sources, Some(&dest)),
        Op::Delete { paths } => shell_op(FO_DELETE, &paths, None),
        Op::Rename { from, to } => rename_path(&from, &to),
    };
    if started == 0 || started == 1 {
        unsafe { CoUninitialize() };
    }
    ok
}

fn shell_op(function: u32, sources: &[String], dest: Option<&str>) -> bool {
    if sources.is_empty() {
        return false;
    }
    let from = double_null(sources);
    let to = dest.map(|path| double_null(&[path]));
    let mut flags = FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI;
    if function == FO_DELETE {
        flags |= FOF_ALLOWUNDO;
    }
    if function == FO_COPY || function == FO_MOVE {
        flags |= FOF_RENAMEONCOLLISION;
    }
    let mut file_op = SHFILEOPSTRUCTW {
        hwnd: std::ptr::null_mut(),
        wFunc: function,
        pFrom: from.as_ptr(),
        pTo: to
            .as_ref()
            .map(|buf| buf.as_ptr())
            .unwrap_or(std::ptr::null()),
        fFlags: flags as u16,
        fAnyOperationsAborted: 0,
        hNameMappings: std::ptr::null_mut(),
        lpszProgressTitle: std::ptr::null(),
    };
    unsafe { SHFileOperationW(&mut file_op) == 0 && file_op.fAnyOperationsAborted == 0 }
}

fn rename_path(from: &str, to: &str) -> bool {
    let from = wide(from);
    let to = wide(to);
    unsafe { MoveFileW(from.as_ptr(), to.as_ptr()) != 0 }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}
