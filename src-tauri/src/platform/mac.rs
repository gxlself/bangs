use std::ffi::c_void;
use std::ptr;

use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSCursor, NSRunningApplication, NSScreen};
use objc2_foundation::{ns_string, NSLocale, NSString};
use tauri::{AppHandle, Manager, Monitor};
use tauri_nspanel::{CollectionBehavior, ManagerExt, PanelLevel, StyleMask, WebviewWindowExt};

use super::NotchMetrics;
use crate::MAIN_WINDOW;

/// Kept in its own module because `tauri_panel!` expands to a set of `use`
/// items that would otherwise clash with this file's imports.
mod panel {
    // The expansion calls `WebviewWindow::app_handle`.
    use tauri::Manager;

    tauri_nspanel::tauri_panel! {
        panel!(NotchPanel {
            config: {
                // Only ever to type in the to-do field, and only when asked:
                // `becomes_key_only_if_needed` keeps a click on the notch from
                // taking the key window away from whatever is in front.
                can_become_key_window: true,
                can_become_main_window: false,
                is_floating_panel: true,
                hides_on_deactivate: false
            }
        })
    }
}
use panel::NotchPanel;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

const COMBINED_SESSION_STATE: i32 = 0;
const LEFT_BUTTON: u32 = 0;
const RIGHT_BUTTON: u32 = 1;

const WINDOWS_ON_SCREEN: u32 = 1 << 0;
const WINDOWS_EXCLUDE_DESKTOP: u32 = 1 << 4;
const CF_NUMBER_DOUBLE: i32 = 13;
/// How far a window may miss the display's edges and still count as covering
/// it; some players are a pixel off.
const COVER_SLACK: f64 = 2.0;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventCreate(source: *const c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *const c_void) -> CGPoint;
    fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(object: *const c_void);
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    fn CFDictionaryGetValue(dictionary: *const c_void, key: *const c_void) -> *const c_void;
    fn CFNumberGetValue(number: *const c_void, kind: i32, value: *mut c_void) -> bool;
}

/// Turns the Tauri window into a non-activating NSPanel above the menu bar.
pub fn prepare_window(app: &AppHandle) -> tauri::Result<()> {
    let window = app
        .get_webview_window(MAIN_WINDOW)
        .expect("main window is declared in tauri.conf.json");
    let panel = window.to_panel::<NotchPanel>()?;
    // The menu bar lives at MainMenu (24); Status draws on top of it.
    panel.set_level(PanelLevel::Status.value());
    // Clicking the notch must not steal focus from the frontmost app.
    panel.set_style_mask(StyleMask::empty().nonactivating_panel().into());
    panel.set_collection_behavior(
        CollectionBehavior::new()
            .can_join_all_spaces()
            .stationary()
            .full_screen_auxiliary()
            .ignores_cycle()
            .into(),
    );
    panel.set_has_shadow(false);
    panel.set_becomes_key_only_if_needed(true);
    Ok(())
}

/// Takes keyboard input, or hands it back. The panel is non-activating, so the
/// app in front stays active either way; while this is on it loses the key
/// window, which is the price of a caret in the to-do field.
pub fn set_keyboard_capture(app: &AppHandle, capture: bool) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Ok(panel) = handle.get_webview_panel(MAIN_WINDOW) else { return };
        if capture {
            panel.make_key_window();
        } else {
            panel.resign_key_window();
        }
    });
}

pub fn set_window_visible(app: &AppHandle, visible: bool) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Ok(panel) = handle.get_webview_panel(MAIN_WINDOW) {
            if visible {
                panel.show();
            } else {
                panel.hide();
            }
        }
    });
}

/// macOS clips through the panel's transparent pixels on its own.
pub fn set_hit_region(_app: &AppHandle, _width: f64, _height: f64, _scale: f64) {}

/// Sets the mouse cursor. The panel never becomes key, so WKWebView is never
/// asked to update the cursor itself and the webview tells us what it wants
/// (see src/lib/hover.ts). Setting it sticks until the pointer moves over
/// another app's window, which sets its own.
pub fn set_cursor(app: &AppHandle, shape: &str) {
    let shape = shape.to_string();
    let _ = app.run_on_main_thread(move || {
        let cursor = match shape.as_str() {
            "pointer" => NSCursor::pointingHandCursor(),
            "grab" => NSCursor::openHandCursor(),
            "grabbing" => NSCursor::closedHandCursor(),
            "text" => NSCursor::IBeamCursor(),
            _ => NSCursor::arrowCursor(),
        };
        cursor.set();
    });
}

/// The language the system prefers, as a tag like "zh-Hans-CN" or "en-GB".
pub fn system_language() -> String {
    NSLocale::preferredLanguages()
        .iter()
        .next()
        .map(|language| language.to_string())
        .unwrap_or_default()
}

/// Whether an app with this bundle identifier is running right now. Asking
/// before talking to it keeps AppleScript from launching it.
pub fn app_is_running(bundle_id: &str) -> bool {
    let id = NSString::from_str(bundle_id);
    !NSRunningApplication::runningApplicationsWithBundleIdentifier(&id).is_empty()
}

/// Global cursor position in points, top-left origin of the main display.
pub fn cursor_position() -> Option<(f64, f64)> {
    unsafe {
        let event = CGEventCreate(ptr::null());
        if event.is_null() {
            return None;
        }
        let point = CGEventGetLocation(event);
        CFRelease(event);
        Some((point.x, point.y))
    }
}

pub fn mouse_button_down() -> bool {
    unsafe {
        CGEventSourceButtonState(COMBINED_SESSION_STATE, LEFT_BUTTON)
            || CGEventSourceButtonState(COMBINED_SESSION_STATE, RIGHT_BUTTON)
    }
}

/// The NSScreen showing `monitor`, matched by its frame.
fn matching_screen(mtm: MainThreadMarker, monitor: &Monitor) -> Option<Retained<NSScreen>> {
    let scale = monitor.scale_factor();
    let x = monitor.position().x as f64 / scale;
    let width = monitor.size().width as f64 / scale;
    let height = monitor.size().height as f64 / scale;
    NSScreen::screens(mtm).into_iter().find(|screen| {
        let frame = screen.frame();
        (frame.origin.x - x).abs() <= 1.0
            && (frame.size.width - width).abs() <= 1.0
            && (frame.size.height - height).abs() <= 1.0
    })
}

/// Folds traditional characters to simplified ones, so a title that reads
/// 當時的月亮 in one catalogue can be compared with 当时的月亮 in another.
/// ICU does the work; text with nothing to fold comes back as it was.
pub fn to_simplified(text: &str) -> String {
    NSString::from_str(text)
        .stringByApplyingTransform_reverse(ns_string!("Hant-Hans"), false)
        .map(|folded| folded.to_string())
        .unwrap_or_else(|| text.to_string())
}

/// Opens a folder in VS Code or Cursor; fails when that editor is missing.
pub fn open_in_editor(editor: &str, path: &str) -> Result<(), String> {
    let app = match editor {
        "code" => "Visual Studio Code",
        "cursor" => "Cursor",
        other => return Err(format!("unknown editor {other}")),
    };
    let status = std::process::Command::new("/usr/bin/open")
        .args(["-a", app, path])
        .status()
        .map_err(|error| error.to_string())?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("{app} could not open {path}"))
}

/// One number out of a CFDictionary the window list handed over.
fn dict_number(dictionary: *const c_void, key: &NSString) -> Option<f64> {
    let value = unsafe { CFDictionaryGetValue(dictionary, (key as *const NSString).cast()) };
    if value.is_null() {
        return None;
    }
    let mut number = 0f64;
    let read = unsafe {
        CFNumberGetValue(value, CF_NUMBER_DOUBLE, (&mut number as *mut f64).cast())
    };
    read.then_some(number)
}

/// True while a window covers the whole of `rect` — full-screen video, a game,
/// a presentation. `rect` is logical, top-left based, the space the window
/// list reports bounds in.
///
/// Only geometry is read. Window bounds are public; their titles are what
/// needs the screen-recording permission, and those are never asked for.
pub fn fullscreen_over(rect: (f64, f64, f64, f64)) -> bool {
    let (left, top, width, height) = rect;
    if width <= 0.0 || height <= 0.0 {
        return false;
    }
    let list = unsafe { CGWindowListCopyWindowInfo(WINDOWS_ON_SCREEN | WINDOWS_EXCLUDE_DESKTOP, 0) };
    if list.is_null() {
        return false;
    }
    let own_pid = f64::from(std::process::id());
    let mut covered = false;
    for index in 0..unsafe { CFArrayGetCount(list) } {
        let window = unsafe { CFArrayGetValueAtIndex(list, index) };
        if window.is_null() {
            continue;
        }
        // Layer 0 is an ordinary application window. The menu bar, the Dock,
        // the wallpaper and this notch itself all sit on other layers.
        if dict_number(window, ns_string!("kCGWindowLayer")) != Some(0.0) {
            continue;
        }
        if dict_number(window, ns_string!("kCGWindowOwnerPID")) == Some(own_pid) {
            continue;
        }
        let bounds = unsafe {
            CFDictionaryGetValue(window, (ns_string!("kCGWindowBounds") as *const NSString).cast())
        };
        if bounds.is_null() {
            continue;
        }
        let (Some(x), Some(y), Some(w), Some(h)) = (
            dict_number(bounds, ns_string!("X")),
            dict_number(bounds, ns_string!("Y")),
            dict_number(bounds, ns_string!("Width")),
            dict_number(bounds, ns_string!("Height")),
        ) else {
            continue;
        };
        if (x - left).abs() <= COVER_SLACK
            && (y - top).abs() <= COVER_SLACK
            && (w - width).abs() <= COVER_SLACK
            && (h - height).abs() <= COVER_SLACK
        {
            covered = true;
            break;
        }
    }
    unsafe { CFRelease(list) };
    covered
}

/// Unique, human readable display name. tao reports "Monitor #<model>" on
/// macOS, which repeats for identical displays.
pub fn display_label(monitor: &Monitor) -> Option<String> {
    let screen = matching_screen(MainThreadMarker::new()?, monitor)?;
    Some(screen.localizedName().to_string())
}

/// Reads notch and menu bar sizes from the NSScreen matching `monitor`.
/// Returns defaults when called off the main thread.
pub fn notch_metrics(monitor: &Monitor) -> NotchMetrics {
    let Some(screen) = MainThreadMarker::new().and_then(|mtm| matching_screen(mtm, monitor)) else {
        return NotchMetrics::default();
    };
    let frame = screen.frame();

    let top_inset = screen.safeAreaInsets().top;
    if top_inset > 0.0 {
        let left = screen.auxiliaryTopLeftArea();
        let right = screen.auxiliaryTopRightArea();
        let notch_width = frame.size.width - left.size.width - right.size.width;
        return NotchMetrics {
            has_notch: true,
            notch_width: if (80.0..400.0).contains(&notch_width) { notch_width } else { 200.0 },
            notch_height: top_inset,
            menu_bar_height: top_inset,
        };
    }

    let visible = screen.visibleFrame();
    let menu_bar = (frame.origin.y + frame.size.height) - (visible.origin.y + visible.size.height);
    NotchMetrics {
        menu_bar_height: if (0.0..60.0).contains(&menu_bar) { menu_bar } else { 0.0 },
        ..NotchMetrics::default()
    }
}
