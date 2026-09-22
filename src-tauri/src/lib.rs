mod activities;
mod clipboard;
mod dev;
mod geometry;
mod i18n;
mod lyrics;
mod media;
mod platform;
mod settings;
mod shelf;
mod todos;
mod tray;
mod update;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use activities::{Activity, ActivityHub};
use dev::{DevHub, DevState};
use geometry::{Geometry, ScreenInfo};
use lyrics::{Lyrics, LyricsHub};
use media::{MediaCommand, MediaHub, MediaState};
use clipboard::{ClipboardHub, ClipboardState};
use settings::{Settings, SettingsState};
use todos::{Todo, TodoHub};
use update::UpdateState;

pub const MAIN_WINDOW: &str = "main";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Bootstrap {
    screen: ScreenInfo,
    settings: Settings,
    media: Option<MediaState>,
    lyrics: Lyrics,
    dev: DevState,
    clipboard: ClipboardState,
    /// Rows other programs asked the notch to show; see docs/plugins.md.
    activities: Vec<Activity>,
    todos: Vec<Todo>,
    drag_icon: Option<String>,
    /// "zh" or "en", resolved from the setting or the system.
    language: &'static str,
}

/// Everything the webview needs for its first render. Events that fire
/// before the webview subscribes are covered by this snapshot.
#[tauri::command]
fn bootstrap(app: AppHandle) -> Bootstrap {
    Bootstrap {
        screen: app.state::<Geometry>().screen(),
        settings: app.state::<SettingsState>().get(),
        media: app.state::<MediaHub>().current(),
        lyrics: app.state::<LyricsHub>().current(),
        dev: app.state::<DevHub>().current(),
        clipboard: app.state::<ClipboardHub>().current(),
        activities: app.state::<ActivityHub>().current(),
        todos: app.state::<TodoHub>().current(),
        drag_icon: shelf::drag_icon_path(&app),
        language: i18n::code(),
    }
}

/// Called by the webview after its first render so the window never shows
/// an empty frame.
#[tauri::command]
fn notch_ready(app: AppHandle) {
    if app.state::<SettingsState>().get().visible {
        apply_visibility(&app, true);
    }
}

#[tauri::command]
fn set_hit_rect(app: AppHandle, width: f64, height: f64) {
    geometry::set_hit_rect(&app, width, height);
}

#[tauri::command]
fn set_cursor(app: AppHandle, shape: String) {
    platform::set_cursor(&app, &shape);
}

/// Lets the webview type: the notch keeps its hands off the keyboard except
/// while a field in it is focused.
#[tauri::command]
fn capture_keyboard(app: AppHandle, capture: bool) {
    platform::set_keyboard_capture(&app, capture);
}

#[tauri::command]
fn media_command(app: AppHandle, command: MediaCommand) -> Result<(), String> {
    media::command(&app, command)
}

pub fn apply_visibility(app: &AppHandle, visible: bool) {
    app.state::<Geometry>().set_hidden(!visible);
    platform::set_window_visible(app, visible);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_drag::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ));
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());

    builder
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();
            let settings = settings::init(&handle);
            i18n::apply(settings.get().language.as_deref());
            app.manage(settings);
            app.manage(Geometry::default());
            app.manage(MediaHub::default());
            app.manage(LyricsHub::default());
            app.manage(DevHub::default());
            app.manage(ClipboardHub::default());
            app.manage(ActivityHub::default());
            app.manage(TodoHub::default());
            app.manage(UpdateState::default());

            platform::prepare_window(&handle)?;
            geometry::place_window(&handle);
            geometry::spawn_display_watcher(handle.clone());
            geometry::spawn_cursor_tracker(handle.clone());
            media::start(handle.clone());
            lyrics::start(handle.clone());
            dev::start(handle.clone());
            clipboard::start(handle.clone());
            activities::start(handle.clone());
            todos::start(handle.clone());
            tray::create(&handle)?;
            update::start(handle.clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            notch_ready,
            set_hit_rect,
            set_cursor,
            capture_keyboard,
            settings::set_tab_order,
            media_command,
            lyrics::lyrics_refresh,
            dev::open_project,
            activities::activity_open,
            clipboard::clipboard_use,
            clipboard::clipboard_open,
            clipboard::clipboard_clear,
            clipboard::clipboard_more,
            clipboard::clipboard_install,
            todos::todo_add,
            todos::todo_remove,
            shelf::shelf_inspect,
            shelf::open_file,
            shelf::reveal_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
