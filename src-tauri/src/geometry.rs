use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Monitor, PhysicalPosition,
    PhysicalSize,
};

use crate::settings::SettingsState;
use crate::{platform, MAIN_WINDOW};

/// Fixed size of the transparent host window in logical px. The visible notch
/// animates inside it; everything outside the hit rect is click-through.
/// Keep in sync with `WINDOW` in src/lib/layout.ts.
pub const WINDOW_WIDTH: f64 = 640.0;
pub const WINDOW_HEIGHT: f64 = 280.0;

const CURSOR_POLL: Duration = Duration::from_millis(33);
const DISPLAY_POLL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenInfo {
    pub platform: String,
    pub has_notch: bool,
    pub notch_width: f64,
    pub notch_height: f64,
    pub menu_bar_height: f64,
    pub display_name: String,
    /// Something is covering this display end to end. Screens with a cutout
    /// ignore it — the notch is drawn in space the video never gets anyway.
    pub fullscreen: bool,
}

/// Where the window's top-left corner sits in the coordinate space of
/// `platform::cursor_position`, and how many of those units a logical px is.
#[derive(Debug, Clone, Copy)]
struct Placement {
    x: f64,
    y: f64,
    scale: f64,
}

#[derive(Default)]
pub struct Geometry(Mutex<Inner>);

#[derive(Default)]
struct Inner {
    placement: Option<Placement>,
    hidden: bool,
    hit_width: f64,
    hit_height: f64,
    screen: ScreenInfo,
    monitor_signature: String,
    /// Logical, top-left based frame of the monitor the notch sits on.
    monitor_rect: Option<(f64, f64, f64, f64)>,
}

impl Geometry {
    pub fn screen(&self) -> ScreenInfo {
        self.0.lock().unwrap().screen.clone()
    }

    /// The monitor frame to test for full-screen windows, in the coordinates
    /// `platform::fullscreen_over` expects.
    fn monitor_rect(&self) -> Option<(f64, f64, f64, f64)> {
        self.0.lock().unwrap().monitor_rect
    }

    /// Records what the display is doing; hands back the screen to announce
    /// when that changed.
    fn set_fullscreen(&self, fullscreen: bool) -> Option<ScreenInfo> {
        let mut inner = self.0.lock().unwrap();
        // A cutout is out of the video's way already, so it never collapses.
        let fullscreen = fullscreen && !inner.screen.has_notch;
        if inner.screen.fullscreen == fullscreen {
            return None;
        }
        inner.screen.fullscreen = fullscreen;
        Some(inner.screen.clone())
    }

    /// The interactive area: `width` x `height` logical px, centered at the
    /// top edge of the window.
    fn store_hit_rect(&self, width: f64, height: f64) -> f64 {
        let mut inner = self.0.lock().unwrap();
        inner.hit_width = width.clamp(0.0, WINDOW_WIDTH);
        inner.hit_height = height.clamp(0.0, WINDOW_HEIGHT);
        inner.placement.map(|placement| placement.scale).unwrap_or(1.0)
    }

    pub fn set_hidden(&self, hidden: bool) {
        self.0.lock().unwrap().hidden = hidden;
    }

    /// The hit rect in the cursor's own coordinates: where a drop has to be
    /// caught on Windows (see platform::win_drop).
    #[cfg(windows)]
    fn hit_rect_on_screen(&self) -> Option<(i32, i32, i32, i32)> {
        let inner = self.0.lock().unwrap();
        let placement = inner.placement.filter(|_| !inner.hidden)?;
        let left = placement.x + (WINDOW_WIDTH - inner.hit_width) / 2.0 * placement.scale;
        Some((
            left.round() as i32,
            placement.y.round() as i32,
            (inner.hit_width * placement.scale).round() as i32,
            (inner.hit_height * placement.scale).round() as i32,
        ))
    }

    /// Converts a cursor position to window-local logical px, returning it
    /// only when it falls inside the hit rect.
    fn locate(&self, (cursor_x, cursor_y): (f64, f64)) -> Option<(f64, f64)> {
        let inner = self.0.lock().unwrap();
        let placement = inner.placement.filter(|_| !inner.hidden)?;
        let x = (cursor_x - placement.x) / placement.scale;
        let y = (cursor_y - placement.y) / placement.scale;
        let left = (WINDOW_WIDTH - inner.hit_width) / 2.0;
        let inside = x >= left && x <= left + inner.hit_width && y >= -1.0 && y <= inner.hit_height;
        inside.then_some((x, y.max(0.0)))
    }
}

fn monitor_signature(monitors: &[Monitor]) -> String {
    monitors
        .iter()
        .map(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            format!(
                "{:?}@{},{}:{}x{}*{}",
                monitor.name(),
                position.x,
                position.y,
                size.width,
                size.height,
                monitor.scale_factor()
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn target_monitor(app: &AppHandle) -> Option<Monitor> {
    let window = app.get_webview_window(MAIN_WINDOW)?;
    let monitors = window.available_monitors().unwrap_or_default();
    app.state::<SettingsState>()
        .get()
        .display
        .and_then(|label| {
            monitors
                .iter()
                .find(|monitor| platform::display_label(monitor).as_ref() == Some(&label))
                .cloned()
        })
        .or_else(|| window.primary_monitor().ok().flatten())
        .or_else(|| monitors.into_iter().next())
}

/// Moves the window to the top center of the target monitor and refreshes the
/// screen metrics. Must run on the main thread (NSScreen access on macOS).
pub fn place_window(app: &AppHandle) {
    let (Some(window), Some(monitor)) = (app.get_webview_window(MAIN_WINDOW), target_monitor(app))
    else {
        return;
    };
    let scale = monitor.scale_factor();
    let position = monitor.position();
    let size = monitor.size();

    let placement = if cfg!(target_os = "macos") {
        // tao positions windows in logical, top-left based global coordinates
        // on macOS, the same space CGEventGetLocation reports the cursor in.
        let x = (position.x as f64 / scale + (size.width as f64 / scale - WINDOW_WIDTH) / 2.0)
            .round();
        let y = position.y as f64 / scale;
        let _ = window.set_size(LogicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT));
        let _ = window.set_position(LogicalPosition::new(x, y));
        Placement { x, y, scale: 1.0 }
    } else {
        // Windows desktop coordinates are physical pixels, and monitors can
        // have different DPI, so compute everything in the target's pixels.
        let width = (WINDOW_WIDTH * scale).round();
        let x = (position.x as f64 + (size.width as f64 - width) / 2.0).round();
        let y = position.y as f64;
        let _ = window.set_position(PhysicalPosition::new(x as i32, y as i32));
        let _ = window.set_size(PhysicalSize::new(
            width as u32,
            (WINDOW_HEIGHT * scale).round() as u32,
        ));
        Placement { x, y, scale }
    };

    let monitor_rect = (
        position.x as f64 / scale,
        position.y as f64 / scale,
        size.width as f64 / scale,
        size.height as f64 / scale,
    );
    let metrics = platform::notch_metrics(&monitor);
    let screen = ScreenInfo {
        platform: std::env::consts::OS.to_string(),
        has_notch: metrics.has_notch,
        notch_width: metrics.notch_width,
        notch_height: metrics.notch_height,
        menu_bar_height: metrics.menu_bar_height,
        display_name: platform::display_label(&monitor).unwrap_or_default(),
        fullscreen: !metrics.has_notch && platform::fullscreen_over(monitor_rect),
    };

    let (hit_width, hit_height) = {
        let geometry = app.state::<Geometry>();
        let inner = geometry.0.lock().unwrap();
        (inner.hit_width, inner.hit_height)
    };
    if hit_width > 0.0 {
        platform::set_hit_region(app, hit_width, hit_height, placement.scale);
    }

    let signature = monitor_signature(&window.available_monitors().unwrap_or_default());
    let changed = {
        let geometry = app.state::<Geometry>();
        let mut inner = geometry.0.lock().unwrap();
        inner.placement = Some(placement);
        inner.monitor_signature = signature;
        inner.monitor_rect = Some(monitor_rect);
        let changed = inner.screen != screen;
        inner.screen = screen.clone();
        changed
    };
    if changed {
        let _ = app.emit("bangs://screen", &screen);
    }
}

/// Records the notch's current size and, on Windows, clips the window to it.
pub fn set_hit_rect(app: &AppHandle, width: f64, height: f64) {
    let scale = app.state::<Geometry>().store_hit_rect(width, height);
    platform::set_hit_region(app, width, height, scale);
}

/// Re-attaches the notch when displays are added, removed or rearranged.
pub fn spawn_display_watcher(app: AppHandle) {
    thread::spawn(move || loop {
        thread::sleep(DISPLAY_POLL);
        let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
            continue;
        };
        let signature = monitor_signature(&window.available_monitors().unwrap_or_default());
        let changed = app.state::<Geometry>().0.lock().unwrap().monitor_signature != signature;
        if changed {
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || {
                place_window(&handle);
                crate::tray::refresh(&handle);
            });
        }
        // Full-screen video, games and presentations: macOS shrinks the notch
        // to its idle bar, Windows steps out of the way altogether.
        if let Some(rect) = app.state::<Geometry>().monitor_rect() {
            let fullscreen = platform::fullscreen_over(rect);
            if let Some(screen) = app.state::<Geometry>().set_fullscreen(fullscreen) {
                let _ = app.emit("bangs://screen", &screen);
            }
        }
        #[cfg(windows)]
        platform::sync_fullscreen_visibility(&app);
    });
}

/// Polls the global cursor to drive hover and click-through. Polling works the
/// same on both platforms, needs no accessibility permission, and keeps
/// working while the window ignores mouse events.
///
/// It also streams the pointer position while inside: WKWebView ignores mouse
/// moves in a panel that is not key, so the webview derives hover from this.
pub fn spawn_cursor_tracker(app: AppHandle) {
    thread::spawn(move || {
        // macOS clips input by the panel's alpha once this is on; Windows uses
        // a window region instead (see platform::set_hit_region).
        #[cfg(target_os = "macos")]
        let window = {
            let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
                return;
            };
            let _ = window.set_ignore_cursor_events(true);
            window
        };
        let mut inside = false;
        let mut was_down = false;
        let mut last_pointer = None;
        // A press that began outside the notch and is still held is how a drag
        // from another window looks from here; only Windows needs to know.
        #[cfg(windows)]
        let mut press_began_outside = false;

        loop {
            thread::sleep(CURSOR_POLL);
            let Some(cursor) = platform::cursor_position() else {
                continue;
            };

            let pointer = app.state::<Geometry>().locate(cursor);
            if pointer.is_some() != inside {
                inside = pointer.is_some();
                #[cfg(target_os = "macos")]
                let _ = window.set_ignore_cursor_events(!inside);
                let _ = app.emit("bangs://hover", inside);
            }
            if let Some((x, y)) = pointer.filter(|_| pointer != last_pointer) {
                let _ = app.emit("bangs://pointer", [x, y]);
            }
            last_pointer = pointer;

            let down = platform::mouse_button_down();
            if down && !was_down {
                #[cfg(windows)]
                {
                    press_began_outside = !inside;
                }
                if !inside {
                    let _ = app.emit("bangs://outside-click", ());
                }
            }
            // WKWebView now and then loses a release; a tab being dragged
            // along the bar listens here so it never stays stuck to the pointer.
            if was_down && !down {
                let _ = app.emit("bangs://release", ());
            }
            #[cfg(windows)]
            if !down {
                press_began_outside = false;
            }
            was_down = down;

            // Windows never hands this window a drop, so it is caught with a
            // window of our own while such a drag is over the notch.
            #[cfg(windows)]
            {
                let dragging_in = down && press_began_outside && inside;
                if let Some(rect) = app.state::<Geometry>().hit_rect_on_screen() {
                    platform::set_catching(&app, dragging_in, rect);
                } else {
                    platform::set_catching(&app, false, (0, 0, 0, 0));
                }
            }
        }
    });
}
