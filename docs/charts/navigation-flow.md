# Navigation flow — folder open (with cache short-circuit)

```mermaid
flowchart TD
    Start([User navigates to folder]) --> Q{Fresh listing already cached?}
    Q -->|yes| Render[Render immediately from cache]
    Render --> Refresh[Kick off async background re-enumeration]
    Refresh --> Diff{Changes found?}
    Diff -->|yes| Patch[Patch visible rows in place]
    Diff -->|no| Done1([Done])
    Patch --> Done1

    Q -->|no| Enumerate[Dispatch async enumeration on worker thread\nFindFirstFileEx, large fetch]
    Enumerate --> Populate[Populate virtual list data source\nas batches arrive]
    Populate --> Viewport[UI requests only visible-range rows\nImGuiListClipper-equivalent: SysListView32 virtual mode]
    Viewport --> OwnerDraw[Owner-draw pass: icons, thumbnails, custom columns]
    OwnerDraw --> Done2([Done])
```

Note: the cache short-circuit is the primary lever against Explorer's "click and wait" feel — a previously visited folder must repaint before any disk I/O completes, with reconciliation happening invisibly afterward.

The list half of this flow is `list_shell`. Tree selection, the address bar, Back, and Forward are the ways the user navigates. The window paints `cached` before polling, appends batches on a miss, and replaces rows only when a revalidation finishes with a different listing. A busy volume keeps the rows already shown and retries without blocking. The tree follows the same cache-hit and batch rules for the child folders of a node the user expands. A navigation outside that root replaces the root and does not enumerate it. A visible row can also ask for a small icon. That icon is cached by extension and is drawn by the list view in the Name column. Size and Date modified for those same visible rows are DirectWrite cells. A visible image row also requests a thumbnail on the background pool. The icon stays until that buffer is ready, and leaving the directory cancels the request. The Network node is outside this folder flow: it stays collapsed until expanded, and a connectable share then enters this flow as a chosen path. The list also has a native context menu for Copy, Cut, Paste, Rename, Delete, and Properties. More options asks `menu_host`, a watchdog-supervised process that returns no verbs and does not load a shell extension. Paste, delete, and rename run off the UI thread. A panic writes a local dump file and does not open a socket. The network timeout is a stored setting, default 2 seconds, used only for network volumes, and a timeout is not retried. An in-memory filename index applies a scripted journal and does not read the disk. Search ranks those names by match quality. An empty query matches nothing. Frecency is not applied.
