# Task: UI fixes and polish for v2

**From:** owner notes during demo preparation (2026-10-24). Not for the demo: kept here so they aren't lost.
**Size:** S items, independent.

## Items

- [ ] **Right-click on the left menu does nothing.**
  - The sidebar (`src/components/layout/Sidebar.tsx`) handles click and the keyboard, but there's no `onContextMenu`, so a right-click on an agent, workflow or page shows nothing.
  - Expected: a native context menu per item, using `src/lib/context-menu.ts` (Tauri Menu API, already in the template). For example:
    - Open, Open in new window
    - Rename, Fork, Duplicate
    - Copy link, Show in agents repo
    - Delete (asks first)
  - Each menu shows only what applies to that item.
  - Test: right-click an agent, a workflow and a page. Each shows its own menu, the actions work, and Escape closes the menu.
