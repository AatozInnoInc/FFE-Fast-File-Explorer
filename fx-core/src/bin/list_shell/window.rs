//! One top-level window: address bar, folder tree, and a virtual list.
//!
//! The message loop is `GetMessageW`. Enumeration is `Shell::poll` on a timer.
//! That call does not wait, and this thread does not read a directory. Choosing
//! a folder, typing a path, or going back lists that directory. A cache hit sets
//! the owner-data count and paints before the next `poll`. A row's icon comes
//! from one cached image per extension, or one shared directory icon. Size and
//! Date modified are DirectWrite cells for the rows the list view asks to paint.
//! The Name column stays with the list view so the icon remains. An image in a
//! visible row requests a thumbnail on the background pool, and that buffer
//! covers the icon once it is ready. A Network
//! node stays collapsed until the user expands it.

use std::collections::HashMap;
use std::mem;
use std::process::ExitCode;
use std::ptr;

use windows_sys::core::w;
use windows_sys::Win32::Foundation::{
    GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{UpdateWindow, COLOR_WINDOW, HBRUSH};
use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{
    ImageList_Create, ImageList_ReplaceIcon, InitCommonControlsEx, CDDS_ITEMPOSTPAINT,
    CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDDS_SUBITEM, CDRF_DODEFAULT, CDRF_NOTIFYITEMDRAW,
    CDRF_NOTIFYPOSTPAINT, CDRF_NOTIFYSUBITEMDRAW, CDRF_SKIPDEFAULT, HIMAGELIST, HTREEITEM,
    ICC_LISTVIEW_CLASSES, ICC_TREEVIEW_CLASSES, ILC_COLOR32, ILC_MASK, INITCOMMONCONTROLSEX,
    I_IMAGENONE, LVCFMT_LEFT, LVCF_FMT, LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_IMAGE, LVIF_TEXT,
    LVIR_ICON, LVIS_SELECTED, LVM_GETCOUNTPERPAGE, LVM_GETITEMSTATE, LVM_GETSUBITEMRECT,
    LVM_GETTOPINDEX, LVM_INSERTCOLUMNW, LVM_REDRAWITEMS, LVM_SETCOLUMNWIDTH,
    LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETIMAGELIST, LVM_SETITEMCOUNT, LVN_GETDISPINFOW,
    LVN_ITEMACTIVATE, LVSICF_NOSCROLL, LVSIL_SMALL, LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT,
    LVS_NOSORTHEADER, LVS_OWNERDATA, LVS_REPORT, LVS_SHOWSELALWAYS, NMHDR, NMLISTVIEW,
    NMLVCUSTOMDRAW, NMLVDISPINFOW, NMTREEVIEWW, NM_CUSTOMDRAW, TVE_COLLAPSE, TVE_EXPAND,
    TVGN_CARET, TVIF_CHILDREN, TVIF_HANDLE, TVIF_PARAM, TVIF_TEXT, TVINSERTSTRUCTW,
    TVINSERTSTRUCTW_0, TVITEMW, TVI_LAST, TVI_ROOT, TVM_DELETEITEM, TVM_INSERTITEMW,
    TVM_SELECTITEM, TVM_SETITEMW, TVN_ITEMEXPANDINGW, TVN_SELCHANGEDW, TVS_DISABLEDRAGDROP,
    TVS_HASBUTTONS, TVS_HASLINES, TVS_LINESATROOT, TVS_SHOWSELALWAYS, WC_BUTTONW, WC_EDITW,
    WC_LISTVIEWW, WC_TREEVIEWW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, SetFocus, VK_LEFT, VK_RETURN, VK_RIGHT,
};
use windows_sys::Win32::UI::Shell::{
    SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON, SHGFI_USEFILEATTRIBUTES,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyWindow, DispatchMessageW, GetAncestor,
    GetClientRect, GetMessageW, GetSystemMetrics, GetWindowLongPtrW, GetWindowTextLengthW,
    GetWindowTextW, KillTimer, LoadCursorW, MoveWindow, PostQuitMessage, RegisterClassW,
    SendMessageW, SetTimer, SetWindowLongPtrW, SetWindowTextW, ShowWindow, TranslateMessage,
    BN_CLICKED, BS_PUSHBUTTON, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, ES_AUTOHSCROLL, GA_ROOT,
    GWLP_USERDATA, HMENU, IDC_ARROW, MSG, SIZE_MINIMIZED, SM_CXSMICON, SM_CYSMICON, SW_SHOW,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_COMMAND, WM_DESTROY, WM_KEYDOWN, WM_NOTIFY, WM_SIZE,
    WM_SYSKEYDOWN, WM_TIMER, WNDCLASSW, WS_BORDER, WS_CHILD, WS_CLIPCHILDREN, WS_EX_CLIENTEDGE,
    WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE,
};

use fx_core::thumbs::{ThumbCache, ThumbTicket, TryThumb};
use fx_core::{EnumError, NetworkBrowse, NetworkPlace, OpenedDirectory, Path, Shell, VolumeId};

use crate::columns;
use crate::draw::{self, CellRect};
use crate::icons::{icon_key, IconCache, IconKey};
use crate::images;
use crate::nav::{normalize_address, same_directory, tree_root, History, TreeRoot};
use crate::places::{PlaceChange, PlaceList};
use crate::rows::{DirectoryRows, RowChange};
use crate::tree::{child_directory, ChildChange, Expansion};

/// How often the UI thread drains `Shell::poll`. Not the ≤16 ms paint budget.
const POLL_INTERVAL_MS: u32 = 16;
const TIMER_ID: usize = 1;
/// Width of the folder tree beside the list. Not a performance budget.
const TREE_WIDTH: i32 = 240;
/// Height of the Back, Forward, and address row. Not a performance budget.
const NAV_HEIGHT: i32 = 28;
const NAV_GAP: i32 = 4;
const BACK_WIDTH: i32 = 72;
const FORWARD_WIDTH: i32 = 84;
const ID_LIST: isize = 1;
const ID_TREE: isize = 2;
const ID_BACK: isize = 3;
const ID_FORWARD: isize = 4;
const ID_ADDRESS: isize = 5;
const NETWORK_LABEL: &str = "Network";
const CLASS_NAME: windows_sys::core::PCWSTR = w!("RapidExplorerList");

struct ListWindow {
    shell: Shell,
    list: HWND,
    tree: HWND,
    back: HWND,
    forward: HWND,
    address: HWND,
    history: History,
    /// Directory whose rows are on screen. Unchanged until a navigation opens.
    current: String,
    /// Path of the tree's root item. Not rewritten when a descendant is listed.
    tree_root: String,
    /// A path whose volume worker was busy. The timer retries it. The list stays.
    pending: Option<PendingNav>,
    /// `TVM_SELECTITEM` reenters with `TVN_SELCHANGED`. That must not navigate again.
    suppress_select: bool,
    opened: Option<OpenedDirectory>,
    rows: DirectoryRows,
    /// Image-list indexes. One directory icon, and one icon per extension.
    icons: IconCache<i32>,
    /// Small-icon image list owned by the list view. Zero when it was not created.
    images: HIMAGELIST,
    nodes: Vec<FolderNode>,
    /// The timer is running. This is not "the list is still reading".
    polling: bool,
    /// RGBA thumbnails. The cache is unbounded. Eviction: TODO — define.
    thumbs: ThumbCache,
    /// Tickets for the directory on screen. Navigation cancels these.
    thumb_jobs: HashMap<String, ThumbJob>,
}

struct ThumbJob {
    ticket: ThumbTicket,
    settled: bool,
}

struct FolderNode {
    path: String,
    kind: NodeKind,
    item: HTREEITEM,
    child_indices: Vec<usize>,
    expansion: Option<Expansion>,
    opened: Option<OpenedDirectory>,
    places: Option<PlaceList>,
    browse: Option<NetworkBrowse>,
    removed: bool,
}

#[derive(Clone)]
enum NodeKind {
    Directory,
    /// Collapsed until the user expands it. Not a child of the opened directory.
    NetworkRoot,
    /// A place from a browse. A container expands. A share with a UNC lists that path.
    Place {
        unc: Option<String>,
        container: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NavKind {
    /// Record the path after it opens. Same path as the current entry does nothing.
    Push,
    Back,
    Forward,
}

#[derive(Clone)]
struct PendingNav {
    path: String,
    kind: NavKind,
}

impl FolderNode {
    fn new(path: String) -> Self {
        Self::blank(path, NodeKind::Directory)
    }

    fn network() -> Self {
        Self::blank(String::new(), NodeKind::NetworkRoot)
    }

    fn place(place: &NetworkPlace) -> Self {
        Self::blank(
            String::new(),
            NodeKind::Place {
                unc: place.unc.clone(),
                container: place.container,
            },
        )
    }

    fn blank(path: String, kind: NodeKind) -> Self {
        Self {
            path,
            kind,
            item: 0,
            child_indices: Vec::new(),
            expansion: None,
            opened: None,
            places: None,
            browse: None,
            removed: false,
        }
    }
}

pub fn run() -> ExitCode {
    let Some(directory) = std::env::args().nth(1) else {
        eprintln!("usage: list_shell <directory>");
        return ExitCode::from(2);
    };
    let path = Path::new(directory);
    let shell = Shell::new();
    let start = match shell.open(VolumeId::from_directory(&path), path.clone()) {
        Ok(opened) => WindowStart::opened(shell, path, opened),
        Err(err) => WindowStart::failed(shell, path, err),
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
    shell: Shell,
    root: String,
    caption: String,
    opened: Option<OpenedDirectory>,
    rows: DirectoryRows,
}

impl WindowStart {
    fn opened(shell: Shell, path: Path, mut opened: OpenedDirectory) -> Self {
        let rows = DirectoryRows::from_cached(opened.cached.take());
        Self {
            shell,
            root: path.as_str().to_string(),
            caption: format!("Rapid Explorer — {}", path.as_str()),
            opened: Some(opened),
            rows,
        }
    }

    fn failed(shell: Shell, path: Path, err: EnumError) -> Self {
        let mut rows = DirectoryRows::from_cached(None);
        rows.stop();
        Self {
            shell,
            root: path.as_str().to_string(),
            caption: format!("Rapid Explorer — {} — {err}", path.as_str()),
            opened: None,
            rows,
        }
    }
}

fn attach(hwnd: HWND, start: WindowStart) -> Result<(), u8> {
    let controls = INITCOMMONCONTROLSEX {
        dwSize: mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_LISTVIEW_CLASSES | ICC_TREEVIEW_CLASSES,
    };
    let _ = unsafe { InitCommonControlsEx(&controls) };

    let list = create_control(
        hwnd,
        WC_LISTVIEWW,
        "",
        list_style(),
        0,
        ID_LIST,
        "list view",
    )?;
    let extended = LVS_EX_DOUBLEBUFFER | LVS_EX_FULLROWSELECT;
    unsafe {
        SendMessageW(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            extended as WPARAM,
            extended as LPARAM,
        );
    }
    insert_columns(list);
    let images = create_image_list(list);

    let tree = create_control(
        hwnd,
        WC_TREEVIEWW,
        "",
        tree_style(),
        0,
        ID_TREE,
        "tree view",
    )?;
    let back = create_control(
        hwnd,
        WC_BUTTONW,
        "Back",
        button_style(),
        0,
        ID_BACK,
        "Back button",
    )?;
    let forward = create_control(
        hwnd,
        WC_BUTTONW,
        "Forward",
        button_style(),
        0,
        ID_FORWARD,
        "Forward button",
    )?;
    let address = create_control(
        hwnd,
        WC_EDITW,
        &start.root,
        address_style(),
        WS_EX_CLIENTEDGE,
        ID_ADDRESS,
        "address bar",
    )?;

    let root = start.root.clone();
    let polling = !start.rows.is_settled();
    let state = Box::new(ListWindow {
        shell: start.shell,
        list,
        tree,
        back,
        forward,
        address,
        history: History::start(root.clone()),
        current: root.clone(),
        tree_root: root.clone(),
        pending: None,
        suppress_select: false,
        opened: start.opened,
        rows: start.rows,
        icons: IconCache::new(),
        images,
        nodes: Vec::new(),
        polling,
        thumbs: ThumbCache::with_decoder(images::decode_image),
        thumb_jobs: HashMap::new(),
    });
    let shown = state.rows.shown().len();
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
    }
    insert_root(hwnd, &root);
    insert_network(hwnd);
    update_nav_buttons(hwnd);

    fit_panes(hwnd);
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

fn create_control(
    parent: HWND,
    class: windows_sys::core::PCWSTR,
    text: &str,
    style: WINDOW_STYLE,
    ex_style: WINDOW_EX_STYLE,
    id: isize,
    what: &str,
) -> Result<HWND, u8> {
    let text = wide(text);
    let hwnd = unsafe {
        CreateWindowExW(
            ex_style,
            class,
            text.as_ptr(),
            style,
            0,
            0,
            0,
            0,
            parent,
            id as HMENU,
            instance(),
            ptr::null(),
        )
    };
    if hwnd.is_null() {
        eprintln!("list_shell could not create the {what}: {}", last_error());
        Err(1)
    } else {
        Ok(hwnd)
    }
}

fn tree_style() -> WINDOW_STYLE {
    WS_CHILD
        | WS_VISIBLE
        | WS_TABSTOP
        | TVS_HASBUTTONS
        | TVS_HASLINES
        | TVS_LINESATROOT
        | TVS_SHOWSELALWAYS
        | TVS_DISABLEDRAGDROP
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

fn button_style() -> WINDOW_STYLE {
    WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as WINDOW_STYLE
}

fn address_style() -> WINDOW_STYLE {
    WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as WINDOW_STYLE
}

fn insert_columns(list: HWND) {
    for index in 0..columns::COUNT {
        let mut title = wide(columns::title(index));
        let width = match index {
            columns::SIZE => columns::SIZE_WIDTH,
            columns::MODIFIED => columns::MODIFIED_WIDTH,
            _ => 240,
        };
        let column = LVCOLUMNW {
            mask: LVCF_TEXT | LVCF_WIDTH | LVCF_FMT,
            fmt: LVCFMT_LEFT,
            cx: width,
            pszText: title.as_mut_ptr(),
            cchTextMax: 0,
            iSubItem: index,
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
                index as WPARAM,
                &column as *const LVCOLUMNW as LPARAM,
            );
        }
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
        if intercept(&message) {
            continue;
        }
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

/// Alt+Left, Alt+Right, and Enter in the address bar. The focused child would
/// otherwise eat those keys. Enter is not dispatched, so the edit does not beep.
fn intercept(message: &MSG) -> bool {
    if message.hwnd.is_null() {
        return false;
    }
    let root = unsafe { GetAncestor(message.hwnd, GA_ROOT) };
    if root.is_null() || state_mut(root).is_none() {
        return false;
    }
    match message.message {
        WM_SYSKEYDOWN => match message.wParam as u16 {
            VK_LEFT => {
                go_back(root);
                true
            }
            VK_RIGHT => {
                go_forward(root);
                true
            }
            _ => false,
        },
        WM_KEYDOWN if message.wParam == VK_RETURN as WPARAM && is_address(root, message.hwnd) => {
            submit_address(root);
            true
        }
        _ => false,
    }
}

fn is_address(hwnd: HWND, candidate: HWND) -> bool {
    state_mut(hwnd).is_some_and(|state| state.address == candidate)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            on_timer(hwnd);
            0
        }
        WM_SIZE => {
            if wparam != SIZE_MINIMIZED as WPARAM {
                fit_panes(hwnd);
            }
            0
        }
        WM_COMMAND => {
            on_command(hwnd, wparam);
            0
        }
        WM_NOTIFY => on_notify(hwnd, lparam),
        WM_DESTROY => {
            on_destroy(hwnd);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn on_command(hwnd: HWND, wparam: WPARAM) {
    let id = (wparam & 0xffff) as isize;
    let code = ((wparam >> 16) & 0xffff) as u32;
    if code != BN_CLICKED {
        return;
    }
    match id {
        ID_BACK => go_back(hwnd),
        ID_FORWARD => go_forward(hwnd),
        _ => {}
    }
}

fn go_back(hwnd: HWND) {
    let Some(path) = state_mut(hwnd).and_then(|state| state.history.previous().map(str::to_string))
    else {
        return;
    };
    request_navigation(hwnd, path, NavKind::Back);
}

fn go_forward(hwnd: HWND) {
    let Some(path) =
        state_mut(hwnd).and_then(|state| state.history.following().map(str::to_string))
    else {
        return;
    };
    request_navigation(hwnd, path, NavKind::Forward);
}

fn submit_address(hwnd: HWND) {
    let Some(address) = state_mut(hwnd).map(|state| state.address) else {
        return;
    };
    let Some(path) = normalize_address(&control_text(address)) else {
        return;
    };
    request_navigation(hwnd, path, NavKind::Push);
}

fn request_navigation(hwnd: HWND, path: String, kind: NavKind) {
    if path.is_empty() {
        return;
    }
    if kind == NavKind::Push && state_mut(hwnd).is_some_and(|state| state.history.current() == path)
    {
        if let Some(state) = state_mut(hwnd) {
            state.pending = None;
        }
        return;
    }
    if let Some(state) = state_mut(hwnd) {
        state.pending = Some(PendingNav { path, kind });
    }
    try_pending(hwnd);
}

/// Open `pending` without blocking. A busy volume stays pending for the timer.
/// The rows already on screen stay until this returns `Ok`.
fn try_pending(hwnd: HWND) {
    let Some(pending) = state_mut(hwnd).and_then(|state| state.pending.clone()) else {
        return;
    };
    let Some(shell) = state_mut(hwnd).map(|state| state.shell.clone()) else {
        return;
    };
    let directory = Path::new(pending.path.clone());
    let volume = VolumeId::from_directory(&directory);
    let target_key = volume.key().to_string();
    release_for_navigation(hwnd, &target_key);
    match shell.open(volume, directory) {
        Ok(opened) => {
            if let Some(state) = state_mut(hwnd) {
                state.pending = None;
                commit_history(state, &pending);
            }
            present_directory(hwnd, &pending.path, opened);
            follow_tree(hwnd, &pending.path);
            update_nav_buttons(hwnd);
        }
        Err(EnumError::VolumeBusy) => ensure_timer(hwnd),
        Err(_) => {
            if let Some(state) = state_mut(hwnd) {
                state.pending = None;
            }
        }
    }
}

fn commit_history(state: &mut ListWindow, pending: &PendingNav) {
    match pending.kind {
        NavKind::Push => {
            let _ = state.history.go(&pending.path);
        }
        NavKind::Back => {
            let _ = state.history.back();
        }
        NavKind::Forward => {
            let _ = state.history.forward();
        }
    }
}

fn present_directory(hwnd: HWND, path: &str, mut opened: OpenedDirectory) {
    let rows = DirectoryRows::from_cached(opened.cached.take());
    let shown = rows.shown().len();
    let needs_poll = !rows.is_settled();
    let Some((list, address)) = (|| {
        let state = state_mut(hwnd)?;
        cancel_thumbs(state);
        state.rows = rows;
        state.opened = Some(opened);
        state.current = path.to_string();
        Some((state.list, state.address))
    })() else {
        return;
    };
    set_count(list, shown);
    set_control_text(address, path);
    set_caption(hwnd, path);
    // Paint the cached count before the timer, or this function's caller, polls.
    unsafe { UpdateWindow(list) };
    if needs_poll {
        ensure_timer(hwnd);
    }
}

fn follow_tree(hwnd: HWND, target: &str) {
    let Some(root) = state_mut(hwnd).map(|state| state.tree_root.clone()) else {
        return;
    };
    match tree_root(&root, target) {
        TreeRoot::Replace => replace_tree_root(hwnd, target),
        TreeRoot::Keep => {
            if let Some(index) = find_node_path(hwnd, target) {
                select_node(hwnd, index);
            }
        }
    }
}

fn replace_tree_root(hwnd: HWND, path: &str) {
    let Some(tree) = tree_handle(hwnd) else {
        return;
    };
    if let Some(state) = state_mut(hwnd) {
        state.suppress_select = true;
        for node in &mut state.nodes {
            cancel_node(node);
        }
    }
    unsafe {
        SendMessageW(tree, TVM_DELETEITEM, 0, TVI_ROOT as LPARAM);
    }
    if let Some(state) = state_mut(hwnd) {
        state.nodes.clear();
        state.tree_root = path.to_string();
        state.suppress_select = false;
    }
    insert_root(hwnd, path);
    insert_network(hwnd);
}

fn select_node(hwnd: HWND, index: usize) {
    let Some((tree, item)) = node_item(hwnd, index) else {
        return;
    };
    if let Some(state) = state_mut(hwnd) {
        state.suppress_select = true;
    }
    unsafe {
        SendMessageW(tree, TVM_SELECTITEM, TVGN_CARET as WPARAM, item as LPARAM);
    }
    if let Some(state) = state_mut(hwnd) {
        state.suppress_select = false;
    }
}

/// Cancel the list, and any tree listing on `target_key`, so the next open can
/// take that volume's worker. An expand that had not inserted children can be
/// tried again. Children already inserted stay.
fn release_for_navigation(hwnd: HWND, target_key: &str) {
    let mut rearm = Vec::new();
    if let Some(state) = state_mut(hwnd) {
        if let Some(opened) = state.opened.take() {
            opened.cancel();
        }
        for (index, node) in state.nodes.iter_mut().enumerate() {
            if node.removed {
                continue;
            }
            let volume = VolumeId::from_directory(&Path::new(node.path.clone()));
            if volume.key() != target_key {
                continue;
            }
            let Some(opened) = node.opened.take() else {
                continue;
            };
            opened.cancel();
            let unfinished = node
                .expansion
                .as_ref()
                .is_some_and(|expansion| !expansion.is_settled());
            if !unfinished {
                continue;
            }
            if node.child_indices.is_empty() {
                node.expansion = None;
                rearm.push(index);
            } else if let Some(expansion) = node.expansion.as_mut() {
                expansion.stop();
            }
        }
    }
    for index in rearm {
        if let Some((tree, item)) = node_item(hwnd, index) {
            set_child_flag(tree, item, 1);
        }
    }
}

fn update_nav_buttons(hwnd: HWND) {
    let Some((back, forward, can_back, can_forward)) = state_mut(hwnd).map(|state| {
        (
            state.back,
            state.forward,
            state.history.can_back(),
            state.history.can_forward(),
        )
    }) else {
        return;
    };
    unsafe {
        EnableWindow(back, i32::from(u8::from(can_back)));
        EnableWindow(forward, i32::from(u8::from(can_forward)));
    }
}

fn on_timer(hwnd: HWND) {
    try_pending(hwnd);
    let Some(list) = list_handle(hwnd) else {
        return;
    };
    let (change, shown, tree_changes, settled_nodes, idle, thumb_ready) = {
        let Some(state) = state_mut(hwnd) else {
            return;
        };
        let change = drain(state);
        let shown = state.rows.shown().len();
        let (tree_changes, settled_nodes) = drain_tree(state);
        let thumb_ready = poll_thumbs(state);
        let idle = state.pending.is_none()
            && state.rows.is_settled()
            && !thumbs_pending(state)
            && state
                .nodes
                .iter()
                .all(|node| node.opened.is_none() && node.browse.is_none());
        (
            change,
            shown,
            tree_changes,
            settled_nodes,
            idle,
            thumb_ready,
        )
    };
    paint(list, change, shown);
    if thumb_ready {
        redraw_visible(list);
    }
    for (index, child_change) in tree_changes {
        match child_change {
            NodeChange::Directories(change) => apply_children(hwnd, index, change),
            NodeChange::Places(places) => insert_places(hwnd, index, &places),
        }
    }
    for index in settled_nodes {
        sync_expander(hwnd, index);
    }
    if idle {
        unsafe { KillTimer(hwnd, TIMER_ID) };
        if let Some(state) = state_mut(hwnd) {
            state.polling = false;
        }
    }
}

enum NodeChange {
    Directories(ChildChange),
    Places(Vec<NetworkPlace>),
}

fn drain_tree(state: &mut ListWindow) -> (Vec<(usize, NodeChange)>, Vec<usize>) {
    let mut changes = Vec::new();
    let mut settled = Vec::new();
    for index in 0..state.nodes.len() {
        if state.nodes[index].removed {
            continue;
        }
        if state.nodes[index].places.is_some() {
            let change = drain_places(&mut state.nodes[index]);
            if state.nodes[index]
                .places
                .as_ref()
                .is_some_and(PlaceList::is_settled)
            {
                state.nodes[index].browse = None;
                settled.push(index);
            }
            if let PlaceChange::Append(places) = change {
                changes.push((index, NodeChange::Places(places)));
            }
            continue;
        }
        let change = drain_node(&mut state.nodes[index]);
        if state.nodes[index]
            .expansion
            .as_ref()
            .is_some_and(Expansion::is_settled)
        {
            state.nodes[index].opened = None;
            settled.push(index);
        }
        if change != ChildChange::Unchanged {
            changes.push((index, NodeChange::Directories(change)));
        }
    }
    (changes, settled)
}

fn drain_places(node: &mut FolderNode) -> PlaceChange {
    let FolderNode { places, browse, .. } = node;
    let Some(places) = places.as_mut() else {
        return PlaceChange::Unchanged;
    };
    let Some(browse) = browse.as_mut() else {
        if !places.is_settled() {
            places.stop();
        }
        return PlaceChange::Unchanged;
    };
    places.apply_available(|| browse.try_recv())
}

fn drain_node(node: &mut FolderNode) -> ChildChange {
    let FolderNode {
        expansion, opened, ..
    } = node;
    let Some(expansion) = expansion.as_mut() else {
        return ChildChange::Unchanged;
    };
    let Some(opened) = opened.as_mut() else {
        if !expansion.is_settled() {
            expansion.stop();
        }
        return ChildChange::Unchanged;
    };
    expansion.apply_available(|| opened.poll())
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

fn on_notify(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    if lparam == 0 {
        return 0;
    }
    let code = unsafe { (*(lparam as *const NMHDR)).code };
    match code {
        NM_CUSTOMDRAW => on_custom_draw(hwnd, lparam),
        LVN_GETDISPINFOW => {
            on_list_dispinfo(hwnd, lparam);
            0
        }
        LVN_ITEMACTIVATE => {
            on_item_activate(hwnd, lparam);
            0
        }
        TVN_ITEMEXPANDINGW => on_item_expanding(hwnd, lparam),
        TVN_SELCHANGEDW => on_selection(hwnd, lparam),
        _ => 0,
    }
}

fn on_list_dispinfo(hwnd: HWND, lparam: LPARAM) {
    let info = unsafe { &mut *(lparam as *mut NMLVDISPINFOW) };
    let item = &mut info.item;
    if item.iItem < 0 || !(columns::NAME..=columns::MODIFIED).contains(&item.iSubItem) {
        return;
    }
    let subitem = item.iSubItem;
    let wants_text = item.mask & LVIF_TEXT != 0;
    let wants_image = subitem == columns::NAME && (item.mask & LVIF_IMAGE) != 0;
    if !wants_text && !wants_image {
        return;
    }
    let index = item.iItem as usize;
    let buffer = item.pszText;
    let capacity = item.cchTextMax;
    let row = state_mut(hwnd).and_then(|state| {
        state.rows.shown().get(index).map(|entry| {
            let label = if wants_text {
                columns::text(subitem, entry)
            } else {
                String::new()
            };
            let icon = if wants_image {
                Some((entry.name.clone(), entry.is_directory))
            } else {
                None
            };
            (label, icon)
        })
    });
    if wants_text {
        match &row {
            Some((label, _)) => write_name(buffer, capacity, label),
            None => write_name(buffer, capacity, ""),
        }
    }
    if wants_image {
        item.iImage = match &row {
            Some((_, Some((name, is_directory)))) => cached_icon(hwnd, name, *is_directory),
            _ => I_IMAGENONE,
        };
    }
}

/// Size and Date modified for rows the list view is painting. Subitem 0 stays
/// with the control so the extension icon is still drawn. The tree's custom
/// draw is ignored. The label is copied before DirectWrite runs.
fn on_custom_draw(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let draw = unsafe { &*(lparam as *const NMLVCUSTOMDRAW) };
    let Some(list) = list_handle(hwnd) else {
        return CDRF_DODEFAULT as LRESULT;
    };
    if draw.nmcd.hdr.hwndFrom != list {
        return CDRF_DODEFAULT as LRESULT;
    }
    match draw.nmcd.dwDrawStage {
        CDDS_PREPAINT => CDRF_NOTIFYITEMDRAW as LRESULT,
        CDDS_ITEMPREPAINT => {
            request_visible_thumb(hwnd, draw.nmcd.dwItemSpec);
            CDRF_NOTIFYSUBITEMDRAW as LRESULT
        }
        stage if stage == (CDDS_ITEMPREPAINT | CDDS_SUBITEM) => paint_subitem(hwnd, list, draw),
        stage if stage == (CDDS_ITEMPOSTPAINT | CDDS_SUBITEM) => overlay_thumb(hwnd, list, draw),
        _ => CDRF_DODEFAULT as LRESULT,
    }
}

fn paint_subitem(hwnd: HWND, list: HWND, draw: &NMLVCUSTOMDRAW) -> LRESULT {
    let subitem = draw.iSubItem;
    if subitem == columns::NAME {
        return CDRF_NOTIFYPOSTPAINT as LRESULT;
    }
    if subitem != columns::SIZE && subitem != columns::MODIFIED {
        return CDRF_DODEFAULT as LRESULT;
    }
    let row = draw.nmcd.dwItemSpec;
    let text = state_mut(hwnd).and_then(|state| {
        state
            .rows
            .shown()
            .get(row)
            .map(|entry| columns::text(subitem, entry))
    });
    let Some(text) = text.filter(|text| !text.is_empty()) else {
        return CDRF_DODEFAULT as LRESULT;
    };
    let selected = row_selected(list, row);
    let bounds = cell_rect(draw.nmcd.rc);
    let text_bounds = cell_rect(draw.rcText);
    if draw::paint_text(draw.nmcd.hdc, bounds, text_bounds, &text, selected) {
        CDRF_SKIPDEFAULT as LRESULT
    } else {
        CDRF_DODEFAULT as LRESULT
    }
}

fn row_selected(list: HWND, row: usize) -> bool {
    let state = unsafe {
        SendMessageW(
            list,
            LVM_GETITEMSTATE,
            row as WPARAM,
            LVIS_SELECTED as LPARAM,
        )
    };
    state & LVIS_SELECTED as LRESULT != 0
}

fn cell_rect(rect: RECT) -> CellRect {
    CellRect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

fn cancel_thumbs(state: &mut ListWindow) {
    for job in state.thumb_jobs.values() {
        job.ticket.cancel();
    }
    state.thumb_jobs.clear();
}

fn thumbs_pending(state: &ListWindow) -> bool {
    state.thumb_jobs.values().any(|job| !job.settled)
}

/// `try_recv` never waits. A transition to Ready asks for one redraw.
fn poll_thumbs(state: &mut ListWindow) -> bool {
    let mut ready = false;
    for job in state.thumb_jobs.values_mut() {
        if job.settled {
            continue;
        }
        match job.ticket.try_recv() {
            TryThumb::Pending => {}
            TryThumb::Ready(_) => {
                job.settled = true;
                ready = true;
            }
            TryThumb::Failed | TryThumb::Cancelled => job.settled = true,
        }
    }
    ready
}

/// Visible rows only: the list view sends item prepaint for those rows.
/// `request` does not decode and does not sleep.
fn request_visible_thumb(hwnd: HWND, row: usize) {
    let Some(path) = state_mut(hwnd).and_then(|state| thumb_path(state, row)) else {
        return;
    };
    let pending = state_mut(hwnd).is_some_and(|state| {
        if state.thumb_jobs.contains_key(&path) {
            return false;
        }
        let ticket = state.thumbs.request(&path);
        let settled = !matches!(ticket.try_recv(), TryThumb::Pending);
        state.thumb_jobs.insert(path, ThumbJob { ticket, settled });
        !settled
    });
    if pending {
        ensure_timer(hwnd);
    }
}

fn thumb_path(state: &ListWindow, row: usize) -> Option<String> {
    let entry = state.rows.shown().get(row)?;
    if entry.is_directory || !images::wants_thumbnail(&entry.name) {
        return None;
    }
    Some(child_directory(&state.current, &entry.name))
}

fn overlay_thumb(hwnd: HWND, list: HWND, draw: &NMLVCUSTOMDRAW) -> LRESULT {
    if draw.iSubItem != columns::NAME {
        return CDRF_DODEFAULT as LRESULT;
    }
    let row = draw.nmcd.dwItemSpec;
    let rgba = state_mut(hwnd).and_then(|state| {
        let path = thumb_path(state, row)?;
        match state.thumb_jobs.get(&path)?.ticket.try_recv() {
            TryThumb::Ready(rgba) => Some(rgba),
            _ => None,
        }
    });
    let Some(rgba) = rgba else {
        return CDRF_DODEFAULT as LRESULT;
    };
    let Some(bounds) = icon_rect(list, row) else {
        return CDRF_DODEFAULT as LRESULT;
    };
    let _ = draw::paint_bitmap(draw.nmcd.hdc, bounds, rgba.width, rgba.height, &rgba.pixels);
    CDRF_DODEFAULT as LRESULT
}

fn icon_rect(list: HWND, row: usize) -> Option<CellRect> {
    let mut rect = RECT {
        left: columns::NAME,
        top: LVIR_ICON as i32,
        right: 0,
        bottom: 0,
    };
    let found = unsafe {
        SendMessageW(
            list,
            LVM_GETSUBITEMRECT,
            row as WPARAM,
            &mut rect as *mut RECT as LPARAM,
        )
    };
    if found == 0 || rect.right <= rect.left || rect.bottom <= rect.top {
        None
    } else {
        Some(cell_rect(rect))
    }
}

fn redraw_visible(list: HWND) {
    let top = unsafe { SendMessageW(list, LVM_GETTOPINDEX, 0, 0) };
    let page = unsafe { SendMessageW(list, LVM_GETCOUNTPERPAGE, 0, 0) };
    if top < 0 || page <= 0 {
        return;
    }
    let last = top.saturating_add(page).saturating_sub(1);
    unsafe {
        SendMessageW(list, LVM_REDRAWITEMS, top as WPARAM, last as LPARAM);
    }
}

/// The image list index for this row. `SHGetFileInfoW` runs once per [`IconKey`].
/// The borrow of the window is dropped first, because that call can reenter.
fn cached_icon(hwnd: HWND, name: &str, is_directory: bool) -> i32 {
    let key = icon_key(name, is_directory);
    if let Some(index) = state_mut(hwnd).and_then(|state| state.icons.get(&key).copied()) {
        return index;
    }
    let Some(images) = state_mut(hwnd)
        .map(|state| state.images)
        .filter(|images| *images != 0)
    else {
        return I_IMAGENONE;
    };
    let index = load_shell_icon(images, &key);
    state_mut(hwnd)
        .map(|state| *state.icons.get_or_insert_with(key, || index))
        .unwrap_or(index)
}

fn create_image_list(list: HWND) -> HIMAGELIST {
    let mut cx = unsafe { GetSystemMetrics(SM_CXSMICON) };
    let mut cy = unsafe { GetSystemMetrics(SM_CYSMICON) };
    if cx <= 0 {
        cx = 16;
    }
    if cy <= 0 {
        cy = 16;
    }
    let images = unsafe { ImageList_Create(cx, cy, ILC_COLOR32 | ILC_MASK, 4, 4) };
    if images != 0 {
        unsafe {
            SendMessageW(list, LVM_SETIMAGELIST, LVSIL_SMALL as WPARAM, images);
        }
    }
    images
}

/// Attributes only. The name is synthetic, so this does not open the file.
fn load_shell_icon(images: HIMAGELIST, key: &IconKey) -> i32 {
    ensure_com();
    let (label, attributes) = match key {
        IconKey::Directory => ("folder".to_string(), FILE_ATTRIBUTE_DIRECTORY),
        IconKey::File => ("file".to_string(), FILE_ATTRIBUTE_NORMAL),
        IconKey::Extension(extension) => (format!("file.{extension}"), FILE_ATTRIBUTE_NORMAL),
    };
    let wide_name = wide(&label);
    let mut info = SHFILEINFOW {
        hIcon: ptr::null_mut(),
        iIcon: 0,
        dwAttributes: 0,
        szDisplayName: [0; 260],
        szTypeName: [0; 80],
    };
    let flags = SHGFI_USEFILEATTRIBUTES | SHGFI_ICON | SHGFI_SMALLICON;
    let found = unsafe {
        SHGetFileInfoW(
            wide_name.as_ptr(),
            attributes,
            &mut info,
            mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        )
    };
    if found == 0 || info.hIcon.is_null() {
        return I_IMAGENONE;
    }
    let index = unsafe { ImageList_ReplaceIcon(images, -1, info.hIcon) };
    unsafe {
        DestroyIcon(info.hIcon);
    }
    if index < 0 {
        I_IMAGENONE
    } else {
        index
    }
}

fn ensure_com() {
    use std::sync::Once;
    static START: Once = Once::new();
    START.call_once(|| {
        let _ = unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    });
}

fn on_item_activate(hwnd: HWND, lparam: LPARAM) {
    let info = unsafe { &*(lparam as *const NMLISTVIEW) };
    if info.iItem < 0 {
        return;
    }
    let Some((current, name)) = activated_directory(hwnd, info.iItem as usize) else {
        return;
    };
    request_navigation(hwnd, child_directory(&current, &name), NavKind::Push);
}

fn activated_directory(hwnd: HWND, index: usize) -> Option<(String, String)> {
    let state = state_mut(hwnd)?;
    let entry = state.rows.shown().get(index)?;
    if !entry.is_directory || entry.name.is_empty() {
        return None;
    }
    Some((state.current.clone(), entry.name.clone()))
}

fn on_selection(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    if state_mut(hwnd).is_some_and(|state| state.suppress_select) {
        return 0;
    }
    let info = unsafe { &*(lparam as *const NMTREEVIEWW) };
    let item = info.itemNew.hItem;
    if item == 0 {
        return 0;
    }
    let Some(index) = find_node(hwnd, item) else {
        return 0;
    };
    let Some(path) = navigable_path(hwnd, index) else {
        return 0;
    };
    if state_mut(hwnd).is_some_and(|state| same_directory(&state.current, &path)) {
        return 0;
    }
    request_navigation(hwnd, path, NavKind::Push);
    0
}

/// `TRUE` refuses the expand. A busy volume is refused instead of waited on.
fn on_item_expanding(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let info = unsafe { &*(lparam as *const NMTREEVIEWW) };
    if info.action & TVE_EXPAND == 0 || info.action & TVE_COLLAPSE != 0 {
        return 0;
    }
    let item = info.itemNew.hItem;
    let Some(index) = find_node(hwnd, item) else {
        return 0;
    };
    let Some(kind) = expand_kind(hwnd, index) else {
        return 0;
    };
    match kind {
        NodeKind::Directory => expand_directory(hwnd, index),
        NodeKind::NetworkRoot => start_browse(hwnd, index, None),
        NodeKind::Place {
            container: true,
            unc: Some(unc),
        } => start_browse(hwnd, index, Some(unc.as_str())),
        NodeKind::Place { .. } => 1,
    }
}

/// `None` means this node already started, so the expand is allowed.
fn expand_kind(hwnd: HWND, index: usize) -> Option<NodeKind> {
    let state = state_mut(hwnd)?;
    let node = state.nodes.get(index)?;
    if node.expansion.is_some() || node.places.is_some() {
        return None;
    }
    Some(node.kind.clone())
}

fn expand_directory(hwnd: HWND, index: usize) -> LRESULT {
    let Some(path) = node_path(hwnd, index) else {
        return 1;
    };
    let Some(shell) = state_mut(hwnd).map(|state| state.shell.clone()) else {
        return 1;
    };
    let directory = Path::new(path.clone());
    let opened = match shell.open(VolumeId::from_directory(&directory), directory) {
        Ok(opened) => opened,
        Err(_) => return 1,
    };
    begin_expansion(hwnd, index, opened);
    0
}

fn start_browse(hwnd: HWND, index: usize, parent: Option<&str>) -> LRESULT {
    let browse = match Shell::browse_network(parent) {
        Ok(browse) => browse,
        Err(_) => return 1,
    };
    if let Some(state) = state_mut(hwnd) {
        state.nodes[index].places = Some(PlaceList::start());
        state.nodes[index].browse = Some(browse);
    }
    ensure_timer(hwnd);
    0
}

fn begin_expansion(hwnd: HWND, index: usize, mut opened: OpenedDirectory) {
    let cached = opened.cached.take();
    let (expansion, change) = Expansion::start(cached.as_deref());
    let needs_poll = !expansion.is_settled();
    if let Some(state) = state_mut(hwnd) {
        state.nodes[index].expansion = Some(expansion);
        state.nodes[index].opened = Some(opened);
    }
    if let ChildChange::Append(names) = change {
        insert_directories(hwnd, index, &names);
    }
    if needs_poll {
        ensure_timer(hwnd);
    }
}

fn apply_children(hwnd: HWND, index: usize, change: ChildChange) {
    match change {
        ChildChange::Unchanged => {}
        ChildChange::Append(names) => insert_directories(hwnd, index, &names),
        ChildChange::Replace(names) => {
            delete_children(hwnd, index);
            insert_directories(hwnd, index, &names);
        }
    }
    sync_expander(hwnd, index);
}

/// The Network root is not a child of the opened directory, and it is not browsed here.
fn insert_network(hwnd: HWND) {
    let Some(tree) = tree_handle(hwnd) else {
        return;
    };
    let index = {
        let Some(state) = state_mut(hwnd) else {
            return;
        };
        let index = state.nodes.len();
        state.nodes.push(FolderNode::network());
        index
    };
    let item = insert_tree_item(tree, TVI_ROOT, NETWORK_LABEL, 1, index);
    if let Some(state) = state_mut(hwnd) {
        state.nodes[index].item = item;
    }
}

fn insert_root(hwnd: HWND, path: &str) {
    let Some(tree) = tree_handle(hwnd) else {
        return;
    };
    let index = {
        let Some(state) = state_mut(hwnd) else {
            return;
        };
        let index = state.nodes.len();
        state.nodes.push(FolderNode::new(path.to_string()));
        index
    };
    let item = insert_tree_item(tree, TVI_ROOT, path, 1, index);
    if let Some(state) = state_mut(hwnd) {
        state.nodes[index].item = item;
    }
}

fn insert_directories(hwnd: HWND, parent: usize, names: &[String]) {
    let Some((tree, parent_item, parent_path)) = node_insert_target(hwnd, parent) else {
        return;
    };
    for name in names {
        if name.is_empty() {
            continue;
        }
        let path = child_directory(&parent_path, name);
        let index = {
            let Some(state) = state_mut(hwnd) else {
                return;
            };
            let index = state.nodes.len();
            state.nodes.push(FolderNode::new(path));
            index
        };
        let item = insert_tree_item(tree, parent_item, name, 1, index);
        if let Some(state) = state_mut(hwnd) {
            state.nodes[index].item = item;
            state.nodes[parent].child_indices.push(index);
        }
    }
}

fn insert_places(hwnd: HWND, parent: usize, places: &[NetworkPlace]) {
    let Some((tree, parent_item, _)) = node_insert_target(hwnd, parent) else {
        return;
    };
    for place in places {
        if place.name.is_empty() {
            continue;
        }
        let expandable = place.container && place.unc.is_some();
        let index = {
            let Some(state) = state_mut(hwnd) else {
                return;
            };
            let index = state.nodes.len();
            state.nodes.push(FolderNode::place(place));
            index
        };
        let item = insert_tree_item(
            tree,
            parent_item,
            &place.name,
            i32::from(u8::from(expandable)),
            index,
        );
        if let Some(state) = state_mut(hwnd) {
            state.nodes[index].item = item;
            state.nodes[parent].child_indices.push(index);
        }
    }
    sync_expander(hwnd, parent);
}

fn delete_children(hwnd: HWND, parent: usize) {
    let children = {
        let Some(state) = state_mut(hwnd) else {
            return;
        };
        mem::take(&mut state.nodes[parent].child_indices)
    };
    for child in children {
        delete_subtree(hwnd, child);
    }
}

fn delete_subtree(hwnd: HWND, index: usize) {
    let children = {
        let Some(state) = state_mut(hwnd) else {
            return;
        };
        if state.nodes[index].removed {
            return;
        }
        mem::take(&mut state.nodes[index].child_indices)
    };
    for child in children {
        delete_subtree(hwnd, child);
    }
    let Some((tree, item)) = take_removed_item(hwnd, index) else {
        return;
    };
    if item != 0 {
        unsafe {
            SendMessageW(tree, TVM_DELETEITEM, 0, item as LPARAM);
        }
    }
}

fn take_removed_item(hwnd: HWND, index: usize) -> Option<(HWND, HTREEITEM)> {
    let state = state_mut(hwnd)?;
    let tree = state.tree;
    let node = state.nodes.get_mut(index)?;
    if node.removed {
        return None;
    }
    node.removed = true;
    cancel_node(node);
    Some((tree, node.item))
}

fn cancel_node(node: &mut FolderNode) {
    if let Some(opened) = node.opened.take() {
        opened.cancel();
    }
    if let Some(browse) = node.browse.take() {
        browse.cancel();
    }
}

fn sync_expander(hwnd: HWND, index: usize) {
    let Some((tree, item, empty, settled)) = expander_state(hwnd, index) else {
        return;
    };
    if !settled {
        return;
    }
    set_child_flag(tree, item, if empty { 0 } else { 1 });
}

fn expander_state(hwnd: HWND, index: usize) -> Option<(HWND, HTREEITEM, bool, bool)> {
    let state = state_mut(hwnd)?;
    let tree = state.tree;
    let node = state.nodes.get(index)?;
    if node.removed || node.item == 0 {
        return None;
    }
    let empty = node.child_indices.is_empty();
    let settled = node.expansion.as_ref().is_some_and(Expansion::is_settled)
        || node.places.as_ref().is_some_and(PlaceList::is_settled);
    Some((tree, node.item, empty, settled))
}

fn insert_tree_item(
    tree: HWND,
    parent: HTREEITEM,
    text: &str,
    children: i32,
    index: usize,
) -> HTREEITEM {
    let mut text = wide(text);
    let item = TVITEMW {
        mask: TVIF_TEXT | TVIF_CHILDREN | TVIF_PARAM,
        hItem: 0,
        state: 0,
        stateMask: 0,
        pszText: text.as_mut_ptr(),
        cchTextMax: 0,
        iImage: 0,
        iSelectedImage: 0,
        cChildren: children,
        lParam: index as LPARAM,
    };
    let insert = TVINSERTSTRUCTW {
        hParent: parent,
        hInsertAfter: TVI_LAST,
        Anonymous: TVINSERTSTRUCTW_0 { item },
    };
    unsafe {
        SendMessageW(
            tree,
            TVM_INSERTITEMW,
            0,
            &insert as *const TVINSERTSTRUCTW as LPARAM,
        ) as HTREEITEM
    }
}

fn set_child_flag(tree: HWND, item: HTREEITEM, children: i32) {
    let update = TVITEMW {
        mask: TVIF_HANDLE | TVIF_CHILDREN,
        hItem: item,
        state: 0,
        stateMask: 0,
        pszText: ptr::null_mut(),
        cchTextMax: 0,
        iImage: 0,
        iSelectedImage: 0,
        cChildren: children,
        lParam: 0,
    };
    unsafe {
        SendMessageW(tree, TVM_SETITEMW, 0, &update as *const TVITEMW as LPARAM);
    }
}

fn ensure_timer(hwnd: HWND) {
    let Some(state) = state_mut(hwnd) else {
        return;
    };
    if state.polling {
        return;
    }
    if unsafe { SetTimer(hwnd, TIMER_ID, POLL_INTERVAL_MS, None) } == 0 {
        eprintln!("list_shell could not start polling: {}", last_error());
        return;
    }
    if let Some(state) = state_mut(hwnd) {
        state.polling = true;
    }
}

fn node_path(hwnd: HWND, index: usize) -> Option<String> {
    let state = state_mut(hwnd)?;
    let node = state.nodes.get(index)?;
    if node.removed || !matches!(node.kind, NodeKind::Directory) {
        None
    } else {
        Some(node.path.clone())
    }
}

/// A directory, or a connectable share. Containers and the Network root do not list.
fn navigable_path(hwnd: HWND, index: usize) -> Option<String> {
    let state = state_mut(hwnd)?;
    let node = state.nodes.get(index)?;
    if node.removed {
        return None;
    }
    match &node.kind {
        NodeKind::Directory => Some(node.path.clone()),
        NodeKind::Place {
            unc,
            container: false,
        } => unc.clone(),
        NodeKind::NetworkRoot | NodeKind::Place { .. } => None,
    }
}

fn node_item(hwnd: HWND, index: usize) -> Option<(HWND, HTREEITEM)> {
    let state = state_mut(hwnd)?;
    let tree = state.tree;
    let node = state.nodes.get(index)?;
    if node.removed || node.item == 0 {
        None
    } else {
        Some((tree, node.item))
    }
}

fn node_insert_target(hwnd: HWND, index: usize) -> Option<(HWND, HTREEITEM, String)> {
    let state = state_mut(hwnd)?;
    let tree = state.tree;
    let node = state.nodes.get(index)?;
    if node.removed || node.item == 0 {
        return None;
    }
    Some((tree, node.item, node.path.clone()))
}

fn find_node(hwnd: HWND, item: HTREEITEM) -> Option<usize> {
    let state = state_mut(hwnd)?;
    state
        .nodes
        .iter()
        .position(|node| !node.removed && node.item == item)
}

fn find_node_path(hwnd: HWND, path: &str) -> Option<usize> {
    let state = state_mut(hwnd)?;
    state.nodes.iter().position(|node| {
        !node.removed
            && matches!(node.kind, NodeKind::Directory)
            && same_directory(&node.path, path)
    })
}

fn tree_handle(hwnd: HWND) -> Option<HWND> {
    state_mut(hwnd).map(|state| state.tree)
}

fn fit_panes(hwnd: HWND) {
    let Some(list) = list_handle(hwnd) else {
        return;
    };
    let Some((tree, back, forward, address)) =
        state_mut(hwnd).map(|state| (state.tree, state.back, state.forward, state.address))
    else {
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
        MoveWindow(back, 0, 0, BACK_WIDTH, NAV_HEIGHT, 1);
        let forward_x = BACK_WIDTH + NAV_GAP;
        MoveWindow(forward, forward_x, 0, FORWARD_WIDTH, NAV_HEIGHT, 1);
        let address_x = forward_x + FORWARD_WIDTH + NAV_GAP;
        let address_width = (width - address_x).max(0);
        MoveWindow(address, address_x, 0, address_width, NAV_HEIGHT, 1);
        let top = NAV_HEIGHT + NAV_GAP;
        let body = (height - top).max(0);
        let tree_width = TREE_WIDTH.min(width.max(0));
        let list_width = (width - tree_width).max(0);
        MoveWindow(tree, 0, top, tree_width, body, 1);
        MoveWindow(list, tree_width, top, list_width, body, 1);
        let fixed = columns::SIZE_WIDTH + columns::MODIFIED_WIDTH;
        let name_width = (list_width - fixed).max(80);
        SendMessageW(
            list,
            LVM_SETCOLUMNWIDTH,
            columns::NAME as WPARAM,
            name_width as LPARAM,
        );
        SendMessageW(
            list,
            LVM_SETCOLUMNWIDTH,
            columns::SIZE as WPARAM,
            columns::SIZE_WIDTH as LPARAM,
        );
        SendMessageW(
            list,
            LVM_SETCOLUMNWIDTH,
            columns::MODIFIED as WPARAM,
            columns::MODIFIED_WIDTH as LPARAM,
        );
    }
}

fn on_destroy(hwnd: HWND) {
    let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut ListWindow;
    if !raw.is_null() {
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
        let mut state = unsafe { Box::from_raw(raw) };
        cancel_thumbs(&mut state);
        if state.polling {
            unsafe { KillTimer(hwnd, TIMER_ID) };
        }
        if let Some(opened) = &state.opened {
            opened.cancel();
        }
        for node in &state.nodes {
            if let Some(opened) = &node.opened {
                opened.cancel();
            }
            if let Some(browse) = &node.browse {
                browse.cancel();
            }
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

fn set_caption(hwnd: HWND, path: &str) {
    let title = wide(&format!("Rapid Explorer — {path}"));
    unsafe { SetWindowTextW(hwnd, title.as_ptr()) };
}

fn set_control_text(hwnd: HWND, text: &str) {
    let text = wide(text);
    unsafe { SetWindowTextW(hwnd, text.as_ptr()) };
}

fn control_text(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; len as usize + 1];
    let written = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if written <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..written as usize])
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
