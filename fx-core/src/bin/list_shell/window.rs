//! One top-level window and a virtual `SysListView32`.
//!
//! The message loop is `GetMessageW`. Enumeration is `Shell::poll` on a timer.
//! That call does not wait, and this thread does not read the directory.

use std::mem;
use std::process::ExitCode;
use std::ptr;

use windows_sys::core::w;
use windows_sys::Win32::Foundation::{
    GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{UpdateWindow, COLOR_WINDOW, HBRUSH};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{
    InitCommonControlsEx, ICC_LISTVIEW_CLASSES, INITCOMMONCONTROLSEX, LVCFMT_LEFT, LVCF_FMT,
    LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_TEXT, LVM_INSERTCOLUMNW, LVM_REDRAWITEMS,
    LVM_SETCOLUMNWIDTH, LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETITEMCOUNT, LVN_GETDISPINFOW,
    LVSICF_NOSCROLL, LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT, LVS_NOSORTHEADER, LVS_OWNERDATA,
    LVS_REPORT, LVS_SHOWSELALWAYS, NMLVDISPINFOW, WC_LISTVIEWW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetWindowLongPtrW, KillTimer, LoadCursorW, MoveWindow, PostQuitMessage, RegisterClassW,
    SendMessageW, SetTimer, SetWindowLongPtrW, ShowWindow, TranslateMessage, CS_HREDRAW,
    CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, HMENU, IDC_ARROW, MSG, SIZE_MINIMIZED, SW_SHOW,
    WINDOW_STYLE, WM_DESTROY, WM_NOTIFY, WM_SIZE, WM_TIMER, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN,
    WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE,
};

use fx_core::{EnumError, OpenedDirectory, Path, Shell, VolumeId};

use crate::rows::{DirectoryRows, RowChange};

/// How often the UI thread drains `Shell::poll`. Not the ≤16 ms paint budget.
const POLL_INTERVAL_MS: u32 = 16;
const TIMER_ID: usize = 1;
const CLASS_NAME: windows_sys::core::PCWSTR = w!("RapidExplorerList");

struct ListWindow {
    list: HWND,
    opened: Option<OpenedDirectory>,
    rows: DirectoryRows,
    polling: bool,
}

pub fn run() -> ExitCode {
    let Some(directory) = std::env::args().nth(1) else {
        eprintln!("usage: list_shell <directory>");
        return ExitCode::from(2);
    };
    let path = Path::new(directory);
    let start = match Shell::new().open(VolumeId::from_directory(&path), path.clone()) {
        Ok(opened) => WindowStart::opened(path, opened),
        Err(err) => WindowStart::failed(path, err),
    };

    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance(),
        hIcon: ptr::null_mut(),
        hCursor: unsafe { LoadCursorW(ptr::null_mut(), IDC_ARROW) },
        hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
        lpszMenuName: ptr::null(),
        lpszClassName: CLASS_NAME,
    };
    if unsafe { RegisterClassW(&class) } == 0 {
        eprintln!(
            "list_shell could not register the window class: {}",
            last_error()
        );
        return ExitCode::from(1);
    }

    let title_wide = wide(&start.caption);
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            CLASS_NAME,
            title_wide.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            ptr::null_mut(),
            ptr::null_mut(),
            instance(),
            ptr::null(),
        )
    };
    if hwnd.is_null() {
        eprintln!("list_shell could not create the window: {}", last_error());
        return ExitCode::from(1);
    }

    if let Err(code) = attach(hwnd, start) {
        destroy(hwnd);
        return ExitCode::from(code);
    }
    message_loop()
}

struct WindowStart {
    caption: String,
    opened: Option<OpenedDirectory>,
    rows: DirectoryRows,
}

impl WindowStart {
    fn opened(path: Path, mut opened: OpenedDirectory) -> Self {
        let rows = DirectoryRows::from_cached(opened.cached.take());
        Self {
            caption: format!("Rapid Explorer — {}", path.as_str()),
            opened: Some(opened),
            rows,
        }
    }

    fn failed(path: Path, err: EnumError) -> Self {
        let mut rows = DirectoryRows::from_cached(None);
        rows.stop();
        Self {
            caption: format!("Rapid Explorer — {} — {err}", path.as_str()),
            opened: None,
            rows,
        }
    }
}

fn attach(hwnd: HWND, start: WindowStart) -> Result<(), u8> {
    let controls = INITCOMMONCONTROLSEX {
        dwSize: mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_LISTVIEW_CLASSES,
    };
    let _ = unsafe { InitCommonControlsEx(&controls) };

    let list = unsafe {
        CreateWindowExW(
            0,
            WC_LISTVIEWW,
            ptr::null(),
            list_style(),
            0,
            0,
            0,
            0,
            hwnd,
            1 as HMENU,
            instance(),
            ptr::null(),
        )
    };
    if list.is_null() {
        eprintln!(
            "list_shell could not create the list view: {}",
            last_error()
        );
        return Err(1);
    }

    let extended = LVS_EX_DOUBLEBUFFER | LVS_EX_FULLROWSELECT;
    unsafe {
        SendMessageW(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            extended as WPARAM,
            extended as LPARAM,
        );
    }
    insert_name_column(list);

    let polling = !start.rows.is_settled();
    let state = Box::new(ListWindow {
        list,
        opened: start.opened,
        rows: start.rows,
        polling,
    });
    let shown = state.rows.shown().len();
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
    }

    fit_list(hwnd);
    unsafe { ShowWindow(hwnd, SW_SHOW) };
    if shown > 0 {
        set_count(list, shown);
    }
    // `UpdateWindow` paints cached rows before `SetTimer` can deliver the first poll.
    unsafe {
        UpdateWindow(list);
        SetFocus(list);
    }
    if polling && unsafe { SetTimer(hwnd, TIMER_ID, POLL_INTERVAL_MS, None) } == 0 {
        eprintln!("list_shell could not start polling: {}", last_error());
        if let Some(state) = state_mut(hwnd) {
            state.polling = false;
        }
    }
    Ok(())
}

fn list_style() -> WINDOW_STYLE {
    WS_CHILD
        | WS_VISIBLE
        | WS_TABSTOP
        | LVS_REPORT
        | LVS_OWNERDATA
        | LVS_SHOWSELALWAYS
        | LVS_NOSORTHEADER
}

fn insert_name_column(list: HWND) {
    let mut name = wide("Name");
    let column = LVCOLUMNW {
        mask: LVCF_TEXT | LVCF_WIDTH | LVCF_FMT,
        fmt: LVCFMT_LEFT,
        cx: 240,
        pszText: name.as_mut_ptr(),
        cchTextMax: 0,
        iSubItem: 0,
        iImage: 0,
        iOrder: 0,
        cxMin: 0,
        cxDefault: 0,
        cxIdeal: 0,
    };
    unsafe {
        SendMessageW(
            list,
            LVM_INSERTCOLUMNW,
            0,
            &column as *const LVCOLUMNW as LPARAM,
        );
    }
}

fn message_loop() -> ExitCode {
    loop {
        let mut message = MSG {
            hwnd: ptr::null_mut(),
            message: 0,
            wParam: 0,
            lParam: 0,
            time: 0,
            pt: POINT { x: 0, y: 0 },
        };
        let status = unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) };
        if status == 0 {
            return ExitCode::from(0);
        }
        if status < 0 {
            eprintln!("list_shell message loop failed: {}", last_error());
            return ExitCode::from(1);
        }
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            on_timer(hwnd);
            0
        }
        WM_SIZE => {
            if wparam != SIZE_MINIMIZED as WPARAM {
                fit_list(hwnd);
            }
            0
        }
        WM_NOTIFY => {
            on_notify(hwnd, lparam);
            0
        }
        WM_DESTROY => {
            on_destroy(hwnd);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn on_timer(hwnd: HWND) {
    let Some(list) = list_handle(hwnd) else {
        return;
    };
    let (change, shown, settled) = {
        let Some(state) = state_mut(hwnd) else {
            return;
        };
        let change = drain(state);
        let shown = state.rows.shown().len();
        let settled = state.rows.is_settled();
        (change, shown, settled)
    };
    paint(list, change, shown);
    if settled {
        unsafe { KillTimer(hwnd, TIMER_ID) };
        if let Some(state) = state_mut(hwnd) {
            state.polling = false;
        }
    }
}

fn drain(state: &mut ListWindow) -> RowChange {
    let ListWindow { opened, rows, .. } = state;
    let Some(opened) = opened.as_mut() else {
        rows.stop();
        return RowChange::Unchanged;
    };
    rows.apply_available(|| opened.poll())
}

fn paint(list: HWND, change: RowChange, shown: usize) {
    match change {
        RowChange::Unchanged => {}
        RowChange::Count => set_count(list, shown),
        RowChange::Redraw => {
            if shown > 0 {
                unsafe {
                    SendMessageW(list, LVM_REDRAWITEMS, 0, (shown - 1) as LPARAM);
                }
            }
        }
    }
}

fn set_count(list: HWND, count: usize) {
    let count = count.min(i32::MAX as usize) as WPARAM;
    unsafe {
        SendMessageW(list, LVM_SETITEMCOUNT, count, LVSICF_NOSCROLL as LPARAM);
    }
}

fn on_notify(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let info = unsafe { &mut *(lparam as *mut NMLVDISPINFOW) };
    if info.hdr.code != LVN_GETDISPINFOW {
        return;
    }
    let item = &mut info.item;
    if item.iSubItem != 0 || item.iItem < 0 || (item.mask & LVIF_TEXT) == 0 {
        return;
    }
    let index = item.iItem as usize;
    let buffer = item.pszText;
    let capacity = item.cchTextMax;
    let Some(state) = state_mut(hwnd) else {
        return;
    };
    match state.rows.shown().get(index) {
        Some(entry) => write_name(buffer, capacity, &entry.name),
        None => write_name(buffer, capacity, ""),
    }
}

fn fit_list(hwnd: HWND) {
    let Some(list) = list_handle(hwnd) else {
        return;
    };
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe {
        GetClientRect(hwnd, &mut rect);
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        MoveWindow(list, 0, 0, width, height, 1);
        SendMessageW(list, LVM_SETCOLUMNWIDTH, 0, width as LPARAM);
    }
}

fn on_destroy(hwnd: HWND) {
    let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut ListWindow;
    if !raw.is_null() {
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
        let state = unsafe { Box::from_raw(raw) };
        if state.polling {
            unsafe { KillTimer(hwnd, TIMER_ID) };
        }
        if let Some(opened) = &state.opened {
            opened.cancel();
        }
        drop(state);
    }
    unsafe { PostQuitMessage(0) };
}

fn list_handle(hwnd: HWND) -> Option<HWND> {
    state_mut(hwnd).map(|state| state.list)
}

fn state_mut(hwnd: HWND) -> Option<&'static mut ListWindow> {
    let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut ListWindow;
    if raw.is_null() {
        None
    } else {
        Some(unsafe { &mut *raw })
    }
}

fn write_name(buffer: windows_sys::core::PWSTR, capacity: i32, name: &str) {
    if buffer.is_null() || capacity <= 0 {
        return;
    }
    let dest = unsafe { std::slice::from_raw_parts_mut(buffer, capacity as usize) };
    let mut written = 0usize;
    for unit in name.encode_utf16() {
        if written + 1 >= dest.len() {
            break;
        }
        dest[written] = unit;
        written += 1;
    }
    dest[written] = 0;
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn instance() -> HINSTANCE {
    unsafe { GetModuleHandleW(ptr::null()) }
}

fn last_error() -> u32 {
    unsafe { GetLastError() }
}

fn destroy(hwnd: HWND) {
    unsafe { DestroyWindow(hwnd) };
}
