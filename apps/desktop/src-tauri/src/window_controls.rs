use tauri::{Window, WindowEvent};

pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if !matches!(
        event,
        WindowEvent::Resized(_)
            | WindowEvent::Focused(true)
            | WindowEvent::ScaleFactorChanged { .. }
    ) {
        return;
    }

    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWindow;
        use tauri::Manager;

        let Some(webview) = window.get_webview_window(window.label()) else {
            return;
        };
        let _ = webview.with_webview(|platform| {
            let pointer = platform.ns_window().cast::<NSWindow>();
            if pointer.is_null() {
                return;
            }
            // SAFETY: with_webview runs on the main thread. Tauri owns this NSWindow
            // for the callback's duration; no native reference escapes the callback.
            let native_window = unsafe { &*pointer };
            if let Some(content_view) = native_window.contentView() {
                // Complete AppKit's pending titlebar layout before Wry's content-view
                // drawRect reapplies the configured native traffic-light inset.
                // SAFETY: the window and its view hierarchy are accessed only during
                // this main-thread callback, and the returned view remains retained.
                if let Some(frame_view) = unsafe { content_view.superview() } {
                    frame_view.layoutSubtreeIfNeeded();
                }
                content_view.setNeedsDisplay(true);
                // A deferred redraw can be skipped beneath the opaque webview. Flush
                // this view now, including when a restored window keeps the same size.
                content_view.displayIfNeededIgnoringOpacity();
            }
        });
    }

    #[cfg(not(target_os = "macos"))]
    let _ = window;
}
