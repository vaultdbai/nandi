//! Companion commands. The window itself (label `companion`, declared in
//! tauri.conf.json, hidden at start) is Tauri-only, so showing, hiding, and
//! pinning it happen here; the snapshot and chat are in
//! `services::companion`.
//!
//! - Hotkey: [`COMPANION_SHORTCUT`] (Cmd/Ctrl+Shift+Space) toggles the
//!   window from anywhere; the card opens with the ask box focused. It's
//!   the app's only global hotkey (the template's quick pane is retired).
//! - The bison (`companion.bubble`, on by default): collapsed, the window
//!   is the bison, shown at start and always on top. Clicking it opens the
//!   card in the same window; the card closing (Escape, a click elsewhere,
//!   the hotkey) shrinks it back to the bison, never hides it. Off, the
//!   window hides as before and the hotkey brings it back.
//! - Position: the window sits in a screen corner (`companion.corner`:
//!   the corner and the monitor's name; bottom-right of the primary monitor
//!   at first). Dragging it anywhere snaps it to the nearest corner of the
//!   monitor it was dropped on once the moves stop ([`SNAP_AFTER`]). The
//!   bison and the card both keep to that corner (`place_companion`), so
//!   the card opens toward the middle of the screen. The window-state
//!   plugin leaves this window alone (`lib.rs` denylists it).
//! - Always on top: with the bison, always; without, the saved "pinned"
//!   preference (default off), applied at startup and on every show.
//!
//! Manual test (the hotkey and window behaviour have no automated test):
//! 1. `pnpm tauri:dev`: the bison appears bottom-right, above other
//!    windows, without taking focus. Click it: the card opens from that
//!    corner. Click another app: it shrinks back to the bison. Press
//!    Ctrl+Shift+Space (Cmd+Shift+Space on macOS): the card opens, ask box
//!    focused; press again: back to the bison.
//! 2. Drag the bison to the top-left and let go: it snaps into that corner.
//!    Open the card: it opens from the top-left. Repeat on a second
//!    monitor, one with a different display scale too: it stays on the
//!    monitor you drop it on. Quit, relaunch: the bison is where you left it.
//! 3. Start an agent run: the bison carries; make it wait for an approval:
//!    it waits with the count; approve: a check for a moment, then rest.
//! 4. Settings › Appearance › Companion › Nandi on the desktop off: the
//!    bison goes; the hotkey shows and hides the card as before. On again:
//!    it's back in its corner.
//! 4. Pin it (pin button in the header), click another app: it stays above
//!    it. Unpin: it doesn't. Relaunch: the pin is remembered.
//! 5. Type in the ask box and press Enter: it grows with the chat and stays
//!    on screen; "Collapse" shrinks it back.
//! 6. Notifications: hide the companion, then make an agent ask for an
//!    approval (a `requires_approval` tool). An OS notification "… needs your
//!    approval" appears (once). Click it, open the companion: the item is at
//!    the top of Needs you; Approve resumes the run without the main window.
//!    Turn "Notify me" off in the companion's ⋯ menu: no more notifications.
//! 7. On a Request page, ⋯ → Watch in companion: the companion opens with
//!    that Request pinned at the top of Running.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow, WindowEvent,
};

pub use crate::services::companion::CompanionSnapshot;

use crate::services::chat::SendMessageResult;
use crate::services::companion::{self, Corner, CornerPlace};
use crate::services::AppState;

/// `companion://collapse`: the card shrinks back to the bison
pub const COLLAPSE_EVENT: &str = "companion://collapse";
/// `companion://expand`: the card opens (focus alone doesn't open it, so
/// pressing the bison to drag it doesn't)
pub const EXPAND_EVENT: &str = "companion://expand";

/// The bison's window, logical pixels (`COLLAPSED_SIZE` in companion-window.ts)
const COLLAPSED_SIZE: (f64, f64) = (192.0, 128.0);
/// The card's window (`EXPANDED_SIZE`)
const EXPANDED_SIZE: (f64, f64) = (420.0, 640.0);
/// A drag has ended when the window hasn't moved for this long
const SNAP_AFTER: Duration = Duration::from_millis(350);
/// Moves this soon after we placed the window are ours, not a drag
const OWN_MOVE: Duration = Duration::from_millis(500);

/// What the window is showing, as `place_companion` was last told
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Collapsed,
    Expanded,
    /// Full width; placed by the webview, never snapped
    Overlay,
}

/// Placement bookkeeping, managed in `setup`
struct Placement {
    shape: Mutex<Shape>,
    /// Until when moves are our own `set_position`
    placed_until: Mutex<Instant>,
    /// Bumped on each move; a snap runs only if no move came after it
    moves: AtomicU64,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            shape: Mutex::new(Shape::Collapsed),
            placed_until: Mutex::new(Instant::now()),
            moves: AtomicU64::new(0),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// The corner of `area` nearest `center` (ties go bottom and right).
/// Pure, for tests.
fn nearest_corner(center: (i32, i32), area_pos: (i32, i32), area_size: (u32, u32)) -> Corner {
    let mid_x = area_pos.0 + area_size.0 as i32 / 2;
    let mid_y = area_pos.1 + area_size.1 as i32 / 2;
    match (center.0 < mid_x, center.1 < mid_y) {
        (true, true) => Corner::TopLeft,
        (false, true) => Corner::TopRight,
        (true, false) => Corner::BottomLeft,
        (false, false) => Corner::BottomRight,
    }
}

/// Where a `size` window goes to sit in `corner` of `area`, `margin` from
/// its edges. Pure, for tests.
fn corner_position(
    corner: Corner,
    size: (u32, u32),
    area_pos: (i32, i32),
    area_size: (u32, u32),
    margin: i32,
) -> (i32, i32) {
    let far = (
        area_pos.0 + area_size.0 as i32,
        area_pos.1 + area_size.1 as i32,
    );
    let target = match corner {
        Corner::TopLeft => area_pos,
        Corner::TopRight => (far.0, area_pos.1),
        Corner::BottomLeft => (area_pos.0, far.1),
        Corner::BottomRight => far,
    };
    fit_inside(target, size, area_pos, area_size, margin)
}

/// The saved monitor, else the primary one
fn monitor_for(app: &AppHandle, place: &CornerPlace) -> Option<tauri::Monitor> {
    let named = place.monitor.as_ref().and_then(|name| {
        app.available_monitors()
            .ok()?
            .into_iter()
            .find(|m| m.name() == Some(name))
    });
    named.or_else(|| app.primary_monitor().ok().flatten())
}

/// Sizes the window (`size`, logical pixels; `None` keeps it) and puts it
/// in the saved corner
async fn place(app: &AppHandle, size: Option<(f64, f64)>) {
    let (Ok(window), Some(state)) = (window(app), app.try_state::<AppState>()) else {
        return;
    };
    let place = companion::corner(&state).await.unwrap_or_default();
    let Some(monitor) = monitor_for(app, &place) else {
        return;
    };
    let scale = monitor.scale_factor();
    let physical = match size {
        Some((width, height)) => {
            let size = PhysicalSize::new((width * scale) as u32, (height * scale) as u32);
            let _ = window.set_size(size);
            (size.width, size.height)
        }
        None => match window.outer_size() {
            Ok(size) => (size.width, size.height),
            Err(_) => return,
        },
    };
    let (area_pos, area_size) = work_area(&monitor);
    let (x, y) = corner_position(
        place.corner,
        physical,
        area_pos,
        area_size,
        margin(&monitor),
    );
    if let Some(placement) = app.try_state::<Placement>() {
        *lock(&placement.placed_until) = Instant::now() + OWN_MOVE;
    }
    if let Err(e) = window.set_position(PhysicalPosition::new(x, y)) {
        log::warn!("Failed to place the companion: {e}");
    }
}

/// After a drag: the nearest corner of the monitor it was dropped on
/// becomes the companion's corner, and the window moves there
async fn snap(app: &AppHandle) {
    let Ok(window) = window(app) else {
        return;
    };
    let (Ok(pos), Ok(size), Ok(Some(monitor))) = (
        window.outer_position(),
        window.outer_size(),
        window.current_monitor(),
    ) else {
        return;
    };
    let (area_pos, area_size) = work_area(&monitor);
    let center = (
        pos.x + size.width as i32 / 2,
        pos.y + size.height as i32 / 2,
    );
    let dropped = CornerPlace {
        corner: nearest_corner(center, area_pos, area_size),
        monitor: monitor.name().cloned(),
    };
    if let Some(state) = app.try_state::<AppState>() {
        if let Err(e) = companion::set_corner(&state, dropped).await {
            log::warn!("Failed to save the companion's corner: {e}");
        }
    }
    place(app, None).await;
}

/// A move: unless it's ours or the overlay's, snap once the moves stop
fn on_moved(app: &AppHandle) {
    let Some(placement) = app.try_state::<Placement>() else {
        return;
    };
    if *lock(&placement.shape) == Shape::Overlay || Instant::now() < *lock(&placement.placed_until)
    {
        return;
    }
    let seen = placement.moves.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SNAP_AFTER).await;
        let Some(placement) = app.try_state::<Placement>() else {
            return;
        };
        if placement.moves.load(Ordering::SeqCst) == seen {
            snap(&app).await;
        }
    });
}

/// Another monitor's scale. Our own placement crossed monitors: the same
/// corner, at the new size. A drag crossed them: a move like any other, so
/// it snaps to the monitor it's dropped on (placing it now would pull it
/// back to the saved one mid-drag)
fn on_scale_changed(app: &AppHandle) {
    let Some(placement) = app.try_state::<Placement>() else {
        return;
    };
    if Instant::now() < *lock(&placement.placed_until) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move { place(&app, None).await });
    } else {
        on_moved(app);
    }
}

async fn bubble_on(app: &AppHandle) -> bool {
    match app.try_state::<AppState>() {
        Some(state) => companion::is_bubble(&state).await.unwrap_or(true),
        None => false,
    }
}

/// Window label in tauri.conf.json
pub const COMPANION_LABEL: &str = "companion";

/// Global shortcut toggling the companion
pub const COMPANION_SHORTCUT: &str = "CommandOrControl+Shift+Space";

/// Gap between the window and the screen edges, logical pixels
const EDGE_MARGIN: f64 = 24.0;

/// How long `setup` waits for `AppState` (managed after it runs)
const STATE_WAIT_STEPS: u32 = 300;
const STATE_WAIT_STEP: std::time::Duration = std::time::Duration::from_millis(100);

fn window(app: &AppHandle) -> Result<WebviewWindow, String> {
    app.get_webview_window(COMPANION_LABEL)
        .ok_or_else(|| "Companion window not found".to_string())
}

/// Called once from the app's `setup`: register the companion hotkey
/// (Cmd/Ctrl+Shift+Space) and restore its pinned state. Must not fail the
/// app's start; log instead.
pub fn setup(app: &AppHandle) {
    let Ok(window) = window(app) else {
        log::error!("Companion window missing from tauri.conf.json");
        return;
    };
    // Pinned is off by default; the saved preference is applied once the
    // core is up (it's managed after `setup` runs)
    if let Err(e) = window.set_always_on_top(false) {
        log::warn!("Failed to reset companion always-on-top: {e}");
    }
    app.manage(Placement::default());
    let events_window = window.clone();
    let events_app = app.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Moved(_) => on_moved(&events_app),
        WindowEvent::ScaleFactorChanged { .. } => on_scale_changed(&events_app),
        // Closing (e.g. Alt+F4) hides it, so the hotkey can bring it back
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            let _ = events_window.hide();
        }
        _ => {}
    });

    #[cfg(desktop)]
    if let Err(e) = register_shortcut(app) {
        // Another app (or another Nandi instance) may hold it: not fatal
        log::warn!("Companion shortcut not registered: {e}");
    }

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        for _ in 0..STATE_WAIT_STEPS {
            if app.try_state::<AppState>().is_some() {
                apply_pinned(&app).await;
                if bubble_on(&app).await {
                    show_bison(&app).await;
                }
                // Only the bison: the main window waits behind it
                let state = app.state::<AppState>();
                if companion::starts_bison_only(&state).await.unwrap_or(false) {
                    if let Some(main) = app.get_webview_window("main") {
                        let _ = main.hide();
                        log::info!("Started with only the bison");
                    }
                }
                return;
            }
            tokio::time::sleep(STATE_WAIT_STEP).await;
        }
        log::warn!("Companion: core not ready; pinned state not restored");
    });
}

#[cfg(desktop)]
fn register_shortcut(app: &AppHandle) -> Result<(), String> {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

    app.global_shortcut()
        .on_shortcut(COMPANION_SHORTCUT, |app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = toggle(&app).await {
                        log::error!("Failed to toggle the companion: {e}");
                    }
                });
            }
        })
        .map_err(|e| format!("Failed to register '{COMPANION_SHORTCUT}': {e}"))?;
    log::info!("Registered companion shortcut: {COMPANION_SHORTCUT}");
    Ok(())
}

/// The bison in its corner, on top, without taking focus from the app
/// the user is in
async fn show_bison(app: &AppHandle) {
    let Ok(window) = window(app) else {
        return;
    };
    if let Some(placement) = app.try_state::<Placement>() {
        *lock(&placement.shape) = Shape::Collapsed;
    }
    let _ = app.emit_to(COMPANION_LABEL, COLLAPSE_EVENT, ());
    place(app, Some(COLLAPSED_SIZE)).await;
    if let Err(e) = window.show() {
        log::warn!("Failed to show the bison: {e}");
    }
    // After showing: on Windows, topmost set on a hidden window may not hold
    apply_pinned(app).await;
}

/// The window stays above others while the bison is on the desktop, and
/// otherwise when the card is pinned. Pure, for tests.
fn stays_on_top(pinned: bool, bubble: bool) -> bool {
    pinned || bubble
}

/// On top with the bison, else as the saved "pinned" preference says
async fn apply_pinned(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let pinned = companion::is_pinned(&state).await.unwrap_or_else(|e| {
        log::warn!("Failed to read the companion pin: {e}");
        false
    });
    let on_top = stays_on_top(pinned, bubble_on(app).await);
    if let Ok(window) = window(app) {
        if let Err(e) = window.set_always_on_top(on_top) {
            log::warn!("Failed to set companion always-on-top: {e}");
        }
    }
}

/// `(x, y)` that puts a `size` window inside `area` (position + size),
/// `margin` from the edges where it fits. Pure, for tests.
fn fit_inside(
    pos: (i32, i32),
    size: (u32, u32),
    area_pos: (i32, i32),
    area_size: (u32, u32),
    margin: i32,
) -> (i32, i32) {
    let axis = |p: i32, len: u32, start: i32, span: u32| {
        let max = start + span as i32 - len as i32 - margin;
        let min = start + margin;
        if max < min {
            start
        } else {
            p.clamp(min, max)
        }
    };
    (
        axis(pos.0, size.0, area_pos.0, area_size.0),
        axis(pos.1, size.1, area_pos.1, area_size.1),
    )
}

/// The monitor's work area (excludes the taskbar/dock), physical pixels
fn work_area(monitor: &tauri::Monitor) -> ((i32, i32), (u32, u32)) {
    let area = monitor.work_area();
    (
        (area.position.x, area.position.y),
        (area.size.width, area.size.height),
    )
}

fn margin(monitor: &tauri::Monitor) -> i32 {
    (EDGE_MARGIN * monitor.scale_factor()) as i32
}

/// Shows and focuses the card (it opens on focus), in its corner
async fn show(app: &AppHandle) -> Result<(), String> {
    let window = window(app)?;
    if !window.is_visible().unwrap_or(false) {
        place(app, None).await;
    }
    window.show().map_err(|e| format!("Failed to show: {e}"))?;
    let _ = window.unminimize();
    // After showing, as for the bison
    apply_pinned(app).await;
    window
        .set_focus()
        .map_err(|e| format!("Failed to focus: {e}"))?;
    app.emit_to(COMPANION_LABEL, EXPAND_EVENT, ())
        .map_err(|e| format!("Failed to open the card: {e}"))
}

/// The hotkey: brings the card forward, or puts it away when it's in
/// front (back to the bison, or hidden without it)
async fn toggle(app: &AppHandle) -> Result<(), String> {
    let window = window(app)?;
    let visible = window.is_visible().unwrap_or(false);
    let focused = window.is_focused().unwrap_or(false);
    if !(visible && focused) {
        // Shown but behind another window: bring it forward instead
        return show(app).await;
    }
    if bubble_on(app).await {
        app.emit_to(COMPANION_LABEL, COLLAPSE_EVENT, ())
            .map_err(|e| format!("Failed to collapse: {e}"))
    } else {
        window.hide().map_err(|e| format!("Failed to hide: {e}"))
    }
}

/// `day_start`: local midnight, epoch milliseconds
#[tauri::command]
#[specta::specta]
pub async fn get_companion_snapshot(
    state: State<'_, AppState>,
    day_start: f64,
) -> Result<CompanionSnapshot, String> {
    companion::get_companion_snapshot(&state, day_start).await
}

/// Shows (and focuses) the companion if hidden or behind other windows,
/// hides it if it's shown and focused
#[tauri::command]
#[specta::specta]
pub async fn toggle_companion_window(app: AppHandle) -> Result<(), String> {
    toggle(&app).await
}

/// Quits Nandi from the companion: with the main window closed (hidden),
/// this is the way out
#[tauri::command]
#[specta::specta]
pub fn quit_nandi(app: AppHandle) {
    log::info!("Quit from the companion");
    app.exit(0);
}

/// Closing the main window hides it and leaves the companion on screen,
/// so there's always a way back ("Open in Nandi") and a way out (Quit):
/// the bison if it's on, else the card
pub fn show_after_main_closed(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if bubble_on(&app).await {
            if !window(&app).is_ok_and(|w| w.is_visible().unwrap_or(false)) {
                show_bison(&app).await;
            }
        } else if let Err(e) = show(&app).await {
            log::warn!("Main window closed, but the companion didn't show: {e}");
        }
    });
}

/// Sizes the window for what it shows (`collapsed` or `expanded`) and
/// keeps it in its corner. `overlay` is placed by the webview and isn't
/// snapped while it's up.
#[tauri::command]
#[specta::specta]
pub async fn place_companion(app: AppHandle, shape: String) -> Result<(), String> {
    let (shape, size) = match shape.as_str() {
        "collapsed" => (Shape::Collapsed, Some(COLLAPSED_SIZE)),
        "expanded" => (Shape::Expanded, Some(EXPANDED_SIZE)),
        "overlay" => (Shape::Overlay, None),
        other => return Err(format!("Unknown companion shape '{other}'")),
    };
    if let Some(placement) = app.try_state::<Placement>() {
        *lock(&placement.shape) = shape;
        // The overlay's own placement isn't a drag either
        *lock(&placement.placed_until) = Instant::now() + OWN_MOVE;
    }
    if size.is_some() {
        place(&app, size).await;
    }
    Ok(())
}

/// Start Nandi with only the bison (the main window hidden until opened
/// from the companion)
#[tauri::command]
#[specta::specta]
pub async fn set_companion_start_bison_only(
    state: State<'_, AppState>,
    on: bool,
) -> Result<(), String> {
    companion::set_start_bison_only(&state, on).await
}

/// The bison on the desktop: on shows it (on top, in its corner); off
/// hides the window, which the hotkey then brings back
#[tauri::command]
#[specta::specta]
pub async fn set_companion_bubble(
    app: AppHandle,
    state: State<'_, AppState>,
    bubble: bool,
) -> Result<(), String> {
    companion::set_bubble(&state, bubble).await?;
    apply_pinned(&app).await;
    if bubble {
        show_bison(&app).await;
        Ok(())
    } else {
        window(&app)?
            .hide()
            .map_err(|e| format!("Failed to hide: {e}"))
    }
}

/// Keeps the card above other windows (and remembers it). With the bison
/// on the desktop the window stays on top either way: unpinning only lets
/// the card close when you click elsewhere.
#[tauri::command]
#[specta::specta]
pub async fn set_companion_pinned(
    app: AppHandle,
    state: State<'_, AppState>,
    pinned: bool,
) -> Result<(), String> {
    companion::set_pinned(&state, pinned).await?;
    window(&app)?;
    apply_pinned(&app).await;
    Ok(())
}

/// A companion chat turn: Studio (grounded by its tools), scoped to a
/// Request when `scope_request_id` is set; "@Agent …" asks that agent
#[tauri::command]
#[specta::specta]
pub async fn send_companion_message(
    state: State<'_, AppState>,
    text: String,
    conversation_id: Option<String>,
    scope_request_id: Option<String>,
) -> Result<SendMessageResult, String> {
    companion::send_companion_message(
        &state,
        &text,
        conversation_id.as_deref(),
        scope_request_id.as_deref(),
    )
    .await
}

/// "Notify me": OS notifications for approvals and questions
#[tauri::command]
#[specta::specta]
pub async fn set_companion_notify(state: State<'_, AppState>, notify: bool) -> Result<(), String> {
    companion::set_notify(&state, notify).await
}

/// "Notify me when requests come back"
#[tauri::command]
#[specta::specta]
pub async fn set_companion_notify_results(
    state: State<'_, AppState>,
    notify: bool,
) -> Result<(), String> {
    companion::set_notify_results(&state, notify).await
}

/// `companion://focus_request`: the companion pins a Request at the top
pub const FOCUS_REQUEST_EVENT: &str = "companion://focus_request";

/// Watch in companion: shows the companion with `request_id` pinned
#[tauri::command]
#[specta::specta]
pub async fn companion_focus_request(app: AppHandle, request_id: String) -> Result<(), String> {
    show(&app).await?;
    app.emit_to(
        COMPANION_LABEL,
        FOCUS_REQUEST_EVENT,
        serde_json::json!({ "requestId": request_id }),
    )
    .map_err(|e| format!("Failed to tell the companion: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: ((i32, i32), (u32, u32)) = ((0, 0), (1920, 1040));

    #[test]
    fn bottom_right_is_inside_the_work_area() {
        let (x, y) = fit_inside((1920, 1040), (380, 220), AREA.0, AREA.1, 24);
        assert_eq!((x, y), (1920 - 380 - 24, 1040 - 220 - 24));
    }

    #[test]
    fn expanding_near_the_edge_moves_the_window_back() {
        // Compact at bottom-right, then grown to 500×700
        let (x, y) = fit_inside((1516, 796), (500, 700), AREA.0, AREA.1, 24);
        assert_eq!((x, y), (1920 - 500 - 24, 1040 - 700 - 24));
        // Already inside: unchanged
        assert_eq!(
            fit_inside((100, 100), (500, 700), AREA.0, AREA.1, 24),
            (100, 100)
        );
    }

    #[test]
    fn secondary_monitors_and_tiny_screens() {
        // A monitor left of the primary one
        let area = ((-1280, 0), (1280, 1000));
        assert_eq!(
            fit_inside((-2000, 50), (380, 220), area.0, area.1, 24),
            (-1256, 50)
        );
        // Larger than the screen: pinned to the top-left corner
        assert_eq!(
            fit_inside((50, 50), (500, 700), (0, 0), (400, 600), 24),
            (0, 0)
        );
    }

    #[test]
    fn the_bison_stays_on_top_pinned_or_not() {
        assert!(
            stays_on_top(false, true),
            "unpinning keeps the bison on top"
        );
        assert!(stays_on_top(true, true));
        assert!(stays_on_top(true, false));
        assert!(!stays_on_top(false, false));
    }

    #[test]
    fn a_drop_snaps_to_the_nearest_corner() {
        let (pos, size) = AREA;
        assert_eq!(nearest_corner((100, 100), pos, size), Corner::TopLeft);
        assert_eq!(nearest_corner((1800, 50), pos, size), Corner::TopRight);
        assert_eq!(nearest_corner((10, 1000), pos, size), Corner::BottomLeft);
        assert_eq!(nearest_corner((1500, 900), pos, size), Corner::BottomRight);
        // The middle goes bottom-right
        assert_eq!(nearest_corner((960, 520), pos, size), Corner::BottomRight);
        // On a monitor left of the primary one
        let left = ((-1280, 0), (1280, 1000));
        assert_eq!(
            nearest_corner((-1200, 900), left.0, left.1),
            Corner::BottomLeft
        );
    }

    #[test]
    fn the_bison_and_the_card_keep_to_their_corner() {
        let (pos, size) = AREA;
        // The bison bottom-right, then the card grown from the same corner
        assert_eq!(
            corner_position(Corner::BottomRight, (88, 88), pos, size, 24),
            (1920 - 88 - 24, 1040 - 88 - 24)
        );
        assert_eq!(
            corner_position(Corner::BottomRight, (420, 640), pos, size, 24),
            (1920 - 420 - 24, 1040 - 640 - 24)
        );
        assert_eq!(
            corner_position(Corner::TopLeft, (420, 640), pos, size, 24),
            (24, 24)
        );
        assert_eq!(
            corner_position(Corner::TopRight, (88, 88), pos, size, 24),
            (1920 - 88 - 24, 24)
        );
        // A work area that starts below a top taskbar
        assert_eq!(
            corner_position(Corner::BottomLeft, (88, 88), (0, 40), (1920, 1000), 24),
            (24, 40 + 1000 - 88 - 24)
        );
    }

    #[test]
    fn the_shortcut_parses() {
        assert!(COMPANION_SHORTCUT
            .parse::<tauri_plugin_global_shortcut::Shortcut>()
            .is_ok());
    }

    /// Platform configs are merge patches: their `windows` array replaces
    /// the base one, so each must declare every window the app looks up
    #[test]
    fn every_platform_config_declares_the_companion_window() {
        for (name, config) in [
            ("tauri.conf.json", include_str!("../../tauri.conf.json")),
            (
                "tauri.windows.conf.json",
                include_str!("../../tauri.windows.conf.json"),
            ),
            (
                "tauri.macos.conf.json",
                include_str!("../../tauri.macos.conf.json"),
            ),
            (
                "tauri.linux.conf.json",
                include_str!("../../tauri.linux.conf.json"),
            ),
        ] {
            let config: serde_json::Value = serde_json::from_str(config).unwrap();
            let labels: Vec<&str> = config["app"]["windows"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|w| w["label"].as_str())
                .collect();
            assert!(labels.contains(&"main"), "{name}: {labels:?}");
            assert!(labels.contains(&COMPANION_LABEL), "{name}: {labels:?}");
        }
    }
}
