//! Network-timeout dialog template, and the modal dialog on Windows.
//!
//! The template is bytes the dialog manager reads. Saving goes through
//! [`fx_core::settings::TimeoutSetting`]. A rejected value is not stored.
//! This does not auto-retry a timed-out volume.

#![cfg_attr(not(windows), allow(dead_code))]

use fx_core::settings::{ParseError, TimeoutSetting};

const EDIT_ID: u16 = 101;
const ATOM_BUTTON: u16 = 0x0080;
const ATOM_EDIT: u16 = 0x0081;
const ATOM_STATIC: u16 = 0x0082;

/// In-memory dialog: a label, the millisecond edit, OK, and Cancel.
pub fn template() -> Vec<u8> {
    let mut buf = Vec::new();
    push_template(&mut buf);
    push_u16(&mut buf, 0);
    push_u16(&mut buf, 0);
    push_utf16(&mut buf, "Network timeout");
    push_item(
        &mut buf,
        Item {
            style: ws_child_visible(),
            x: 8,
            y: 8,
            cx: 200,
            cy: 10,
            id: 0,
            class_atom: ATOM_STATIC,
            title: "Milliseconds for network volumes only",
        },
    );
    push_item(
        &mut buf,
        Item {
            style: ws_child_visible() | ws_tabstop() | ws_border() | es_number(),
            x: 8,
            y: 22,
            cx: 200,
            cy: 12,
            id: EDIT_ID,
            class_atom: ATOM_EDIT,
            title: "",
        },
    );
    push_item(
        &mut buf,
        Item {
            style: ws_child_visible() | ws_tabstop() | bs_defpushbutton(),
            x: 90,
            y: 44,
            cx: 50,
            cy: 14,
            id: 1,
            class_atom: ATOM_BUTTON,
            title: "OK",
        },
    );
    push_item(
        &mut buf,
        Item {
            style: ws_child_visible() | ws_tabstop(),
            x: 148,
            y: 44,
            cx: 56,
            cy: 14,
            id: 2,
            class_atom: ATOM_BUTTON,
            title: "Cancel",
        },
    );
    buf
}

fn push_template(buf: &mut Vec<u8>) {
    let style = ws_popup() | ws_caption() | ws_sysmenu() | ws_visible() | ds_modal() | ds_center();
    buf.extend_from_slice(&style.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&4u16.to_le_bytes());
    buf.extend_from_slice(&0i16.to_le_bytes());
    buf.extend_from_slice(&0i16.to_le_bytes());
    buf.extend_from_slice(&220i16.to_le_bytes());
    buf.extend_from_slice(&68i16.to_le_bytes());
}

struct Item<'a> {
    style: u32,
    x: i16,
    y: i16,
    cx: i16,
    cy: i16,
    id: u16,
    class_atom: u16,
    title: &'a str,
}

fn push_item(buf: &mut Vec<u8>, item: Item<'_>) {
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
    buf.extend_from_slice(&item.style.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&item.x.to_le_bytes());
    buf.extend_from_slice(&item.y.to_le_bytes());
    buf.extend_from_slice(&item.cx.to_le_bytes());
    buf.extend_from_slice(&item.cy.to_le_bytes());
    buf.extend_from_slice(&item.id.to_le_bytes());
    push_u16(buf, 0xFFFF);
    push_u16(buf, item.class_atom);
    push_utf16(buf, item.title);
    push_u16(buf, 0);
}

fn push_u16(buf: &mut Vec<u8>, value: u16) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn push_utf16(buf: &mut Vec<u8>, text: &str) {
    for unit in text.encode_utf16() {
        push_u16(buf, unit);
    }
    push_u16(buf, 0);
}

fn ws_popup() -> u32 {
    0x8000_0000
}
fn ws_caption() -> u32 {
    0x00C0_0000
}
fn ws_sysmenu() -> u32 {
    0x0008_0000
}
fn ws_visible() -> u32 {
    0x1000_0000
}
fn ws_child_visible() -> u32 {
    0x4000_0000 | ws_visible()
}
fn ws_tabstop() -> u32 {
    0x0001_0000
}
fn ws_border() -> u32 {
    0x0080_0000
}
fn es_number() -> u32 {
    0x2000
}
fn bs_defpushbutton() -> u32 {
    1
}
fn ds_modal() -> u32 {
    0x80
}
fn ds_center() -> u32 {
    0x0800
}

/// Parses the edit box text as `network_timeout_ms`. Empty and out of range fail.
pub fn parse_millis(text: &str) -> Result<TimeoutSetting, ParseError> {
    TimeoutSetting::parse(&format!("network_timeout_ms={}", text.trim()))
}

#[cfg(windows)]
pub fn show(
    parent: windows_sys::Win32::Foundation::HWND,
    millis: u64,
    path: &std::path::Path,
) -> Option<u64> {
    use std::ptr;

    use windows_sys::Win32::Foundation::LPARAM;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{DialogBoxIndirectParamW, DLGTEMPLATE};

    let bytes = template();
    let mut aligned = vec![0u32; bytes.len().div_ceil(4)];
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), aligned.as_mut_ptr() as *mut u8, bytes.len());
    }
    let mut state = DialogState {
        initial: millis.to_string(),
        path: path.to_path_buf(),
        saved: None,
    };
    let code = unsafe {
        DialogBoxIndirectParamW(
            GetModuleHandleW(ptr::null()),
            aligned.as_ptr() as *const DLGTEMPLATE,
            parent,
            Some(dialog_proc),
            &mut state as *mut DialogState as LPARAM,
        )
    };
    if code == 1 {
        state.saved
    } else {
        None
    }
}

#[cfg(windows)]
struct DialogState {
    initial: String,
    path: std::path::PathBuf,
    saved: Option<u64>,
}

#[cfg(windows)]
unsafe extern "system" fn dialog_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EndDialog, SetDlgItemTextW, SetWindowLongPtrW, IDCANCEL, IDOK, WM_COMMAND, WM_INITDIALOG,
    };

    const DWLP_USER: i32 = (std::mem::size_of::<isize>() * 2) as i32;
    match msg {
        WM_INITDIALOG => {
            SetWindowLongPtrW(hwnd, DWLP_USER, lparam);
            if let Some(state) = dialog_state(hwnd) {
                let text = wide(&state.initial);
                SetDlgItemTextW(hwnd, EDIT_ID as i32, text.as_ptr());
            }
            1
        }
        WM_COMMAND => {
            let id = (wparam & 0xffff) as i32;
            if id == IDOK {
                accept(hwnd);
            } else if id == IDCANCEL {
                EndDialog(hwnd, 0);
            }
            1
        }
        _ => 0,
    }
}

#[cfg(windows)]
fn accept(hwnd: windows_sys::Win32::Foundation::HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{EndDialog, GetDlgItemTextW};

    let Some(state) = dialog_state(hwnd) else {
        unsafe { EndDialog(hwnd, 0) };
        return;
    };
    let mut buf = [0u16; 32];
    let len = unsafe { GetDlgItemTextW(hwnd, EDIT_ID as i32, buf.as_mut_ptr(), buf.len() as i32) };
    let text = String::from_utf16_lossy(&buf[..len as usize]);
    match parse_millis(&text) {
        Ok(setting) => match setting.store(&state.path) {
            Ok(()) => {
                state.saved = Some(setting.millis);
                unsafe { EndDialog(hwnd, 1) };
            }
            Err(_) => {
                tell(hwnd, "Could not save the network timeout.");
            }
        },
        Err(_) => {
            tell(
                hwnd,
                "Enter a whole number of milliseconds from 1 through 3600000.",
            );
        }
    }
}

#[cfg(windows)]
fn tell(hwnd: windows_sys::Win32::Foundation::HWND, text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONWARNING, MB_OK};

    let text = wide(text);
    let title = wide("Network timeout");
    unsafe { MessageBoxW(hwnd, text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONWARNING) };
}

#[cfg(windows)]
fn dialog_state(hwnd: windows_sys::Win32::Foundation::HWND) -> Option<&'static mut DialogState> {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW;

    const DWLP_USER: i32 = (std::mem::size_of::<isize>() * 2) as i32;
    let raw = unsafe { GetWindowLongPtrW(hwnd, DWLP_USER) } as *mut DialogState;
    if raw.is_null() {
        None
    } else {
        Some(unsafe { &mut *raw })
    }
}

#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_has_four_controls_and_the_title() {
        let bytes = template();
        assert!(bytes.len() >= 18);
        let count = u16::from_le_bytes([bytes[8], bytes[9]]);
        assert_eq!(count, 4);
        let title: Vec<u8> = "Network timeout"
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        assert!(bytes.windows(title.len()).any(|window| window == title));
    }

    #[test]
    fn edit_text_uses_the_same_bounds_as_the_file() {
        assert_eq!(parse_millis("2000").unwrap().millis, 2_000);
        assert_eq!(parse_millis("  1500").unwrap().millis, 1_500);
        assert!(parse_millis("0").is_err());
        assert!(parse_millis("3600001").is_err());
        assert!(parse_millis("").is_err());
        assert!(parse_millis("1.5").is_err());
        assert!(parse_millis("auto_retry").is_err());
    }
}
