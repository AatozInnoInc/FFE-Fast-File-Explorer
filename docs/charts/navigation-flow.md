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
