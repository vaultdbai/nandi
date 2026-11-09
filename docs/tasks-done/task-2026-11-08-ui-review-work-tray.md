# UI review: reachability and clicks (work tray)

Shipped on `main` (859c7b7, 2026-11-08), follow-ups done the same day.

## What the review found

| #   | Problem                                                                                                                                                                       | Where                                                       |
| --- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------- |
| 1   | Running and finished work show only on Home. On any other page you can't see that a run finished or failed.                                                                   | `HomeView.tsx` (Running, Recent)                            |
| 2   | The Inbox has no page and no sidebar entry. It is a section inside Home, shown only when it has items. The "Waiting" tile scrolls Home instead of opening it.                 | `HomeView.tsx`, `nav-tree.ts` ("Home stays one item")       |
| 3   | All activity is reachable only from the "All activity" button under Recent, after up to ten blocks on Home.                                                                   | `HomeView.tsx`                                              |
| 4   | Home stacks Ask, model notice, welcome, backup nudge, email to-do, Chief of Staff digest, network line, tiles, Inbox, Running, Recent, Recent changes. Running sits far down. | `HomeView.tsx`                                              |
| 5   | A tool approval takes two moves: notice the badge, go to Home or the agent, then decide.                                                                                      | `Inbox.tsx`, `ApprovalRequestCard`                          |
| 6   | Settings has three entries in view at once: the title-bar gear, the sidebar item and Ctrl+,. The gear duplicates the sidebar.                                                 | `TitleBarContent.tsx`, `nav-tree.ts`                        |
| 7   | Panel visibility resets on every launch.                                                                                                                                      | `ui-store.ts`                                               |
| 8   | Comments and the test catalog said "seven pages"; there are eight since Notes.                                                                                                | `navigation-store.ts`, `Sidebar.tsx`, `docs/testing/` SH-08 |

## What shipped

- **Work tray** on the right of every page (`src/components/tray/`). Three groups in small type with coloured dots: Needs you (amber), Working (blue, pulsing), Done (green; red when failed; grey when cancelled; quieter text). Each group has a count and **All**. A tool approval has ✓ and ✗ in the row. Rows open their page. Data is Home's own feed (`useHomeFeed`), so the tray and Home can't disagree.
- **Toggle:** title-bar button on the right, the tray's own close button, Ctrl+2, View › Toggle Work Tray, and palette commands. This reverses SH-07's "no right panel" decision.
- **Inbox page** (`{ kind: 'inbox' }`, `InboxView.tsx`). Needs you › All opens it.
- **Sidebar:** Home now has two children, Inbox (with its count) and All activity.
- Strings in en, fr, ar, ja. Test catalog: SH-07, SH-08, SH-25 updated; SH-36 added.

## Why a right tray, not bottom trays

- Bottom-left and bottom-right trays cover content and the agent chat's message box, and fight the companion window for the same job.
- A right pane is resizable and hideable with the same pattern as the left one, and needs no new layout rules.
- One place for the three states reads in a single glance; two trays split the "Needs you" and "Done" story.

## Follow-ups (done 2026-11-08)

1. Home is slim: Ask, the digest, the email to-do, the Inbox and Recent changes. Running, Recent and the tiles are gone (the tray and the companion have them).
2. Both panes' visibility is saved in this machine's preferences (`sidebarVisible`, `workTrayVisible`).
3. The tiles are gone; Home's sidebar count is its own button and opens the Inbox page.
4. The title-bar Settings gear is gone (the sidebar and Ctrl+, have Settings).
5. `WorkTray.test.tsx`: empty groups, ✓ and ✗ decide, rows and All open, the close button.
6. Until the user shows or hides the tray, it is hidden below 1000px wide.
