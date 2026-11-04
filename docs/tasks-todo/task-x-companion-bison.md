# Task: The companion is a bison on the desktop

**From:** owner feedback 2026-10-20: "the companion icon should be the bison, stay on top, move to any corner, click opens the companion", "let the user have just the companion by default, minimised, showing the bison waiting, working…", "how does one get back to the main screen after closing it?".
**Size:** M. The way-back item ships first and alone.

## Today

- Before 2026-10-21, the `companion` window (`tauri.conf.json`): 380×220, transparent, frameless, skip-taskbar, **hidden** at start, not on top. Opened only with Ctrl/Cmd+Shift+Space (`commands/companion.rs`, `COMPANION_SHORTCUT`).
- Before 2026-10-21, `CompanionWindow.tsx` had collapsed (88 px Sparkles orb, `CompanionCollapsed.tsx`), expanded and overlay states.
- Bison poses exist: `src/assets/brand/mascot/nandi-bull-{rest,wait,carry,done}.png` (white on transparent), mapped in `src/themes/palette-icons.ts`.

## 1. Way back to the main window (first)

- [x] (2026-10-20, `d829976`) The companion header's **Open in Nandi** shows and focuses the main window (`openInMain`).
- [x] Closing `main` hides it (it doesn't quit); the companion shows (the bison if it's on), and Quit Nandi is in the companion's ⋯ menu.

## 2. The bubble

**Decided (2026-10-21): one window, not two.** The companion window's collapsed state is the bison. A separate `bubble` window would need cross-window state and has a click/blur race (clicking the bubble blurs the card). In one window, the card losing focus simply shrinks back to the bison in its corner.

- [x] Collapsed, the window is the bison on a `bg-primary` disc with the badge; ring colour per state. Pose from `poseFor` (`companion-model.ts`, tested): needs you / problem → `wait`, running → `carry`, quiet → `rest`, `done` for 4 s after the Inbox shrinks.
- [x] With **Nandi on the desktop** (`companion.bubble`, default on) it's shown at start (without focus) and always on top; closing the card (Escape, blur, ×, the hotkey) goes back to the bison. Off: the window hides as before.
- [x] Corner placement in Rust (`place_companion`): the bison and the card keep to the saved corner (`companion.corner`: corner + monitor name; bottom-right of the primary monitor at first), so the card opens toward the middle. Pure `nearest_corner` / `corner_position`, tested. Re-placed on scale change; falls back to the primary monitor if the saved one is gone.
- [x] Drag anywhere; when the moves stop (350 ms) it snaps to the nearest corner of the monitor it was dropped on. Our own moves (500 ms after placing) and the overlay are never snapped. The window-state plugin denylists the companion.
- [x] Click vs drag: a press that moves more than 5 px starts `startDragging()`; otherwise it's a click (tested). The card opens on `companion://expand` (the hotkey, Watch in companion) or a click, never on focus alone, so pressing the bison to drag it doesn't open it.
- [x] Right-click menu on the bison (native, so the small window doesn't cut it off): Open the companion, Open in Nandi, Hide the bison, Quit.

## 3. Companion-only by default

- [x] Setting "Start with only the bison" (`companion.start_bison_only`, needs the bison on): the main window is hidden once the core is up, so it may show for a moment at launch.
- [x] Settings › Appearance › Companion: "Nandi on the desktop" switch.
- [x] The start mode switch; Bison moves (when the system allows, always, never). i18n in en, fr, ar, ja.

## Tests

- [x] Rust: `nearest_corner`, `corner_position`, the bison and corner settings.
- [x] Vitest: pose mapping, click vs drag.

## Acceptance (in the app)

- [ ] Close the main window: the bison stays; clicking it opens the companion; "Open Nandi" brings the main window back.
- [ ] Drag the bison to each corner on two monitors; it snaps and is there after a restart.
- [ ] Start a run: the bison carries; an approval waits: it waits.
