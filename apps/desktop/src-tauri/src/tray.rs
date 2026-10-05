//! The menu-bar item: one template icon whose click toggles a small popover
//! window anchored under it, and a menu that opens the main window or quits.
//! The popover is a separate, narrowly permitted webview; it holds no state of
//! its own and reads today's figures when it is shown.
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{
    AppHandle, Emitter, LogicalPosition, Manager, Rect, Runtime, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, Window, WindowEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
};

/// Off, and not ready: on macOS 26 the system gave neither this item nor a
/// plain AppKit status item from this app a place in the menu bar, and no
/// product fix is known. This one switch turns off the item, its popover
/// window and the main window's hide-on-close together. With only some of them
/// off, closing the main window would leave the app running with no window and
/// no item to bring it back; with all of them off, closing it ends the app, as
/// it did before there was a menu-bar item.
pub const ENABLED: bool = false;

pub const MAIN_LABEL: &str = "main";
pub const TRAY_LABEL: &str = "tray";
/// Sent to the popover each time it is shown, so it reads again.
pub const TRAY_SHOWN_EVENT: &str = "tray://shown";
/// Sent to the popover each time it is hidden, so it stops reading.
pub const TRAY_HIDDEN_EVENT: &str = "tray://hidden";
pub const WIDTH: f64 = 360.0;
pub const HEIGHT: f64 = 540.0;
/// Space between the menu-bar item and the popover, and the popover's
/// minimum distance from a screen's usable edges, in points.
const GAP: f64 = 6.0;
const MARGIN: f64 = 8.0;
/// A click on the menu-bar item first takes focus from the open popover, which
/// hides it; the click that follows within this interval only closes it.
const REOPEN_GUARD: Duration = Duration::from_millis(400);
const OPEN_ID: &str = "tray-open";
const QUIT_ID: &str = "tray-quit";

/// A screen in global logical points, with the scale its physical
/// coordinates were reported at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    pub frame: (f64, f64, f64, f64),
    pub work: (f64, f64, f64, f64),
    pub scale: f64,
}

impl Screen {
    fn from_monitor(monitor: &tauri::Monitor) -> Self {
        let scale = monitor.scale_factor();
        let (position, size, work) = (monitor.position(), monitor.size(), monitor.work_area());
        Self {
            frame: (
                f64::from(position.x) / scale,
                f64::from(position.y) / scale,
                f64::from(size.width) / scale,
                f64::from(size.height) / scale,
            ),
            work: (
                f64::from(work.position.x) / scale,
                f64::from(work.position.y) / scale,
                f64::from(work.size.width) / scale,
                f64::from(work.size.height) / scale,
            ),
            scale,
        }
    }
}

/// The popover's top-left point, centered under the item and kept inside the
/// usable area of the screen the item is on. The item's rectangle is
/// reported at its own screen's scale, so each screen is tried at its scale.
pub fn anchor(icon: Rect, screens: &[Screen], fallback_scale: f64) -> (f64, f64) {
    let logical = |scale: f64| {
        let position = icon.position.to_logical::<f64>(scale);
        let size = icon.size.to_logical::<f64>(scale);
        (position.x, position.y, size.width, size.height)
    };
    let found = screens.iter().find_map(|screen| {
        let (x, y, width, height) = logical(screen.scale);
        let (cx, cy) = (x + width / 2.0, y + height / 2.0);
        let (sx, sy, sw, sh) = screen.frame;
        (cx >= sx && cx < sx + sw && cy >= sy && cy < sy + sh)
            .then_some((screen, (x, y, width, height)))
    });
    let (screen, (x, y, width, height)) = match found {
        Some((screen, icon)) => (Some(screen), icon),
        None => (None, logical(fallback_scale)),
    };
    let mut left = x + width / 2.0 - WIDTH / 2.0;
    let mut top = y + height + GAP;
    if let Some(screen) = screen {
        let (wx, wy, ww, _) = screen.work;
        left = left.min(wx + ww - WIDTH - MARGIN).max(wx + MARGIN);
        top = top.max(wy);
    }
    (left.round(), top.round())
}

/// Whether a click that finds the popover hidden should open it: not when a
/// loss of focus hid it just now, because that loss was this same click.
pub fn should_open(last_blur_hide: Option<Instant>, now: Instant) -> bool {
    last_blur_hide.is_none_or(|hidden| now.saturating_duration_since(hidden) >= REOPEN_GUARD)
}

#[derive(Default)]
pub struct TrayState {
    last_blur_hide: Mutex<Option<Instant>>,
}

/// Bring the existing main window forward: shown, restored and focused.
pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(MAIN_LABEL) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn hide_popover<R: Runtime>(window: &WebviewWindow<R>) {
    if window.is_visible().unwrap_or(true) {
        let _ = window.hide();
        let _ = window.emit_to(TRAY_LABEL, TRAY_HIDDEN_EVENT, ());
    }
}

fn toggle<R: Runtime>(app: &AppHandle<R>, icon: Rect) {
    let Some(window) = app.get_webview_window(TRAY_LABEL) else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        hide_popover(&window);
        return;
    }
    let state = app.state::<TrayState>();
    let last = *state
        .last_blur_hide
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !should_open(last, Instant::now()) {
        return;
    }
    let screens: Vec<Screen> = app
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(Screen::from_monitor)
        .collect();
    let fallback = app
        .primary_monitor()
        .ok()
        .flatten()
        .map_or(1.0, |monitor| monitor.scale_factor());
    let (x, y) = anchor(icon, &screens, fallback);
    let _ = window.set_position(LogicalPosition::new(x, y));
    let _ = window.show();
    let _ = window.set_focus();
    let _ = window.emit_to(TRAY_LABEL, TRAY_SHOWN_EVENT, ());
}

fn on_menu<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    match event.id().as_ref() {
        OPEN_ID => show_main(app),
        // The same exit path as the app menu: RunEvent::Exit still stops the
        // index and closes the database.
        QUIT_ID => app.exit(0),
        _ => {}
    }
}

fn on_icon<R: Runtime>(tray: &TrayIcon<R>, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        rect,
        ..
    } = event
    {
        toggle(tray.app_handle(), rect);
    }
}

/// Create the hidden popover window and the menu-bar item. While the item is
/// off this creates nothing: a hidden popover alone would keep the app running
/// after its main window closed.
pub fn setup<R: Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
    if !ENABLED {
        return Ok(());
    }
    app.manage(TrayState::default());
    WebviewWindowBuilder::new(app, TRAY_LABEL, WebviewUrl::App("index.html#/tray".into()))
        .title("XTrace")
        .inner_size(WIDTH, HEIGHT)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .closable(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(false)
        .build()?;
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, OPEN_ID, "Open XTrace Desktop", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, QUIT_ID, "Quit XTrace Desktop", true, None::<&str>)?,
        ],
    )?;
    TrayIconBuilder::with_id("xtrace")
        .icon(tauri::include_image!("icons/tray-template.png"))
        .icon_as_template(true)
        .tooltip("XTrace Desktop")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu)
        .on_tray_icon_event(on_icon)
        .build(app)?;
    Ok(())
}

/// The popover hides when it loses focus; the main window hides instead of
/// closing, so the menu-bar item can bring it back. While the item is off the
/// main window's close is left alone: it closes, and the app ends with it.
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    match (window.label(), event) {
        (TRAY_LABEL, WindowEvent::Focused(false)) => {
            let Some(popover) = window.get_webview_window(TRAY_LABEL) else {
                return;
            };
            if popover.is_visible().unwrap_or(false) {
                *window
                    .state::<TrayState>()
                    .last_blur_hide
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Instant::now());
                hide_popover(&popover);
            }
        }
        (MAIN_LABEL, WindowEvent::CloseRequested { api, .. }) if ENABLED => {
            api.prevent_close();
            let _ = window.hide();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::{PhysicalPosition, PhysicalSize};

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            position: PhysicalPosition::new(x, y).into(),
            size: PhysicalSize::new(width, height).into(),
        }
    }
    fn screen(frame: (f64, f64, f64, f64), menu_bar: f64, scale: f64) -> Screen {
        Screen {
            frame,
            work: (frame.0, frame.1 + menu_bar, frame.2, frame.3 - menu_bar),
            scale,
        }
    }

    #[test]
    fn tray_popover_centers_under_the_item_at_each_scale() {
        // A 1x display: a 30x24 point item at x=1000.
        let one = [screen((0.0, 0.0, 1920.0, 1080.0), 24.0, 1.0)];
        assert_eq!(
            anchor(rect(1000.0, 0.0, 30.0, 24.0), &one, 1.0),
            (835.0, 30.0)
        );
        // The same item reported at 2x lands on the same points.
        let two = [screen((0.0, 0.0, 1512.0, 982.0), 37.0, 2.0)];
        assert_eq!(
            anchor(rect(2000.0, 0.0, 60.0, 74.0), &two, 2.0),
            (835.0, 43.0)
        );
    }

    #[test]
    fn tray_popover_stays_inside_the_usable_area() {
        let screens = [screen((0.0, 0.0, 1512.0, 982.0), 37.0, 2.0)];
        // Near the right edge: clamped to 8 points from it.
        assert_eq!(
            anchor(rect(2960.0, 0.0, 48.0, 74.0), &screens, 2.0),
            (1512.0 - WIDTH - MARGIN, 43.0)
        );
        // Near the left edge.
        assert_eq!(anchor(rect(10.0, 0.0, 48.0, 74.0), &screens, 2.0).0, MARGIN);
    }

    #[test]
    fn tray_popover_uses_the_screen_the_item_is_on_with_mixed_scales() {
        // A 2x built-in display, and a 1x display to its left at negative x.
        let screens = [
            screen((0.0, 0.0, 1512.0, 982.0), 37.0, 2.0),
            screen((-1920.0, -98.0, 1920.0, 1080.0), 24.0, 1.0),
        ];
        // An item on the 1x display, reported at 1x: read at 2x its center
        // would fall outside the built-in display, so the 1x screen is used.
        assert_eq!(
            anchor(rect(-400.0, -98.0, 30.0, 24.0), &screens, 2.0),
            (-565.0, -68.0)
        );
        // Near that display's right edge, it is clamped to that display.
        assert_eq!(
            anchor(rect(-40.0, -98.0, 30.0, 24.0), &screens, 2.0).0,
            -WIDTH - MARGIN
        );
        // No screen contains it: the fallback scale still centers it.
        assert_eq!(
            anchor(rect(9000.0, 0.0, 60.0, 48.0), &[], 2.0),
            (4335.0, 30.0)
        );
    }

    #[test]
    fn tray_click_that_blurred_the_popover_does_not_reopen_it() {
        let now = Instant::now();
        assert!(should_open(None, now));
        assert!(!should_open(Some(now), now + Duration::from_millis(100)));
        assert!(should_open(Some(now), now + REOPEN_GUARD));
        // A clock read before the recorded hide never reopens early.
        assert!(!should_open(Some(now + Duration::from_millis(50)), now));
    }

    /// The main window's close button on Tauri's mock runtime, through the real
    /// `setup` and window handler: what `setup` created, and what the run loop
    /// then reported.
    fn close_main_window() -> (Vec<String>, Vec<&'static str>) {
        use std::sync::{Arc, Mutex};
        use tauri::{
            RunEvent,
            test::{mock_builder, mock_context, noop_assets},
        };

        let created = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let app = mock_builder()
            .setup({
                let created = Arc::clone(&created);
                move |app| {
                    setup(app)?;
                    let mut made: Vec<String> = app.webview_windows().into_keys().collect();
                    made.sort();
                    if app.tray_by_id("xtrace").is_some() {
                        made.push("menu-bar item".into());
                    }
                    *created.lock().unwrap() = made;
                    Ok(())
                }
            })
            .build(mock_context(noop_assets()))
            .unwrap();
        // The configuration's main window: it exists before setup runs.
        WebviewWindowBuilder::new(&app, MAIN_LABEL, WebviewUrl::default())
            .build()
            .unwrap();

        let (mut close_requested, mut still_running) = (false, false);
        app.run({
            let seen = Arc::clone(&seen);
            move |handle, event| match event {
                // The close button.
                RunEvent::Ready => handle
                    .get_webview_window(MAIN_LABEL)
                    .unwrap()
                    .close()
                    .unwrap(),
                // The mock runtime reports a window's event to the run loop
                // only; the real one also hands it to the window handler.
                RunEvent::WindowEvent { label, event, .. } => {
                    let window = handle.get_webview_window(&label).unwrap();
                    on_window_event(&window.as_ref().window(), &event);
                    close_requested = true;
                    seen.lock().unwrap().push("close requested");
                }
                RunEvent::ExitRequested { .. } => seen.lock().unwrap().push("exit requested"),
                RunEvent::Exit => seen.lock().unwrap().push("exit"),
                // The loop goes on after the close only while a window is
                // still open. Destroy what is left, so the loop ends and the
                // window that stayed is reported.
                RunEvent::MainEventsCleared if close_requested && !still_running => {
                    still_running = true;
                    seen.lock().unwrap().push("still running");
                    for window in handle.webview_windows().into_values() {
                        let _ = window.destroy();
                    }
                }
                _ => {}
            }
        });
        let (created, seen) = (created.lock().unwrap(), seen.lock().unwrap());
        (created.clone(), seen.clone())
    }

    /// With the menu-bar item off, nothing may outlive the main window: no
    /// popover is created to stay behind hidden, the close button's request is
    /// not turned into a hide, and the run loop ends through its ordinary exit.
    /// The Dock's next click is then an ordinary launch, never a request to an
    /// app left running with no window and no item.
    #[test]
    fn tray_off_closing_the_main_window_ends_the_app() {
        let (sent, outcome) = std::sync::mpsc::channel();
        // On its own thread: an app that never ends is reported, not waited on.
        std::thread::spawn(move || sent.send(close_main_window()));
        let (created, seen) = outcome
            .recv_timeout(Duration::from_secs(10))
            .expect("the app ends once its main window is closed");
        assert_eq!(created, [MAIN_LABEL]);
        assert_eq!(seen, ["close requested", "exit requested", "exit"]);
    }
}
