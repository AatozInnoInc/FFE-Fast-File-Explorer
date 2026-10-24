//! One top-level window: address bar, folder tree, and two virtual lists.
//!
//! The message loop is `GetMessageW`. Enumeration is `Shell::poll` on a timer.
//! That call does not wait, and this thread does not read a directory. Choosing
//! a folder, typing a path, or going back lists that directory. A cache hit sets
//! the owner-data count and paints before the next `poll`. A row's icon comes
//! from one cached image per extension, or one shared directory icon. Size and
//! Date modified are DirectWrite cells for the rows the list view asks to paint.
//! The Name column stays with the list view so the icon remains. An image in a
//! visible row requests a thumbnail on the background pool, and that buffer
//! covers the icon once it is ready. The list's context menu copies, cuts,
//! pastes, renames, deletes, and opens properties. More options asks a helper
//! process supervised by the watchdog. That helper returns no verbs and does
//! not load a shell extension. A Timeout button stores the network timeout.
//! Local volumes do not use it, and a timeout is not retried. A search box
//! stores typed text and does not rank on the keystroke. The timer copies
//! settled local listings into the filename index and ranks those names. An
//! empty or whitespace query leaves the directory listing in place. Names
//! that later disappear stay indexed. The left and right lists start at the
//! same path. Left is active. Switching panes changes focus and leaves both
//! paths as they are. Each pane has its own tabs. A new tab remembers that
//! pane's path, and choosing a tab lists that path in that pane only. A
//! Network node stays collapsed until the user expands it.

use std::collections::HashMap;
use std::mem;
use std::process::ExitCode;
use std::ptr;
use std::time::Instant;

use windows_sys::core::w;
use windows_sys::Win32::Foundation::{
    GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{ScreenToClient, UpdateWindow, COLOR_WINDOW, HBRUSH};
use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{
    ImageList_Create, ImageList_ReplaceIcon, InitCommonControlsEx, CDDS_ITEMPOSTPAINT,
    CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDDS_SUBITEM, CDRF_DODEFAULT, CDRF_NOTIFYITEMDRAW,
    CDRF_NOTIFYPOSTPAINT, CDRF_NOTIFYSUBITEMDRAW, CDRF_SKIPDEFAULT, HIMAGELIST, HTREEITEM,
    ICC_LISTVIEW_CLASSES, ICC_TREEVIEW_CLASSES, ILC_COLOR32, ILC_MASK, INITCOMMONCONTROLSEX,
    I_IMAGENONE, LVCFMT_LEFT, LVCF_FMT, LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVHITTESTINFO,
    LVIF_IMAGE, LVIF_STATE, LVIF_TEXT, LVIR_ICON, LVIS_SELECTED, LVITEMW, LVM_EDITLABELW,
    LVM_GETCOUNTPERPAGE, LVM_GETITEMSTATE, LVM_GETNEXTITEM, LVM_GETSELECTEDCOUNT,
    LVM_GETSUBITEMRECT, LVM_GETTOPINDEX, LVM_HITTEST, LVM_INSERTCOLUMNW, LVM_REDRAWITEMS,
    LVM_SETCOLUMNWIDTH, LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETIMAGELIST, LVM_SETITEMCOUNT,
    LVM_SETITEMSTATE, LVNI_SELECTED, LVN_ENDLABELEDITW, LVN_GETDISPINFOW, LVN_ITEMACTIVATE,
    LVSICF_NOSCROLL, LVSIL_SMALL, LVS_EDITLABELS, LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT,
    LVS_NOSORTHEADER, LVS_OWNERDATA, LVS_REPORT, LVS_SHOWSELALWAYS, NMHDR, NMLISTVIEW,
    NMLVCUSTOMDRAW, NMLVDISPINFOW, NMTREEVIEWW, NM_CUSTOMDRAW, TCM_GETCURSEL, TVE_COLLAPSE,
    TVE_EXPAND, TVGN_CARET, TVIF_CHILDREN, TVIF_HANDLE, TVIF_PARAM, TVIF_TEXT, TVINSERTSTRUCTW,
    TVINSERTSTRUCTW_0, TVITEMW, TVI_LAST, TVI_ROOT, TVM_DELETEITEM, TVM_INSERTITEMW,
    TVM_SELECTITEM, TVM_SETITEMW, TVN_ITEMEXPANDINGW, TVN_SELCHANGEDW, TVS_DISABLEDRAGDROP,
    TVS_HASBUTTONS, TVS_HASLINES, TVS_LINESATROOT, TVS_SHOWSELALWAYS, WC_BUTTONW, WC_EDITW,
    WC_LISTVIEWW, WC_TREEVIEWW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, GetKeyState, SetFocus, VK_CONTROL, VK_F6, VK_LEFT, VK_MENU, VK_RETURN,
    VK_RIGHT, VK_SHIFT, VK_T, VK_TAB, VK_W,
};
use windows_sys::Win32::UI::Shell::{
    SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON, SHGFI_USEFILEATTRIBUTES,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, DispatchMessageW, GetAncestor, GetClientRect, GetCursorPos, GetMessageW,
    GetSystemMetrics, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, KillTimer,
    LoadCursorW, MessageBoxW, MoveWindow, PostQuitMessage, RegisterClassW, SendMessageW, SetTimer,
    SetWindowLongPtrW, SetWindowTextW, ShowWindow, TrackPopupMenu, TranslateMessage, BN_CLICKED,
    BS_PUSHBUTTON, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, EN_CHANGE, ES_AUTOHSCROLL, GA_ROOT,
    GWLP_USERDATA, GWL_STYLE, HMENU, IDC_ARROW, IDYES, MB_ICONWARNING, MB_YESNO, MF_SEPARATOR,
    MF_STRING, MSG, SIZE_MINIMIZED, SM_CXSMICON, SM_CYSMICON, SW_SHOW, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WINDOW_STYLE, WM_COMMAND, WM_CONTEXTMENU, WM_DESTROY,
    WM_GETFONT, WM_KEYDOWN, WM_NOTIFY, WM_SIZE, WM_SYSKEYDOWN, WM_TIMER, WNDCLASSW, WS_BORDER,
    WS_CHILD, WS_CLIPCHILDREN, WS_EX_CLIENTEDGE, WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE,
};

use fx_core::filename_index::{FilenameIndex, IndexRecord};
use fx_core::helper::HelperSession;
use fx_core::settings::TimeoutSetting;
use fx_core::thumbs::{ThumbCache, ThumbTicket, TryThumb};
use fx_core::VolumeKind;
use fx_core::{EnumError, NetworkBrowse, NetworkPlace, OpenedDirectory, Path, Shell, VolumeId};

use crate::cell;
use crate::columns;
use crate::commands::{self, Command};
use crate::draw::{self, CellRect};
use crate::file_ops::{self, FileOp, Op};
use crate::icons::{icon_key, IconCache, IconKey};
use crate::images;
use crate::nav::{normalize_address, same_directory, tree_root, History, TreeRoot};
use crate::panes::{PaneId, Panes};
use crate::places::{PlaceChange, PlaceList};
use crate::query::Query;
use crate::rows::{DirectoryRows, RowChange};
use crate::tab_strip;
use crate::tabs::PaneTabs;
use crate::timeout;
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
const ID_LIST_RIGHT: isize = 8;
const ID_TREE: isize = 2;
const ID_BACK: isize = 3;
const ID_FORWARD: isize = 4;
const ID_ADDRESS: isize = 5;
const ID_TIMEOUT: isize = 6;
const ID_SEARCH: isize = 7;
const ID_OTHER: isize = 9;
/// `TCN_SELCHANGE`. The tab strip sends this after the selection moves.
const TCN_SELCHANGE: u32 = 0xFFFF_FDD9;
/// Width of the search box. Not a performance budget.
const SEARCH_WIDTH: i32 = 180;
/// Width of the button that switches the active pane. Not a performance budget.
const OTHER_WIDTH: i32 = 64;
/// `NM_SETFOCUS`. A list sends this when it gains focus.
const NM_SETFOCUS: u32 = 4294967289;
/// `EM_SETCUEBANNER`.
const EM_SETCUEBANNER: u32 = 0x1501;
/// Width of the Timeout button. Not a performance budget.
const TIMEOUT_WIDTH: i32 = 84;
const NETWORK_LABEL: &str = "Network";
const CLASS_NAME: windows_sys::core::PCWSTR = w!("RapidExplorerList");

struct ListWindow {
    shell: Shell,
    /// Which pane receives navigation, and the path stored for each pane.
    panes: Panes,
    left: PaneView,
    right: PaneView,
    tree: HWND,
    back: HWND,
    forward: HWND,
    address: HWND,
    /// Path of the tree's root item. Not rewritten when a descendant is listed.
    tree_root: String,
    /// `TVM_SELECTITEM` reenters with `TVN_SELCHANGED`. That must not navigate again.
    suppress_select: bool,
    /// Image-list indexes. One directory icon, and one icon per extension.
    icons: IconCache<i32>,
    /// Small-icon image list shared by both lists. Zero when it was not created.
    images: HIMAGELIST,
    nodes: Vec<FolderNode>,
    /// The timer is running. This is not "a list is still reading".
    polling: bool,
    /// RGBA thumbnails. The cache is unbounded. Eviction: TODO — define.
    thumbs: ThumbCache,
    /// Paste, delete, or rename running off this thread. At most one.
    file_op: Option<FileOp>,
    /// More options helper. At most one. Drop kills it and does not join.
    menu_host: Option<HelperSession>,
    /// Network-timeout button.
    timeout: HWND,
    /// Button that switches the active pane without changing either path.
    other: HWND,
    /// `rapid-explorer-settings.txt` beside this executable.
    settings_file: std::path::PathBuf,
    /// Search box. Typing does not rank; the timer does.
    search: HWND,
    query: Query,
    /// Names this window has already listed. Eviction: TODO — define.
    index: FilenameIndex,
    /// Ranked records while [`Query::is_active`], drawn in the active list.
    search_hits: Vec<IndexRecord>,
    /// Query text already ranked. Differs from [`Query::text`] until the timer runs.
    search_applied: String,
}

/// One list, its history, and the listing it is showing.
struct PaneView {
    list: HWND,
    history: History,
    /// Directory whose rows are on screen. Unchanged until a navigation opens.
    current: String,
    /// A path whose volume worker was busy. The timer retries it. The list stays.
    pending: Option<PendingNav>,
    opened: Option<OpenedDirectory>,
    rows: DirectoryRows,
    /// Tickets for this pane's visible rows. Navigation of this pane cancels them.
    thumb_jobs: HashMap<String, ThumbJob>,
    /// Last directory copied from this pane into the window index.
    indexed_path: String,
    /// Paths remembered by this pane's tabs. One list, not a third pane.
    tabs: PaneTabs,
    /// Tab strip above this pane's list.
    tab_strip: HWND,
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
    /// Re-list the current directory after a file operation. Does not push history.
    Refresh,
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
    let settings_file = fx_core::settings::beside_exe()
        .unwrap_or_else(|| std::path::PathBuf::from("rapid-explorer-settings.txt"));
    let mut shell = Shell::new();
    let setting = TimeoutSetting::load(&settings_file).unwrap_or_default();
    shell.set_network_timeout(setting);
    let start = match shell.open(VolumeId::from_directory(&path), path.clone()) {
        Ok(opened) => WindowStart::opened(shell, path, opened, settings_file),
        Err(err) => WindowStart::failed(shell, path, err, settings_file),
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
    settings_file: std::path::PathBuf,
}

impl WindowStart {
    fn opened(
        shell: Shell,
        path: Path,
        mut opened: OpenedDirectory,
        settings_file: std::path::PathBuf,
    ) -> Self {
        let rows = DirectoryRows::from_cached(opened.cached.take());
        Self {
            shell,
            root: path.as_str().to_string(),
            caption: format!("Rapid Explorer — {}", path.as_str()),
            opened: Some(opened),
            rows,
            settings_file,
        }
    }

    fn failed(shell: Shell, path: Path, err: EnumError, settings_file: std::path::PathBuf) -> Self {
        let mut rows = DirectoryRows::from_cached(None);
        rows.stop();
        Self {
            shell,
            root: path.as_str().to_string(),
            caption: format!("Rapid Explorer — {} — {err}", path.as_str()),
            opened: None,
            rows,
            settings_file,
        }
    }
}

fn attach(hwnd: HWND, start: WindowStart) -> Result<(), u8> {
    let controls = INITCOMMONCONTROLSEX {
        dwSize: mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_LISTVIEW_CLASSES | ICC_TREEVIEW_CLASSES | tab_strip::TAB_CLASSES,
    };
    let _ = unsafe { InitCommonControlsEx(&controls) };

    let list = create_list(hwnd, ID_LIST, "left list")?;
    let right_list = create_list(hwnd, ID_LIST_RIGHT, "right list")?;
    let images = create_image_list(list);
    if images != 0 {
        unsafe {
            SendMessageW(right_list, LVM_SETIMAGELIST, LVSIL_SMALL as WPARAM, images);
        }
    }

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
    let timeout = create_control(
        hwnd,
        WC_BUTTONW,
        "Timeout",
        button_style(),
        0,
        ID_TIMEOUT,
        "timeout button",
    )?;
    let other = create_control(
        hwnd,
        WC_BUTTONW,
        "Other",
        button_style(),
        0,
        ID_OTHER,
        "other pane button",
    )?;
    let search = create_control(
        hwnd,
        WC_EDITW,
        "",
        address_style(),
        WS_EX_CLIENTEDGE,
        ID_SEARCH,
        "search box",
    )?;
    let left_tabs = tab_strip::create(hwnd, tab_strip::ID_TABS_LEFT)?;
    let right_tabs = tab_strip::create(hwnd, tab_strip::ID_TABS_RIGHT)?;
    let cue = wide("Search");
    unsafe {
        SendMessageW(search, EM_SETCUEBANNER, 1, cue.as_ptr() as LPARAM);
    }

    let root = start.root.clone();
    let mut right_rows = DirectoryRows::from_cached(None);
    right_rows.stop();
    let polling = !start.rows.is_settled();
    let state = Box::new(ListWindow {
        shell: start.shell,
        panes: Panes::start(root.clone()),
        left: PaneView {
            list,
            history: History::start(root.clone()),
            current: root.clone(),
            pending: None,
            opened: start.opened,
            rows: start.rows,
            thumb_jobs: HashMap::new(),
            indexed_path: String::new(),
            tabs: PaneTabs::start(root.clone()),
            tab_strip: left_tabs,
        },
        right: PaneView {
            list: right_list,
            history: History::start(root.clone()),
            current: root.clone(),
            pending: Some(PendingNav {
                path: root.clone(),
                kind: NavKind::Refresh,
            }),
            opened: None,
            rows: right_rows,
            thumb_jobs: HashMap::new(),
            indexed_path: String::new(),
            tabs: PaneTabs::start(root.clone()),
            tab_strip: right_tabs,
        },
        tree,
        back,
        forward,
        address,
        tree_root: root.clone(),
        suppress_select: false,
        icons: IconCache::new(),
        images,
        nodes: Vec::new(),
        polling,
        thumbs: ThumbCache::with_decoder(images::decode_image),
        file_op: None,
        menu_host: None,
        timeout,
        other,
        settings_file: start.settings_file,
        search,
        query: Query::new(),
        index: FilenameIndex::new(),
        search_hits: Vec::new(),
        search_applied: String::new(),
    });
    let shown = state.left.rows.shown().len();
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
    }
    insert_root(hwnd, &root);
    insert_network(hwnd);
    refill_tabs(hwnd, PaneId::Left);
    refill_tabs(hwnd, PaneId::Right);
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
    // The right pane lists the same path. A busy volume stays pending for the timer.
    try_pending(hwnd);
    let needs_timer = state_mut(hwnd).is_some_and(|state| {
        state.polling
            || state.left.pending.is_some()
            || state.right.pending.is_some()
            || !state.left.rows.is_settled()
            || !state.right.rows.is_settled()
    });
    if needs_timer && unsafe { SetTimer(hwnd, TIMER_ID, POLL_INTERVAL_MS, None) } == 0 {
        eprintln!("list_shell could not start polling: {}", last_error());
        if let Some(state) = state_mut(hwnd) {
            state.polling = false;
        }
    } else if needs_timer {
        if let Some(state) = state_mut(hwnd) {
            state.polling = true;
        }
    }
    Ok(())
}

fn create_list(parent: HWND, id: isize, what: &str) -> Result<HWND, u8> {
    let list = create_control(parent, WC_LISTVIEWW, "", list_style(), 0, id, what)?;
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
    Ok(list)
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
        | LVS_EDITLABELS
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
        WM_KEYDOWN if message.wParam == VK_F6 as WPARAM => {
            toggle_pane(root);
            true
        }
        WM_KEYDOWN if key_down(VK_CONTROL) && !key_down(VK_MENU) => match message.wParam as u16 {
            VK_T if !key_repeat(message.lParam) => {
                new_active_tab(root);
                true
            }
            VK_W if !key_repeat(message.lParam) => {
                close_active_tab(root);
                true
            }
            VK_TAB => {
                cycle_active_tab(root, key_down(VK_SHIFT));
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
        WM_CONTEXTMENU => {
            on_context_menu(hwnd, wparam, lparam);
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
    if id == ID_SEARCH && code == EN_CHANGE {
        on_search_change(hwnd);
        return;
    }
    if code != BN_CLICKED {
        return;
    }
    match id {
        ID_BACK => go_back(hwnd),
        ID_FORWARD => go_forward(hwnd),
        ID_OTHER => toggle_pane(hwnd),
        ID_TIMEOUT => edit_timeout(hwnd),
        _ => {}
    }
}

fn on_search_change(hwnd: HWND) {
    let Some(search) = state_mut(hwnd).map(|state| state.search) else {
        return;
    };
    let text = control_string(search);
    let changed = state_mut(hwnd).is_some_and(|state| state.query.set(&text));
    if changed {
        ensure_timer(hwnd);
    }
}

fn control_string(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; len as usize + 1];
    let copied = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if copied <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..copied as usize])
}

fn index_pane(index: &mut FilenameIndex, pane: &mut PaneView) -> bool {
    if !pane.rows.is_settled() || pane.indexed_path == pane.current {
        return false;
    }
    let volume = VolumeId::from_directory(&Path::new(pane.current.clone()));
    let indexed = volume.kind() == VolumeKind::Local;
    if indexed {
        for entry in pane.rows.shown() {
            if entry.name.is_empty() || entry.name == "." || entry.name == ".." {
                continue;
            }
            let path = child_directory(&pane.current, &entry.name);
            index.upsert(IndexRecord {
                path,
                name: entry.name.clone(),
                volume_key: volume.key().to_string(),
                is_directory: entry.is_directory,
                size: entry.size,
                modified: entry.last_write,
            });
        }
    }
    pane.indexed_path = pane.current.clone();
    indexed
}

fn apply_search(state: &mut ListWindow) -> bool {
    if state.query.text() == state.search_applied {
        return false;
    }
    state.search_applied = state.query.text().to_string();
    let active = state.panes.active();
    cancel_pane_thumbs(pane_mut(state, active));
    if state.query.is_active() {
        state.search_hits = state
            .index
            .search(state.query.needle())
            .into_iter()
            .cloned()
            .collect();
    } else {
        state.search_hits.clear();
    }
    true
}

fn visible_len(state: &ListWindow) -> usize {
    if state.query.is_active() {
        state.search_hits.len()
    } else {
        pane_ref(state, state.panes.active()).rows.shown().len()
    }
}

fn visible_entry(state: &ListWindow, list: HWND, index: usize) -> Option<fx_core::FileEntry> {
    let id = pane_id_from_list(state, list)?;
    if list_shows_search(state.query.is_active(), state.panes.active() == id) {
        state.search_hits.get(index).map(file_entry_from_record)
    } else {
        pane_ref(state, id).rows.shown().get(index).cloned()
    }
}

fn file_entry_from_record(record: &IndexRecord) -> fx_core::FileEntry {
    fx_core::FileEntry {
        name: record.name.clone(),
        attributes: if record.is_directory {
            FILE_ATTRIBUTE_DIRECTORY
        } else {
            FILE_ATTRIBUTE_NORMAL
        },
        size: record.size,
        created: record.modified,
        last_write: record.modified,
        is_directory: record.is_directory,
        is_reparse_point: false,
    }
}

fn edit_timeout(hwnd: HWND) {
    let Some((millis, path)) = state_mut(hwnd).map(|state| {
        let millis = u64::try_from(state.shell.network_timeout().as_millis()).unwrap_or(2_000);
        (millis, state.settings_file.clone())
    }) else {
        return;
    };
    let Some(saved) = timeout::show(hwnd, millis, &path) else {
        return;
    };
    if let Some(state) = state_mut(hwnd) {
        state
            .shell
            .set_network_timeout(TimeoutSetting { millis: saved });
    }
}

fn go_back(hwnd: HWND) {
    let Some(path) = state_mut(hwnd).and_then(|state| {
        pane_ref(state, state.panes.active())
            .history
            .previous()
            .map(str::to_string)
    }) else {
        return;
    };
    request_navigation(hwnd, path, NavKind::Back);
}

fn go_forward(hwnd: HWND) {
    let Some(path) = state_mut(hwnd).and_then(|state| {
        pane_ref(state, state.panes.active())
            .history
            .following()
            .map(str::to_string)
    }) else {
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
    let Some(id) = state_mut(hwnd).map(|state| state.panes.active()) else {
        return;
    };
    if kind == NavKind::Push
        && state_mut(hwnd).is_some_and(|state| pane_ref(state, id).history.current() == path)
    {
        if let Some(state) = state_mut(hwnd) {
            pane_mut(state, id).pending = None;
        }
        return;
    }
    if let Some(state) = state_mut(hwnd) {
        pane_mut(state, id).pending = Some(PendingNav { path, kind });
    }
    try_pending(hwnd);
}

/// Open each pane's pending path without blocking. A busy volume stays pending.
/// The rows already on screen stay until that pane's open returns `Ok`.
fn try_pending(hwnd: HWND) {
    let Some(first) = state_mut(hwnd).map(|state| state.panes.active()) else {
        return;
    };
    let second = match first {
        PaneId::Left => PaneId::Right,
        PaneId::Right => PaneId::Left,
    };
    try_pane_pending(hwnd, first);
    try_pane_pending(hwnd, second);
}

fn try_pane_pending(hwnd: HWND, id: PaneId) {
    let Some(pending) = state_mut(hwnd).and_then(|state| pane_ref(state, id).pending.clone())
    else {
        return;
    };
    let Some(shell) = state_mut(hwnd).map(|state| state.shell.clone()) else {
        return;
    };
    let directory = Path::new(pending.path.clone());
    let volume = VolumeId::from_directory(&directory);
    let target_key = volume.key().to_string();
    release_for_navigation(hwnd, id, &target_key);
    match shell.open(volume, directory) {
        Ok(opened) if opened.worker_busy() => {
            // The names are already cached. Keep `pending` so the timer retries
            // the revalidation. Storing this open would make `poll` settle the rows.
            show_cached_listing(hwnd, id, &pending.path, opened);
            ensure_timer(hwnd);
        }
        Ok(opened) => {
            if let Some(state) = state_mut(hwnd) {
                let pane = pane_mut(state, id);
                pane.pending = None;
                commit_history(pane, &pending);
            }
            refill_tabs(hwnd, id);
            present_directory(hwnd, id, &pending.path, opened);
            if state_mut(hwnd).is_some_and(|state| state.panes.active() == id) {
                follow_tree(hwnd, &pending.path);
                update_nav_buttons(hwnd);
            }
        }
        Err(EnumError::VolumeBusy) => ensure_timer(hwnd),
        Err(_) => {
            if let Some(state) = state_mut(hwnd) {
                pane_mut(state, id).pending = None;
            }
        }
    }
}

fn commit_history(pane: &mut PaneView, pending: &PendingNav) {
    match pending.kind {
        NavKind::Push => {
            let _ = pane.history.go(&pending.path);
        }
        NavKind::Back => {
            let _ = pane.history.back();
        }
        NavKind::Forward => {
            let _ = pane.history.forward();
        }
        NavKind::Refresh => {}
    }
    pane.tabs.set_active_path(&pending.path);
}

fn present_directory(hwnd: HWND, id: PaneId, path: &str, mut opened: OpenedDirectory) {
    let rows = DirectoryRows::from_cached(opened.cached.take());
    let shown = rows.shown().len();
    let needs_poll = !rows.is_settled();
    let Some((list, address, count, active)) = (|| {
        let state = state_mut(hwnd)?;
        let active = state.panes.active() == id;
        let address = state.address;
        let count = displayed_count(
            active,
            state.query.is_active(),
            state.search_hits.len(),
            shown,
        );
        {
            let pane = pane_mut(state, id);
            cancel_pane_thumbs(pane);
            pane.rows = rows;
            pane.opened = Some(opened);
            pane.current = path.to_string();
        }
        if state.panes.active() == id {
            state.panes.set_active_path(path);
        } else {
            state.panes.set_path(id, path);
        }
        let list = pane_ref(state, id).list;
        Some((list, address, count, active))
    })() else {
        return;
    };
    set_count(list, count);
    if active {
        set_control_text(address, path);
        set_caption(hwnd, path);
    }
    // Paint the cached count before the timer, or this function's caller, polls.
    unsafe { UpdateWindow(list) };
    if needs_poll {
        ensure_timer(hwnd);
    }
}

/// Paint a cache hit while its revalidation is still waiting for the volume worker.
///
/// `pending` stays set. The live `Err(VolumeBusy)` listing is not stored, so the
/// next timer tick can open the directory again instead of settling the rows.
fn show_cached_listing(hwnd: HWND, id: PaneId, path: &str, mut opened: OpenedDirectory) {
    let Some(cached) = opened.cached.take() else {
        return;
    };
    let already = state_mut(hwnd).is_some_and(|state| {
        let pane = pane_ref(state, id);
        pane.current == path && pane.rows.shown() == cached.as_slice()
    });
    if already {
        return;
    }
    let rows = DirectoryRows::from_cached(Some(cached));
    let shown = rows.shown().len();
    let Some((list, address, count, active)) = (|| {
        let state = state_mut(hwnd)?;
        let active = state.panes.active() == id;
        let address = state.address;
        let count = displayed_count(
            active,
            state.query.is_active(),
            state.search_hits.len(),
            shown,
        );
        {
            let pane = pane_mut(state, id);
            if let Some(previous) = pane.opened.take() {
                previous.cancel();
            }
            cancel_pane_thumbs(pane);
            pane.rows = rows;
            pane.current = path.to_string();
        }
        if active {
            state.panes.set_active_path(path);
        } else {
            state.panes.set_path(id, path);
        }
        let list = pane_ref(state, id).list;
        Some((list, address, count, active))
    })() else {
        return;
    };
    set_count(list, count);
    if active {
        set_control_text(address, path);
        set_caption(hwnd, path);
    }
    unsafe { UpdateWindow(list) };
}

/// Search rows belong to the active list. The other list keeps its directory count.
fn displayed_count(
    active: bool,
    query_active: bool,
    search_len: usize,
    directory_len: usize,
) -> usize {
    if list_shows_search(query_active, active) {
        search_len
    } else {
        directory_len
    }
}

/// Search hits fill a list only when that list was already the active pane.
///
/// A click on the other list is still a directory row. Activating the pane
/// afterwards must not reread that index as a search hit.
fn list_shows_search(query_active: bool, list_is_active: bool) -> bool {
    query_active && list_is_active
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
fn release_for_navigation(hwnd: HWND, id: PaneId, target_key: &str) {
    let mut rearm = Vec::new();
    if let Some(state) = state_mut(hwnd) {
        if let Some(opened) = pane_mut(state, id).opened.take() {
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
            pane_ref(state, state.panes.active()).history.can_back(),
            pane_ref(state, state.panes.active()).history.can_forward(),
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
    let painted = {
        let Some(state) = state_mut(hwnd) else {
            return;
        };
        let mut left_change = drain_pane(&mut state.left);
        let mut right_change = drain_pane(&mut state.right);
        let indexed_left = index_pane(&mut state.index, &mut state.left);
        let indexed_right = index_pane(&mut state.index, &mut state.right);
        if indexed_left || indexed_right {
            state.search_applied.clear();
        }
        let search_refreshed = apply_search(state);
        let active = state.panes.active();
        if state.query.is_active() {
            match active {
                PaneId::Left => left_change = RowChange::Unchanged,
                PaneId::Right => right_change = RowChange::Unchanged,
            }
        }
        let shown = visible_len(state);
        let (tree_changes, settled_nodes) = drain_tree(state);
        let left_thumbs = poll_pane_thumbs(&mut state.left);
        let right_thumbs = poll_pane_thumbs(&mut state.right);
        let idle = state.left.pending.is_none()
            && state.right.pending.is_none()
            && state.left.rows.is_settled()
            && state.right.rows.is_settled()
            && !pane_thumbs_pending(&state.left)
            && !pane_thumbs_pending(&state.right)
            && !file_op_pending(state)
            && !menu_host_pending(state)
            && state.query.text() == state.search_applied
            && state
                .nodes
                .iter()
                .all(|node| node.opened.is_none() && node.browse.is_none());
        (
            state.left.list,
            state.right.list,
            left_change,
            right_change,
            state.left.rows.shown().len(),
            state.right.rows.shown().len(),
            active,
            shown,
            tree_changes,
            settled_nodes,
            idle,
            left_thumbs,
            right_thumbs,
            search_refreshed,
        )
    };
    let (
        left_list,
        right_list,
        left_change,
        right_change,
        left_rows,
        right_rows,
        active,
        shown,
        tree_changes,
        settled_nodes,
        idle,
        left_thumbs,
        right_thumbs,
        search_refreshed,
    ) = painted;
    let (active_list, active_rows, inactive_list, inactive_rows, active_change, inactive_change) =
        match active {
            PaneId::Left => (
                left_list,
                left_rows,
                right_list,
                right_rows,
                left_change,
                right_change,
            ),
            PaneId::Right => (
                right_list,
                right_rows,
                left_list,
                left_rows,
                right_change,
                left_change,
            ),
        };
    paint(
        active_list,
        active_change,
        if search_refreshed { shown } else { active_rows },
    );
    paint(inactive_list, inactive_change, inactive_rows);
    if search_refreshed {
        set_count(active_list, shown);
    }
    if left_thumbs {
        redraw_visible(left_list);
    }
    if right_thumbs {
        redraw_visible(right_list);
    }
    poll_file_op(hwnd);
    poll_menu_host(hwnd);
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

fn drain_pane(pane: &mut PaneView) -> RowChange {
    let PaneView {
        opened,
        rows,
        pending,
        ..
    } = pane;
    let Some(opened) = opened.as_mut() else {
        // A busy cache hit is showing names and still has a pending open.
        // Settling those rows would skip the revalidation.
        if pending.is_some() {
            return RowChange::Unchanged;
        }
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
        NM_SETFOCUS => {
            on_list_focus(hwnd, lparam);
            0
        }
        NM_CUSTOMDRAW => on_custom_draw(hwnd, lparam),
        LVN_GETDISPINFOW => {
            on_list_dispinfo(hwnd, lparam);
            0
        }
        LVN_ENDLABELEDITW => on_end_label_edit(hwnd, lparam),
        LVN_ITEMACTIVATE => {
            on_item_activate(hwnd, lparam);
            0
        }
        TVN_ITEMEXPANDINGW => on_item_expanding(hwnd, lparam),
        TVN_SELCHANGEDW => on_selection(hwnd, lparam),
        TCN_SELCHANGE => {
            on_tab_change(hwnd, lparam);
            0
        }
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
    let list = info.hdr.hwndFrom;
    let row = state_mut(hwnd).and_then(|state| {
        visible_entry(state, list, index).map(|entry| {
            let label = if wants_text {
                columns::text(subitem, &entry)
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

fn on_list_focus(hwnd: HWND, lparam: LPARAM) {
    let list = unsafe { (*(lparam as *const NMHDR)).hwndFrom };
    let Some(id) = state_mut(hwnd).and_then(|state| pane_id_from_list(state, list)) else {
        return;
    };
    activate_pane(hwnd, id);
}

/// Size and Date modified for rows the list view is painting. Subitem 0 stays
/// with the control so the extension icon is still drawn. The tree's custom
/// draw is ignored. The label is copied before DirectWrite runs. The cell uses
/// the list view's font. A selected row uses the inactive colors when focus is
/// in the tree or the address bar.
fn on_custom_draw(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let draw = unsafe { &*(lparam as *const NMLVCUSTOMDRAW) };
    let list = draw.nmcd.hdr.hwndFrom;
    let known = state_mut(hwnd).is_some_and(|state| pane_id_from_list(state, list).is_some());
    if !known {
        return CDRF_DODEFAULT as LRESULT;
    }
    match draw.nmcd.dwDrawStage {
        CDDS_PREPAINT => CDRF_NOTIFYITEMDRAW as LRESULT,
        CDDS_ITEMPREPAINT => {
            request_visible_thumb(hwnd, list, draw.nmcd.dwItemSpec);
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
        visible_entry(state, list, row).map(|entry| columns::text(subitem, &entry))
    });
    let Some(text) = text.filter(|text| !text.is_empty()) else {
        return CDRF_DODEFAULT as LRESULT;
    };
    let colors = cell::selection(
        row_selected(list, row),
        list_focused(list),
        show_sel_always(list),
    );
    let bounds = cell_rect(draw.nmcd.rc);
    let text_bounds = cell_rect(draw.rcText);
    if draw::paint_text(
        draw.nmcd.hdc,
        list_font(list),
        bounds,
        text_bounds,
        &text,
        colors,
    ) {
        CDRF_SKIPDEFAULT as LRESULT
    } else {
        CDRF_DODEFAULT as LRESULT
    }
}

fn list_focused(list: HWND) -> bool {
    let focus = unsafe { GetFocus() };
    focus == list
}

fn show_sel_always(list: HWND) -> bool {
    let style = unsafe { GetWindowLongPtrW(list, GWL_STYLE) } as u32;
    style & LVS_SHOWSELALWAYS != 0
}

/// The font the Name column is using. Null leaves the cell to the list view.
fn list_font(list: HWND) -> *mut core::ffi::c_void {
    unsafe { SendMessageW(list, WM_GETFONT, 0, 0) as *mut core::ffi::c_void }
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

fn cancel_pane_thumbs(pane: &mut PaneView) {
    for job in pane.thumb_jobs.values() {
        job.ticket.cancel();
    }
    pane.thumb_jobs.clear();
}

fn pane_thumbs_pending(pane: &PaneView) -> bool {
    pane.thumb_jobs.values().any(|job| !job.settled)
}

/// `try_recv` never waits. A transition to Ready asks for one redraw.
fn poll_pane_thumbs(pane: &mut PaneView) -> bool {
    let mut ready = false;
    for job in pane.thumb_jobs.values_mut() {
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
fn request_visible_thumb(hwnd: HWND, list: HWND, row: usize) {
    let Some(path) = state_mut(hwnd).and_then(|state| thumb_path(state, list, row)) else {
        return;
    };
    let pending = state_mut(hwnd).is_some_and(|state| {
        let Some(id) = pane_id_from_list(state, list) else {
            return false;
        };
        let pane = pane_mut(state, id);
        if pane.thumb_jobs.contains_key(&path) {
            return false;
        }
        let ticket = state.thumbs.request(&path);
        let settled = !matches!(ticket.try_recv(), TryThumb::Pending);
        pane_mut(state, id)
            .thumb_jobs
            .insert(path, ThumbJob { ticket, settled });
        !settled
    });
    if pending {
        ensure_timer(hwnd);
    }
}

fn thumb_path(state: &ListWindow, list: HWND, row: usize) -> Option<String> {
    let id = pane_id_from_list(state, list)?;
    if state.query.is_active() && state.panes.active() == id {
        let hit = state.search_hits.get(row)?;
        if hit.is_directory || !images::wants_thumbnail(&hit.name) {
            return None;
        }
        return Some(hit.path.clone());
    }
    let pane = pane_ref(state, id);
    let entry = pane.rows.shown().get(row)?;
    if entry.is_directory || !images::wants_thumbnail(&entry.name) {
        return None;
    }
    Some(child_directory(&pane.current, &entry.name))
}

fn overlay_thumb(hwnd: HWND, list: HWND, draw: &NMLVCUSTOMDRAW) -> LRESULT {
    if draw.iSubItem != columns::NAME {
        return CDRF_DODEFAULT as LRESULT;
    }
    let row = draw.nmcd.dwItemSpec;
    let rgba = state_mut(hwnd).and_then(|state| {
        let path = thumb_path(state, list, row)?;
        let id = pane_id_from_list(state, list)?;
        match pane_ref(state, id).thumb_jobs.get(&path)?.ticket.try_recv() {
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
    // `left` is the `LVIR_*` portion and `top` is the subitem. Name is subitem 0.
    let mut rect = RECT {
        left: LVIR_ICON as i32,
        top: columns::NAME,
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
    let list = info.hdr.hwndFrom;
    // Resolve the row before activation. The inactive list is showing directory
    // rows, and activation must not turn that index into a search hit.
    let path = activated_directory(hwnd, list, info.iItem as usize);
    if let Some(id) = state_mut(hwnd).and_then(|state| pane_id_from_list(state, list)) {
        activate_pane(hwnd, id);
    }
    let Some(path) = path else {
        return;
    };
    request_navigation(hwnd, path, NavKind::Push);
}

fn activated_directory(hwnd: HWND, list: HWND, index: usize) -> Option<String> {
    let state = state_mut(hwnd)?;
    let id = pane_id_from_list(state, list)?;
    if list_shows_search(state.query.is_active(), state.panes.active() == id) {
        let hit = state.search_hits.get(index)?;
        return hit.is_directory.then(|| hit.path.clone());
    }
    let pane = pane_ref(state, id);
    let entry = pane.rows.shown().get(index)?;
    if !entry.is_directory || entry.name.is_empty() {
        return None;
    }
    Some(child_directory(&pane.current, &entry.name))
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
    if state_mut(hwnd)
        .is_some_and(|state| same_directory(&pane_ref(state, state.panes.active()).current, &path))
    {
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
    let Some((
        tree,
        back,
        forward,
        address,
        timeout,
        search,
        other,
        left,
        right,
        left_tabs,
        right_tabs,
    )) = state_mut(hwnd).map(|state| {
        (
            state.tree,
            state.back,
            state.forward,
            state.address,
            state.timeout,
            state.search,
            state.other,
            state.left.list,
            state.right.list,
            state.left.tab_strip,
            state.right.tab_strip,
        )
    })
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
        let other_x = forward_x + FORWARD_WIDTH + NAV_GAP;
        MoveWindow(other, other_x, 0, OTHER_WIDTH, NAV_HEIGHT, 1);
        let address_x = other_x + OTHER_WIDTH + NAV_GAP;
        let timeout_x = (width - TIMEOUT_WIDTH).max(address_x);
        let search_x = (timeout_x - NAV_GAP - SEARCH_WIDTH).max(address_x);
        MoveWindow(search, search_x, 0, SEARCH_WIDTH, NAV_HEIGHT, 1);
        MoveWindow(timeout, timeout_x, 0, TIMEOUT_WIDTH, NAV_HEIGHT, 1);
        let address_width = (search_x - NAV_GAP - address_x).max(0);
        MoveWindow(address, address_x, 0, address_width, NAV_HEIGHT, 1);
        let top = NAV_HEIGHT + NAV_GAP;
        let body = (height - top).max(0);
        let tree_width = TREE_WIDTH.min(width.max(0));
        let lists = (width - tree_width - NAV_GAP).max(0);
        let left_width = lists / 2;
        let right_width = lists - left_width;
        let tab_height = tab_strip::TAB_HEIGHT;
        let list_top = top + tab_height;
        let list_body = (body - tab_height).max(0);
        MoveWindow(tree, 0, top, tree_width, body, 1);
        MoveWindow(left_tabs, tree_width, top, left_width, tab_height, 1);
        MoveWindow(left, tree_width, list_top, left_width, list_body, 1);
        let right_x = tree_width + left_width + NAV_GAP;
        MoveWindow(right_tabs, right_x, top, right_width, tab_height, 1);
        MoveWindow(right, right_x, list_top, right_width, list_body, 1);
        size_columns(left, left_width);
        size_columns(right, right_width);
    }
}

fn size_columns(list: HWND, list_width: i32) {
    let fixed = columns::SIZE_WIDTH + columns::MODIFIED_WIDTH;
    let name_width = (list_width - fixed).max(80);
    unsafe {
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

fn on_context_menu(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    let list = wparam as HWND;
    let Some(id) = state_mut(hwnd).and_then(|state| pane_id_from_list(state, list)) else {
        return;
    };
    let point = context_point(lparam);
    select_at(list, point);
    // Read the rows this list is showing now. Activation can switch the active
    // list to search hits, and those indexes are a different set of paths.
    let paths = paths_for_list(hwnd, list);
    let selected = selected_count(list);
    let restore = selected_indexes(list);
    activate_pane(hwnd, id);
    let query_active = state_mut(hwnd).is_some_and(|state| state.query.is_active());
    if !query_active && selected_count(list) == 0 {
        for index in &restore {
            set_selected(list, *index, true);
        }
    }
    let commands = commands::menu_for(selected, file_ops::clipboard_has_files());
    let Some(command) = popup_command(hwnd, &commands, point) else {
        return;
    };
    run_command(hwnd, command, &paths);
}

fn context_point(lparam: LPARAM) -> POINT {
    if lparam == -1 {
        let mut point = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut point) };
        point
    } else {
        POINT {
            x: (lparam as u32 & 0xffff) as i16 as i32,
            y: ((lparam as u32) >> 16) as i16 as i32,
        }
    }
}

fn select_at(list: HWND, screen: POINT) {
    let mut client = screen;
    unsafe { ScreenToClient(list, &mut client) };
    let mut hit = LVHITTESTINFO {
        pt: client,
        flags: 0,
        iItem: -1,
        iSubItem: 0,
        iGroup: 0,
    };
    let index = unsafe {
        SendMessageW(
            list,
            LVM_HITTEST,
            0,
            &mut hit as *mut LVHITTESTINFO as LPARAM,
        )
    };
    if index < 0 || row_selected(list, index as usize) {
        return;
    }
    set_selected(list, usize::MAX, false);
    set_selected(list, index as usize, true);
}

fn set_selected(list: HWND, row: usize, selected: bool) {
    let mut item = LVITEMW {
        mask: LVIF_STATE,
        iItem: 0,
        iSubItem: 0,
        state: if selected { LVIS_SELECTED } else { 0 },
        stateMask: LVIS_SELECTED,
        pszText: ptr::null_mut(),
        cchTextMax: 0,
        iImage: 0,
        lParam: 0,
        iIndent: 0,
        iGroupId: 0,
        cColumns: 0,
        puColumns: ptr::null_mut(),
        piColFmt: ptr::null_mut(),
        iGroup: 0,
    };
    let index = if row == usize::MAX {
        -1isize as WPARAM
    } else {
        row as WPARAM
    };
    unsafe {
        SendMessageW(
            list,
            LVM_SETITEMSTATE,
            index,
            &mut item as *mut LVITEMW as LPARAM,
        );
    }
}

fn selected_count(list: HWND) -> usize {
    let count = unsafe { SendMessageW(list, LVM_GETSELECTEDCOUNT, 0, 0) };
    if count < 0 {
        0
    } else {
        count as usize
    }
}

fn selected_indexes(list: HWND) -> Vec<usize> {
    let mut indexes = Vec::new();
    let mut index = -1isize;
    loop {
        index = unsafe {
            SendMessageW(
                list,
                LVM_GETNEXTITEM,
                index as WPARAM,
                LVNI_SELECTED as LPARAM,
            )
        };
        if index < 0 {
            break;
        }
        indexes.push(index as usize);
    }
    indexes
}

fn paths_for_list(hwnd: HWND, list: HWND) -> Vec<String> {
    let indexes = selected_indexes(list);
    state_mut(hwnd)
        .map(|state| {
            indexes
                .into_iter()
                .filter_map(|index| path_on_list(state, list, index))
                .collect()
        })
        .unwrap_or_default()
}

fn path_on_list(state: &ListWindow, list: HWND, index: usize) -> Option<String> {
    let id = pane_id_from_list(state, list)?;
    if list_shows_search(state.query.is_active(), state.panes.active() == id) {
        return Some(state.search_hits.get(index)?.path.clone());
    }
    let pane = pane_ref(state, id);
    let name = pane.rows.shown().get(index)?.name.clone();
    Some(child_directory(&pane.current, &name))
}

fn popup_command(hwnd: HWND, commands: &[Command], point: POINT) -> Option<Command> {
    if commands.is_empty() {
        return None;
    }
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return None;
    }
    for (index, command) in commands.iter().enumerate() {
        if *command == Command::MoreOptions && index > 0 {
            unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null()) };
        }
        let label = wide(commands::label(*command));
        unsafe { AppendMenuW(menu, MF_STRING, command_id(*command), label.as_ptr()) };
    }
    let picked = unsafe {
        TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            hwnd,
            ptr::null(),
        )
    };
    unsafe { DestroyMenu(menu) };
    command_from_id(picked as usize)
}

fn command_id(command: Command) -> usize {
    match command {
        Command::Copy => 1,
        Command::Cut => 2,
        Command::Paste => 3,
        Command::Rename => 4,
        Command::Delete => 5,
        Command::Properties => 6,
        Command::MoreOptions => 7,
    }
}

fn command_from_id(id: usize) -> Option<Command> {
    Some(match id {
        1 => Command::Copy,
        2 => Command::Cut,
        3 => Command::Paste,
        4 => Command::Rename,
        5 => Command::Delete,
        6 => Command::Properties,
        7 => Command::MoreOptions,
        _ => return None,
    })
}

fn run_command(hwnd: HWND, command: Command, paths: &[String]) {
    match command {
        Command::Copy => {
            let _ = file_ops::copy_paths(paths, false);
        }
        Command::Cut => {
            let _ = file_ops::copy_paths(paths, true);
        }
        Command::Paste => paste_into(hwnd),
        Command::Rename => begin_rename(hwnd),
        Command::Delete => delete_selection(hwnd, paths),
        Command::Properties => {
            if paths.len() == 1 {
                file_ops::show_properties(hwnd, &paths[0]);
            }
        }
        Command::MoreOptions => begin_menu_host(hwnd, paths),
    }
}

fn paste_into(hwnd: HWND) {
    let Some(paste) = file_ops::paste_request() else {
        return;
    };
    let Some(dest) =
        state_mut(hwnd).map(|state| pane_ref(state, state.panes.active()).current.clone())
    else {
        return;
    };
    let op = if paste.cut {
        Op::MoveInto {
            sources: paste.sources,
            dest: dest.clone(),
        }
    } else {
        Op::CopyInto {
            sources: paste.sources,
            dest: dest.clone(),
        }
    };
    begin_file_op(hwnd, op, dest);
}

fn delete_selection(hwnd: HWND, paths: &[String]) {
    if paths.is_empty() {
        return;
    }
    let prompt = if paths.len() == 1 {
        "Delete this item?".to_string()
    } else {
        format!("Delete {} items?", paths.len())
    };
    let text = wide(&prompt);
    let title = wide("Rapid Explorer");
    let answer = unsafe {
        MessageBoxW(
            hwnd,
            text.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONWARNING,
        )
    };
    if answer != IDYES {
        return;
    }
    let Some(directory) =
        state_mut(hwnd).map(|state| pane_ref(state, state.panes.active()).current.clone())
    else {
        return;
    };
    begin_file_op(
        hwnd,
        Op::Delete {
            paths: paths.to_vec(),
        },
        directory,
    );
}

fn begin_rename(hwnd: HWND) {
    if state_mut(hwnd).is_some_and(|state| state.query.is_active()) {
        return;
    }
    let Some(list) = list_handle(hwnd) else {
        return;
    };
    let indexes = selected_indexes(list);
    if indexes.len() != 1 {
        return;
    }
    unsafe { SendMessageW(list, LVM_EDITLABELW, indexes[0] as WPARAM, 0) };
}

fn on_end_label_edit(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let info = unsafe { &*(lparam as *const NMLVDISPINFOW) };
    let item = &info.item;
    if item.pszText.is_null() || item.iItem < 0 {
        return 0;
    }
    let new_name = control_text_from(item.pszText);
    if state_mut(hwnd).is_some_and(|state| state.query.is_active()) {
        return 0;
    }
    let list = info.hdr.hwndFrom;
    let rename = state_mut(hwnd).and_then(|state| {
        let id = pane_id_from_list(state, list)?;
        let pane = pane_ref(state, id);
        let entry = pane.rows.shown().get(item.iItem as usize)?;
        let accepted = commands::accepted_rename(&entry.name, &new_name)?;
        let from = child_directory(&pane.current, &entry.name);
        let to = child_directory(&pane.current, &accepted);
        let directory = pane.current.clone();
        Some((from, to, directory))
    });
    if let Some((from, to, directory)) = rename {
        begin_file_op(hwnd, Op::Rename { from, to }, directory);
    }
    0
}

fn control_text_from(text: *const u16) -> String {
    if text.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    unsafe {
        while *text.add(len) != 0 {
            len += 1;
            if len > 32_768 {
                break;
            }
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
    }
}

fn begin_file_op(hwnd: HWND, op: Op, directory: String) {
    if file_op_pending_hwnd(hwnd) {
        return;
    }
    let Some(file_op) = FileOp::start(op, directory) else {
        return;
    };
    if let Some(state) = state_mut(hwnd) {
        state.file_op = Some(file_op);
    }
    ensure_timer(hwnd);
}

fn file_op_pending(state: &ListWindow) -> bool {
    state
        .file_op
        .as_ref()
        .is_some_and(|op| op.try_recv().is_none())
}

fn file_op_pending_hwnd(hwnd: HWND) -> bool {
    state_mut(hwnd).is_some_and(|state| file_op_pending(state))
}

fn begin_menu_host(hwnd: HWND, paths: &[String]) {
    if menu_host_pending_hwnd(hwnd) {
        return;
    }
    let Some(directory) =
        state_mut(hwnd).map(|state| pane_ref(state, state.panes.active()).current.clone())
    else {
        return;
    };
    let Some(session) = HelperSession::beside_exe(&directory, paths) else {
        return;
    };
    if let Some(state) = state_mut(hwnd) {
        state.menu_host = Some(session);
    }
    ensure_timer(hwnd);
}

fn menu_host_pending(state: &ListWindow) -> bool {
    state
        .menu_host
        .as_ref()
        .is_some_and(|host| host.try_recv().is_none())
}

fn menu_host_pending_hwnd(hwnd: HWND) -> bool {
    state_mut(hwnd).is_some_and(|state| menu_host_pending(state))
}

fn poll_menu_host(hwnd: HWND) {
    let outcome = state_mut(hwnd).and_then(|state| {
        let host = state.menu_host.as_ref()?;
        host.supervise(Instant::now());
        let verbs = host.try_recv()?;
        Some((verbs, host.directory().to_string()))
    });
    let Some((verbs, directory)) = outcome else {
        return;
    };
    if let Some(state) = state_mut(hwnd) {
        state.menu_host = None;
    }
    let same = state_mut(hwnd)
        .is_some_and(|state| pane_ref(state, state.panes.active()).current == directory);
    if same && verbs.iter().any(|label| !label.is_empty()) {
        show_helper_verbs(hwnd, &verbs);
    }
}

/// A second menu for labels the helper returned. The choice is not invoked in
/// this process. This helper returns no labels, so the native menu stands.
fn show_helper_verbs(hwnd: HWND, verbs: &[String]) {
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return;
    }
    for (index, label) in verbs.iter().enumerate() {
        if label.is_empty() {
            continue;
        }
        let text = wide(label);
        unsafe { AppendMenuW(menu, MF_STRING, 100 + index, text.as_ptr()) };
    }
    let mut point = POINT { x: 0, y: 0 };
    unsafe {
        GetCursorPos(&mut point);
        TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            hwnd,
            ptr::null(),
        );
        DestroyMenu(menu);
    }
}

fn poll_file_op(hwnd: HWND) {
    let outcome = state_mut(hwnd).and_then(|state| {
        let op = state.file_op.as_ref()?;
        let ok = op.try_recv()?;
        Some((ok, op.directory().to_string()))
    });
    let Some((ok, directory)) = outcome else {
        return;
    };
    if let Some(state) = state_mut(hwnd) {
        state.file_op = None;
    }
    if ok {
        refresh_directory(hwnd, directory);
    }
}

fn refresh_directory(hwnd: HWND, directory: String) {
    // Both panes can show this directory. Refreshing only the left list leaves
    // the right list stale, and a retry then reopens the left listing.
    let targets = state_mut(hwnd)
        .map(|state| refresh_targets(&state.left.current, &state.right.current, &directory));
    let Some(targets) = targets else {
        return;
    };
    if targets.is_empty() {
        return;
    }
    if let Some(state) = state_mut(hwnd) {
        for id in &targets {
            pane_mut(state, *id).pending = Some(PendingNav {
                path: directory.clone(),
                kind: NavKind::Refresh,
            });
        }
    }
    try_pending(hwnd);
}

/// Every pane whose `current` path is `directory`.
fn refresh_targets(left: &str, right: &str, directory: &str) -> Vec<PaneId> {
    let mut ids = Vec::new();
    if left == directory {
        ids.push(PaneId::Left);
    }
    if right == directory {
        ids.push(PaneId::Right);
    }
    ids
}

fn on_destroy(hwnd: HWND) {
    let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut ListWindow;
    if !raw.is_null() {
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
        let mut state = unsafe { Box::from_raw(raw) };
        unsafe {
            SendMessageW(state.right.list, LVM_SETIMAGELIST, LVSIL_SMALL as WPARAM, 0);
        }
        cancel_pane_thumbs(&mut state.left);
        cancel_pane_thumbs(&mut state.right);
        if state.polling {
            unsafe { KillTimer(hwnd, TIMER_ID) };
        }
        if let Some(opened) = &state.left.opened {
            opened.cancel();
        }
        if let Some(opened) = &state.right.opened {
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
    state_mut(hwnd).map(|state| pane_ref(state, state.panes.active()).list)
}

fn pane_mut(state: &mut ListWindow, id: PaneId) -> &mut PaneView {
    match id {
        PaneId::Left => &mut state.left,
        PaneId::Right => &mut state.right,
    }
}

fn pane_ref(state: &ListWindow, id: PaneId) -> &PaneView {
    match id {
        PaneId::Left => &state.left,
        PaneId::Right => &state.right,
    }
}

fn pane_id_from_list(state: &ListWindow, list: HWND) -> Option<PaneId> {
    if state.left.list == list {
        Some(PaneId::Left)
    } else if state.right.list == list {
        Some(PaneId::Right)
    } else {
        None
    }
}

fn key_down(vk: u16) -> bool {
    unsafe { GetKeyState(i32::from(vk)) < 0 }
}

fn key_repeat(lparam: LPARAM) -> bool {
    lparam & (1 << 30) != 0
}

fn refill_tabs(hwnd: HWND, id: PaneId) {
    let Some((strip, titles, active)) = state_mut(hwnd).map(|state| {
        let pane = pane_ref(state, id);
        let titles = pane
            .tabs
            .paths()
            .iter()
            .map(|path| PaneTabs::title(path))
            .collect::<Vec<_>>();
        (pane.tab_strip, titles, pane.tabs.active_index())
    }) else {
        return;
    };
    tab_strip::refill(strip, &titles, active);
}

fn new_active_tab(hwnd: HWND) {
    let Some(id) = state_mut(hwnd).map(|state| state.panes.active()) else {
        return;
    };
    if let Some(state) = state_mut(hwnd) {
        let _ = pane_mut(state, id).tabs.new_tab();
    }
    refill_tabs(hwnd, id);
}

fn close_active_tab(hwnd: HWND) {
    let Some(id) = state_mut(hwnd).map(|state| state.panes.active()) else {
        return;
    };
    let Some(path) = state_mut(hwnd)
        .and_then(|state| pane_mut(state, id).tabs.close_active().map(str::to_string))
    else {
        return;
    };
    refill_tabs(hwnd, id);
    let current = state_mut(hwnd).map(|state| pane_ref(state, id).current.clone());
    if current.as_deref() != Some(path.as_str()) {
        request_navigation(hwnd, path, NavKind::Push);
    }
}

fn cycle_active_tab(hwnd: HWND, backward: bool) {
    let Some(id) = state_mut(hwnd).map(|state| state.panes.active()) else {
        return;
    };
    let Some(index) = state_mut(hwnd).map(|state| {
        let tabs = &pane_ref(state, id).tabs;
        let len = tabs.len();
        if len == 0 {
            return 0;
        }
        let active = tabs.active_index();
        if backward {
            if active == 0 {
                len - 1
            } else {
                active - 1
            }
        } else {
            (active + 1) % len
        }
    }) else {
        return;
    };
    choose_tab(hwnd, id, index);
}

fn choose_tab(hwnd: HWND, id: PaneId, index: usize) {
    let Some(path) = state_mut(hwnd)
        .and_then(|state| pane_mut(state, id).tabs.select(index).map(str::to_string))
    else {
        return;
    };
    refill_tabs(hwnd, id);
    activate_pane(hwnd, id);
    request_navigation(hwnd, path, NavKind::Push);
}

fn on_tab_change(hwnd: HWND, lparam: LPARAM) {
    let header = unsafe { &*(lparam as *const NMHDR) };
    let Some(id) = tab_strip::pane_for(header.idFrom as isize) else {
        return;
    };
    let index = unsafe { SendMessageW(header.hwndFrom, TCM_GETCURSEL, 0, 0) };
    if index < 0 {
        return;
    }
    let index = index as usize;
    let already =
        state_mut(hwnd).is_some_and(|state| pane_ref(state, id).tabs.active_index() == index);
    if already {
        return;
    }
    choose_tab(hwnd, id, index);
}

/// Focus `id` without changing either path or opening a listing.
fn activate_pane(hwnd: HWND, id: PaneId) {
    let switched = state_mut(hwnd).is_some_and(|state| {
        if state.panes.active() == id {
            return false;
        }
        state.panes.activate(id);
        true
    });
    if switched {
        show_active_pane(hwnd);
    }
}

fn toggle_pane(hwnd: HWND) {
    let Some(destination) = state_mut(hwnd).map(|state| {
        let destination = state.panes.other_path().to_string();
        state.panes.toggle();
        destination
    }) else {
        return;
    };
    show_active_pane(hwnd);
    if let Some(state) = state_mut(hwnd) {
        debug_assert_eq!(state.panes.active_path(), destination);
    }
    if let Some(list) = list_handle(hwnd) {
        unsafe { SetFocus(list) };
    }
}

/// Point the address bar at the active pane and show search rows only there.
fn show_active_pane(hwnd: HWND) {
    let inactive = state_mut(hwnd).map(|state| match state.panes.active() {
        PaneId::Left => PaneId::Right,
        PaneId::Right => PaneId::Left,
    });
    let Some(inactive) = inactive else {
        return;
    };
    let Some((path, address, active_list, inactive_list, active_count, inactive_count)) =
        state_mut(hwnd).map(|state| {
            let path = state.panes.active_path().to_string();
            let address = state.address;
            let active = state.panes.active();
            let active_list = pane_ref(state, active).list;
            let inactive_list = pane_ref(state, inactive).list;
            let inactive_count = pane_ref(state, inactive).rows.shown().len();
            let active_count = visible_len(state);
            (
                path,
                address,
                active_list,
                inactive_list,
                active_count,
                inactive_count,
            )
        })
    else {
        return;
    };
    set_control_text(address, &path);
    set_caption(hwnd, &path);
    set_count(active_list, active_count);
    set_count(inactive_list, inactive_count);
    update_nav_buttons(hwnd);
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

#[cfg(test)]
mod tests {
    use super::{displayed_count, list_shows_search, refresh_targets, PaneId};

    #[test]
    fn refresh_includes_every_pane_on_that_directory() {
        assert_eq!(
            refresh_targets(r"C:\docs", r"C:\docs", r"C:\docs"),
            vec![PaneId::Left, PaneId::Right]
        );
        assert_eq!(
            refresh_targets(r"C:\left", r"C:\docs", r"C:\docs"),
            vec![PaneId::Right]
        );
        assert!(refresh_targets(r"C:\left", r"C:\right", r"C:\docs").is_empty());
    }

    #[test]
    fn inactive_list_keeps_directory_rows_during_a_query() {
        assert!(!list_shows_search(true, false));
        assert!(list_shows_search(true, true));
        assert!(!list_shows_search(false, true));
        assert!(!list_shows_search(false, false));
    }

    #[test]
    fn active_search_count_survives_a_directory_present() {
        assert_eq!(displayed_count(true, true, 2, 40), 2);
        assert_eq!(displayed_count(false, true, 2, 40), 40);
        assert_eq!(displayed_count(true, false, 2, 40), 40);
    }
}
