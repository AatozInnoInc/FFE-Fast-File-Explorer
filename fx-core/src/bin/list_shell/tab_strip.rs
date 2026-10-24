//! One `SysTabControl32` above a pane's list.
//!
//! The control shows titles. [`crate::tabs::PaneTabs`] remembers the paths.
//! This module does not enumerate and does not open a directory.

use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows_sys::Win32::UI::Controls::{
    TCIF_TEXT, TCITEMW, TCM_DELETEALLITEMS, TCM_INSERTITEMW, TCM_SETCURSEL, WC_TABCONTROLW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, SendMessageW, WINDOW_STYLE, WS_CHILD, WS_CLIPSIBLINGS, WS_VISIBLE,
};

use crate::panes::PaneId;

/// Control id for the left pane's tabs.
pub const ID_TABS_LEFT: isize = 10;
/// Control id for the right pane's tabs.
pub const ID_TABS_RIGHT: isize = 11;
/// `ICC_TAB_CLASSES`.
pub const TAB_CLASSES: u32 = 0x0008;
/// Height of the tab strip. Not a performance budget.
pub const TAB_HEIGHT: i32 = 22;

pub fn pane_for(control_id: isize) -> Option<PaneId> {
    match control_id {
        ID_TABS_LEFT => Some(PaneId::Left),
        ID_TABS_RIGHT => Some(PaneId::Right),
        _ => None,
    }
}

pub fn create(parent: HWND, id: isize) -> Result<HWND, u8> {
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            WC_TABCONTROLW,
            ptr::null(),
            style(),
            0,
            0,
            0,
            0,
            parent,
            id as _,
            ptr::null_mut(),
            ptr::null(),
        )
    };
    if hwnd.is_null() {
        eprintln!("list_shell could not create the tab strip");
        return Err(1);
    }
    Ok(hwnd)
}

pub fn refill(tab: HWND, titles: &[String], active: usize) {
    unsafe {
        SendMessageW(tab, TCM_DELETEALLITEMS, 0, 0);
    }
    for (index, title) in titles.iter().enumerate() {
        let mut text = wide(title);
        let mut item = TCITEMW {
            mask: TCIF_TEXT,
            dwState: 0,
            dwStateMask: 0,
            pszText: text.as_mut_ptr(),
            cchTextMax: 0,
            iImage: -1,
            lParam: 0,
        };
        unsafe {
            SendMessageW(
                tab,
                TCM_INSERTITEMW,
                index as WPARAM,
                &mut item as *mut TCITEMW as LPARAM,
            );
        }
    }
    if !titles.is_empty() {
        unsafe {
            SendMessageW(tab, TCM_SETCURSEL, active as WPARAM, 0);
        }
    }
}

fn style() -> WINDOW_STYLE {
    WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
