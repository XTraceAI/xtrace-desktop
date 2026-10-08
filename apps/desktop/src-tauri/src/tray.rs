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

/// On. Under the earlier app ID `ai.xtrace.desktop`, macOS 26 on one Mac gave
/// this item no place in the menu bar, while the same code under another ID
/// was shown; the app ID changed and the item is back on. This one switch
/// turns the item, its popover window and the main window's hide-on-close on
/// or off together. With only some of them off, closing the main window would
/// leave the app running with no window and no item to bring it back; with all
/// of them off, closing it ends the app, as it did before there was a menu-bar
/// item.
pub const ENABLED: bool = true;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IconAction {
    Show,
    Dismiss,
}

/// Plain event state only. Native application objects never leave a main-thread callback.
#[derive(Default)]
struct TrayInteraction {
    last_blur_hide: Option<Instant>,
    icon_close_pending: bool,
    previous_app: Option<i32>,
    generation: u64,
}

impl TrayInteraction {
    fn icon_action(
        &mut self,
        button: MouseButton,
        button_state: MouseButtonState,
        visible: bool,
        now: Instant,
    ) -> Option<IconAction> {
        if button != MouseButton::Left {
            return None;
        }
        if button_state == MouseButtonState::Down {
            // Keep this press's close intent even if blur hides the popover
            // before release, including a press held longer than the guard.
            self.icon_close_pending = visible || !should_open(self.last_blur_hide, now);
            return None;
        }
        let close = std::mem::take(&mut self.icon_close_pending)
            || visible
            || !should_open(self.last_blur_hide, now);
        Some(if close {
            IconAction::Dismiss
        } else {
            IconAction::Show
        })
    }

    fn shown(&mut self, previous_app: Option<i32>) {
        self.generation = self.generation.wrapping_add(1);
        self.previous_app = previous_app.filter(|pid| *pid > 0);
        self.last_blur_hide = None;
        self.icon_close_pending = false;
    }

    fn blur_hidden(&mut self, now: Instant) {
        // A queued Escape from before an outside click cannot return focus.
        // Keep the PID for an icon release that follows this same blur.
        self.generation = self.generation.wrapping_add(1);
        self.last_blur_hide = Some(now);
    }

    fn discard_return(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.previous_app = None;
    }

    /// None is an obsolete request; Some(None) is a current close with no return target.
    fn dismiss(&mut self, generation: u64) -> Option<Option<i32>> {
        if self.generation != generation {
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        self.last_blur_hide = None;
        Some(self.previous_app.take())
    }
}

#[derive(Default)]
pub struct TrayState {
    interaction: Mutex<TrayInteraction>,
}

/// One decision shared by the native return and event-sequence checks.
#[cfg(any(target_os = "macos", test))]
fn return_target(previous: Option<i32>, frontmost: Option<i32>, own_pid: i32) -> Option<i32> {
    previous.filter(|pid| *pid > 0 && *pid != own_pid && frontmost == Some(own_pid))
}

#[cfg(target_os = "macos")]
fn frontmost_pid() -> Option<i32> {
    objc2::MainThreadMarker::new()?;
    objc2_app_kit::NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|app| app.processIdentifier())
        .filter(|pid| *pid > 0)
}

#[cfg(not(target_os = "macos"))]
fn frontmost_pid() -> Option<i32> {
    None
}

#[cfg(target_os = "macos")]
fn return_to_previous(previous: Option<i32>) {
    use objc2_app_kit::{NSApplication, NSApplicationActivationOptions, NSRunningApplication};
    let Some(main_thread) = objc2::MainThreadMarker::new() else {
        return;
    };
    let Some(pid) = previous else {
        return;
    };
    let Some(target) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
        return;
    };
    if target.isTerminated() {
        return;
    }
    let Ok(own_pid) = i32::try_from(std::process::id()) else {
        return;
    };
    // Check at the point of use, after hiding. A user who selected another
    // app meanwhile keeps that app; a missing or exited target is never launched.
    if return_target(Some(pid), frontmost_pid(), own_pid).is_none() {
        return;
    }
    NSApplication::sharedApplication(main_thread).yieldActivationToApplication(&target);
    // Cooperative activation on supported macOS (14+), without all-windows
    // or ignoring-other-apps flags. AppKit can refuse the request.
    let _ = target.activateWithOptions(NSApplicationActivationOptions::empty());
}

#[cfg(not(target_os = "macos"))]
fn return_to_previous(_previous: Option<i32>) {}

/// Bring the existing main window forward: shown, restored and focused.
pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    // Open, Dock reopen and a second launch all mean the user wants this app.
    // They also invalidate a close that was queued before this request.
    if let Some(state) = app.try_state::<TrayState>() {
        state
            .interaction
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .discard_return();
    }
    if let Some(window) = app.get_webview_window(MAIN_LABEL) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn hide_popover<R: Runtime>(window: &WebviewWindow<R>) {
    let _ = hide_popover_result(window);
}

fn hide_popover_result<R: Runtime>(window: &WebviewWindow<R>) -> bool {
    if !window.is_visible().unwrap_or(true) {
        return true;
    }
    if window.hide().is_err() {
        return false;
    }
    let _ = window.emit_to(TRAY_LABEL, TRAY_HIDDEN_EVENT, ());
    true
}

/// An explicit icon or Escape close, unlike outside blur or Open.
pub fn dismiss_popover<R: Runtime>(window: &WebviewWindow<R>) {
    let app = window.app_handle().clone();
    let generation = app.try_state::<TrayState>().map(|state| {
        state
            .interaction
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .generation
    });
    let popover = window.clone();
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let previous = match generation {
            Some(generation) => {
                let state = handle.state::<TrayState>();
                let Some(previous) = state
                    .interaction
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .dismiss(generation)
                else {
                    return;
                };
                previous
            }
            None => None,
        };
        // The state lock is released before any native window/focus call.
        if hide_popover_result(&popover) {
            return_to_previous(previous);
        }
    });
}

/// Called only on the main thread, after the one icon decision accepted a show.
fn show_popover<R: Runtime>(app: &AppHandle<R>, icon: Rect) {
    let Some(window) = app.get_webview_window(TRAY_LABEL) else {
        return;
    };
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
    let own_pid = i32::try_from(std::process::id()).ok();
    let previous = frontmost_pid().filter(|pid| Some(*pid) != own_pid);
    app.state::<TrayState>()
        .interaction
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .shown(previous);
    if window.show().is_err() {
        app.state::<TrayState>()
            .interaction
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .discard_return();
        return;
    }
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
        button,
        button_state,
        rect,
        ..
    } = event
    {
        let app = tray.app_handle().clone();
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            let Some(window) = handle.get_webview_window(TRAY_LABEL) else {
                return;
            };
            let visible = window.is_visible().unwrap_or(false);
            let action = handle
                .state::<TrayState>()
                .interaction
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .icon_action(button, button_state, visible, Instant::now());
            match action {
                Some(IconAction::Show) => show_popover(&handle, rect),
                Some(IconAction::Dismiss) => dismiss_popover(&window),
                None => {}
            }
        });
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
                window
                    .state::<TrayState>()
                    .interaction
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .blur_hidden(Instant::now());
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

    const OWN: i32 = 10;
    const PREVIOUS: i32 = 20;

    fn down(state: &mut TrayInteraction, visible: bool, now: Instant) {
        assert_eq!(
            state.icon_action(MouseButton::Left, MouseButtonState::Down, visible, now),
            None
        );
    }
    fn up(state: &mut TrayInteraction, visible: bool, now: Instant) -> IconAction {
        state
            .icon_action(MouseButton::Left, MouseButtonState::Up, visible, now)
            .unwrap()
    }
    fn finish_close(state: &mut TrayInteraction, frontmost: Option<i32>) -> Option<i32> {
        let previous = state.dismiss(state.generation).unwrap();
        return_target(previous, frontmost, OWN)
    }

    #[test]
    fn two_slow_or_quick_clicks_show_then_dismiss_without_a_main_window_action() {
        for gap in [Duration::from_millis(20), Duration::from_secs(2)] {
            let mut state = TrayInteraction::default();
            let now = Instant::now();
            down(&mut state, false, now);
            assert_eq!(up(&mut state, false, now + gap), IconAction::Show);
            state.shown(Some(PREVIOUS));
            down(&mut state, true, now + gap * 2);
            assert_eq!(up(&mut state, true, now + gap * 3), IconAction::Dismiss);
            assert_eq!(finish_close(&mut state, Some(OWN)), Some(PREVIOUS));
            // The completed close consumes its return once and does not swallow
            // the next real click, even when all clicks are close together.
            down(&mut state, false, now + gap * 4);
            assert_eq!(up(&mut state, false, now + gap * 5), IconAction::Show);
        }
    }

    #[test]
    fn a_long_second_press_still_closes_when_blur_hides_before_release() {
        let mut state = TrayInteraction::default();
        let now = Instant::now();
        state.shown(Some(PREVIOUS));
        down(&mut state, true, now);
        state.blur_hidden(now + Duration::from_millis(10));
        let released = now + Duration::from_secs(2);
        assert!(
            should_open(state.last_blur_hide, released),
            "the old time guard has expired"
        );
        assert_eq!(up(&mut state, false, released), IconAction::Dismiss);
        assert_eq!(finish_close(&mut state, Some(OWN)), Some(PREVIOUS));
    }

    #[test]
    fn blur_before_down_or_an_up_only_guarded_close_can_return_once() {
        let now = Instant::now();
        let mut state = TrayInteraction::default();
        state.shown(Some(PREVIOUS));
        state.blur_hidden(now);
        down(&mut state, false, now + Duration::from_millis(10));
        assert_eq!(
            up(&mut state, false, now + Duration::from_secs(2)),
            IconAction::Dismiss
        );
        assert_eq!(finish_close(&mut state, Some(OWN)), Some(PREVIOUS));
        assert_eq!(finish_close(&mut state, Some(OWN)), None);

        state.shown(Some(PREVIOUS));
        state.blur_hidden(now);
        assert_eq!(
            up(&mut state, false, now + Duration::from_millis(100)),
            IconAction::Dismiss
        );
        assert_eq!(finish_close(&mut state, Some(OWN)), Some(PREVIOUS));
    }

    #[test]
    fn outside_blur_does_not_return_and_the_next_show_replaces_its_old_target() {
        let now = Instant::now();
        let mut state = TrayInteraction::default();
        state.shown(Some(PREVIOUS));
        let queued_escape = state.generation;
        state.blur_hidden(now);
        assert_eq!(state.dismiss(queued_escape), None);
        assert_eq!(state.previous_app, Some(PREVIOUS));
        assert_eq!(return_target(state.previous_app, Some(30), OWN), None);
        down(&mut state, false, now + REOPEN_GUARD);
        assert_eq!(up(&mut state, false, now + REOPEN_GUARD), IconAction::Show);
        state.shown(Some(30));
        assert_eq!(finish_close(&mut state, Some(OWN)), Some(30));
    }

    #[test]
    fn escape_returns_once_but_keeps_another_apps_new_focus() {
        let mut state = TrayInteraction::default();
        state.shown(Some(PREVIOUS));
        let escape = state.generation;
        assert_eq!(
            return_target(state.dismiss(escape).unwrap(), Some(OWN), OWN),
            Some(PREVIOUS)
        );
        assert_eq!(state.dismiss(escape), None);
        state.shown(Some(PREVIOUS));
        assert_eq!(finish_close(&mut state, Some(30)), None);
        assert_eq!(state.previous_app, None);
        for (previous, current) in [
            (None, Some(OWN)),
            (Some(OWN), Some(OWN)),
            (Some(-1), Some(OWN)),
            (Some(PREVIOUS), None),
        ] {
            assert_eq!(return_target(previous, current, OWN), None);
        }
    }

    #[test]
    fn open_dock_and_second_launch_discard_a_queued_return_and_new_show_invalidates_it() {
        let mut state = TrayInteraction::default();
        state.shown(Some(PREVIOUS));
        let queued = state.generation;
        // All three routes call show_main, whose one decision discards return.
        state.discard_return();
        assert_eq!(state.dismiss(queued), None);
        assert_eq!(finish_close(&mut state, Some(OWN)), None);
        state.shown(Some(PREVIOUS));
        let queued = state.generation;
        state.shown(Some(30));
        assert_eq!(state.dismiss(queued), None);
        assert_eq!(finish_close(&mut state, Some(OWN)), Some(30));
    }

    #[test]
    fn right_click_does_not_toggle_or_consume_the_return_target() {
        let mut state = TrayInteraction::default();
        state.shown(Some(PREVIOUS));
        for button_state in [MouseButtonState::Down, MouseButtonState::Up] {
            assert_eq!(
                state.icon_action(MouseButton::Right, button_state, true, Instant::now()),
                None
            );
        }
        assert_eq!(state.previous_app, Some(PREVIOUS));
    }

    #[test]
    fn show_main_invalidates_return_state_through_the_real_entry_point() {
        use tauri::test::{mock_builder, mock_context, noop_assets};
        let app = mock_builder().build(mock_context(noop_assets())).unwrap();
        let mut interaction = TrayInteraction::default();
        interaction.shown(Some(PREVIOUS));
        let queued = interaction.generation;
        app.manage(TrayState {
            interaction: Mutex::new(interaction),
        });
        WebviewWindowBuilder::new(&app, MAIN_LABEL, WebviewUrl::default())
            .build()
            .unwrap();
        // Mock window operations do not prove native focus or visibility;
        // this checks that Open/Dock's shared entry point cancels the return.
        show_main(app.handle());
        let state = app.state::<TrayState>();
        let mut interaction = state.interaction.lock().unwrap();
        assert_eq!(interaction.previous_app, None);
        assert_eq!(interaction.dismiss(queued), None);
    }

    /// The main window's close button on Tauri's mock runtime, through the
    /// real window handler, and through the real `setup` only while the item
    /// is off (with it on, `setup` creates the menu-bar item, which the mock
    /// runtime deadlocks on): what `setup` created, and what the run loop then
    /// reported.
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
                    // With the item on, `setup` creates the menu-bar item,
                    // which the mock runtime deadlocks on: it waits for the
                    // main thread from inside the loop. The close is then
                    // tested through the window handler alone.
                    if !ENABLED {
                        setup(app)?;
                    }
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

    /// The main window's close button follows the one switch. With the
    /// menu-bar item on, the close is turned into a hide and the app keeps
    /// running, so the item can bring the window back. With it off, nothing
    /// may outlive the main window: no popover is created to stay behind
    /// hidden, the close is not turned into a hide, and the run loop ends
    /// through its ordinary exit, so the Dock's next click is an ordinary
    /// launch, never a request to an app left running with no window and no
    /// item.
    #[test]
    fn closing_the_main_window_follows_the_menu_bar_switch() {
        let (sent, outcome) = std::sync::mpsc::channel();
        // On its own thread: an app that never ends is reported, not waited on.
        std::thread::spawn(move || sent.send(close_main_window()));
        let (created, seen) = outcome
            .recv_timeout(Duration::from_secs(10))
            .expect("the run loop ends");
        if ENABLED {
            // Still running after the close, until the test destroys the
            // hidden window.
            assert_eq!(
                seen,
                ["close requested", "still running", "exit requested", "exit"]
            );
        } else {
            assert_eq!(created, [MAIN_LABEL]);
            assert_eq!(seen, ["close requested", "exit requested", "exit"]);
        }
    }
}
