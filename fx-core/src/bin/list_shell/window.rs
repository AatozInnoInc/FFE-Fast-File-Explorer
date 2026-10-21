//! One top-level window, a folder `SysTreeView32`, and a virtual `SysListView32`.
//!
//! The message loop is `GetMessageW`. Enumeration is `Shell::poll` on a timer.
//! That call does not wait, and this thread does not read a directory. The tree
//! lists a node only after that node expands, and only its direct child folders.

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
    InitCommonControlsEx, HTREEITEM, ICC_LISTVIEW_CLASSES, ICC_TREEVIEW_CLASSES,
    INITCOMMONCONTROLSEX, LVCFMT_LEFT, LVCF_FMT, LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_TEXT,
    LVM_INSERTCOLUMNW, LVM_REDRAWITEMS, LVM_SETCOLUMNWIDTH, LVM_SETEXTENDEDLISTVIEWSTYLE,
    LVM_SETITEMCOUNT, LVN_GETDISPINFOW, LVSICF_NOSCROLL, LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT,
    LVS_NOSORTHEADER, LVS_OWNERDATA, LVS_REPORT, LVS_SHOWSELALWAYS, NMHDR, NMLVDISPINFOW,
    NMTREEVIEWW, TVE_COLLAPSE, TVE_EXPAND, TVIF_CHILDREN, TVIF_HANDLE, TVIF_PARAM, TVIF_TEXT,
    TVINSERTSTRUCTW, TVINSERTSTRUCTW_0, TVITEMW, TVI_LAST, TVI_ROOT, TVM_DELETEITEM,
    TVM_INSERTITEMW, TVM_SETITEMW, TVN_ITEMEXPANDINGW, TVS_DISABLEDRAGDROP, TVS_HASBUTTONS,
    TVS_HASLINES, TVS_LINESATROOT, TVS_SHOWSELALWAYS, WC_LISTVIEWW, WC_TREEVIEWW,
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
use crate::tree::{child_directory, ChildChange, Expansion};

/// How often the UI thread drains `Shell::poll`. Not the ≤16 ms paint budget.
const POLL_INTERVAL_MS: u32 = 16;
const TIMER_ID: usize = 1;
/// Width of the folder tree beside the list. Not a performance budget.
const TREE_WIDTH: i32 = 240;
const CLASS_NAME: windows_sys::core::PCWSTR = w!("RapidExplorerList");

struct ListWindow {
    shell: Shell,
    list: HWND,
    tree: HWND,
    opened: Option<OpenedDirectory>,
    rows: DirectoryRows,
    nodes: Vec<FolderNode>,
    /// The timer is running. This is not "the list is still reading".
    polling: bool,
}

struct FolderNode {
    path: String,
    item: HTREEITEM,
    child_indices: Vec<usize>,
    expansion: Option<Expansion>,
    opened: Option<OpenedDirectory>,
    removed: bool,
}

impl FolderNode {
    fn new(path: String) -> Self {
        Self {
            path,
            item: 0,
            child_indices: Vec::new(),
            expansion: None,
            opened: None,
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

    let tree = unsafe {
        CreateWindowExW(
            0,
            WC_TREEVIEWW,
            ptr::null(),
            tree_style(),
            0,
            0,
            0,
            0,
            hwnd,
            2 as HMENU,
            instance(),
            ptr::null(),
        )
    };
    if tree.is_null() {
        eprintln!(
            "list_shell could not create the tree view: {}",
            last_error()
        );
        return Err(1);
    }

    let root = start.root.clone();
    let polling = !start.rows.is_settled();
    let state = Box::new(ListWindow {
        shell: start.shell,
        list,
        tree,
        opened: start.opened,
        rows: start.rows,
        nodes: Vec::new(),
        polling,
    });
    let shown = state.rows.shown().len();
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
    }
    insert_root(hwnd, &root);

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
                fit_panes(hwnd);
            }
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

fn on_timer(hwnd: HWND) {
    let Some(list) = list_handle(hwnd) else {
        return;
    };
    let (change, shown, tree_changes, settled_nodes, idle) = {
        let Some(state) = state_mut(hwnd) else {
            return;
        };
        let change = drain(state);
        let shown = state.rows.shown().len();
        let (tree_changes, settled_nodes) = drain_tree(state);
        let idle = state.rows.is_settled() && state.nodes.iter().all(|node| node.opened.is_none());
        (change, shown, tree_changes, settled_nodes, idle)
    };
    paint(list, change, shown);
    for (index, child_change) in tree_changes {
        apply_children(hwnd, index, child_change);
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

fn drain_tree(state: &mut ListWindow) -> (Vec<(usize, ChildChange)>, Vec<usize>) {
    let mut changes = Vec::new();
    let mut settled = Vec::new();
    for index in 0..state.nodes.len() {
        if state.nodes[index].removed {
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
            changes.push((index, change));
        }
    }
    (changes, settled)
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
        LVN_GETDISPINFOW => {
            on_list_dispinfo(hwnd, lparam);
            0
        }
        TVN_ITEMEXPANDINGW => on_item_expanding(hwnd, lparam),
        _ => 0,
    }
}

fn on_list_dispinfo(hwnd: HWND, lparam: LPARAM) {
    let info = unsafe { &mut *(lparam as *mut NMLVDISPINFOW) };
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
    let already = state_mut(hwnd).is_some_and(|state| state.nodes[index].expansion.is_some());
    if already {
        return 0;
    }
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
    if let Some(opened) = node.opened.take() {
        opened.cancel();
    }
    Some((tree, node.item))
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
    let settled = node.expansion.as_ref().is_some_and(Expansion::is_settled);
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
    if node.removed {
        None
    } else {
        Some(node.path.clone())
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

fn tree_handle(hwnd: HWND) -> Option<HWND> {
    state_mut(hwnd).map(|state| state.tree)
}

fn fit_panes(hwnd: HWND) {
    let Some(list) = list_handle(hwnd) else {
        return;
    };
    let tree = tree_handle(hwnd);
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
        let tree_width = TREE_WIDTH.min(width.max(0));
        let list_width = (width - tree_width).max(0);
        if let Some(tree) = tree {
            MoveWindow(tree, 0, 0, tree_width, height, 1);
        }
        MoveWindow(list, tree_width, 0, list_width, height, 1);
        SendMessageW(list, LVM_SETCOLUMNWIDTH, 0, list_width as LPARAM);
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
        for node in &state.nodes {
            if let Some(opened) = &node.opened {
                opened.cancel();
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
