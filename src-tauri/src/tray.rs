use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_autostart::ManagerExt as _;

use crate::i18n::t;
use crate::settings::{self, SettingsState};
use crate::sync;
use crate::update;
use crate::{geometry, platform, MAIN_WINDOW};

const TRAY_ID: &str = "bangs-tray";
const DISPLAY_PREFIX: &str = "display:";
const LANGUAGE_PREFIX: &str = "language:";

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let settings = app.state::<SettingsState>().get();
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
    let sync = sync::status(app);
    let autostart = app.autolaunch().is_enabled().unwrap_or(false);
    let monitors = app
        .get_webview_window(MAIN_WINDOW)
        .and_then(|window| window.available_monitors().ok())
        .unwrap_or_default();

    let display = Submenu::with_id(app, "display", t("显示器", "Display"), true)?;
    display.append(&CheckMenuItem::with_id(
        app,
        DISPLAY_PREFIX,
        t("跟随主显示器", "Follow the main display"),
        true,
        settings.display.is_none(),
        None::<&str>,
    )?)?;
    for monitor in &monitors {
        let Some(label) = platform::display_label(monitor) else { continue };
        display.append(&CheckMenuItem::with_id(
            app,
            format!("{DISPLAY_PREFIX}{label}"),
            &label,
            true,
            settings.display.as_ref() == Some(&label),
            None::<&str>,
        )?)?;
    }

    let language = Submenu::with_id(app, "language", t("语言", "Language"), true)?;
    for (value, label) in [
        ("", t("跟随系统", "Follow the system")),
        ("zh", "中文"),
        ("en", "English"),
    ] {
        language.append(&CheckMenuItem::with_id(
            app,
            format!("{LANGUAGE_PREFIX}{value}"),
            label,
            true,
            settings.language.as_deref().unwrap_or("") == value,
            None::<&str>,
        )?)?;
    }

    Menu::with_items(
        app,
        &[
            &CheckMenuItem::with_id(app, "visible", t("显示刘海", "Show the notch"), true, settings.visible, None::<&str>)?,
            &CheckMenuItem::with_id(app, "hover", t("悬停时展开", "Expand on hover"), true, settings.expand_on_hover, None::<&str>)?,
            &CheckMenuItem::with_id(app, "idle-handle", t("空闲时收成细条", "Shrink to a bar when idle"), true, settings.idle_handle, None::<&str>)?,
            #[cfg(windows)]
            &CheckMenuItem::with_id(app, "clipboard-history", t("记录剪贴板", "Record the clipboard"), true, settings.clipboard_history, None::<&str>)?,
            #[cfg(target_os = "macos")]
            &CheckMenuItem::with_id(app, "icloud-sync", t("同步到 iPhone（iCloud）", "Sync with iPhone (iCloud)"), sync.supported, settings.icloud_sync && sync.supported, None::<&str>)?,
            #[cfg(target_os = "macos")]
            &MenuItem::with_id(app, "icloud-status", format!("    {}", sync::status_label(&sync)), false, None::<&str>)?,
            &CheckMenuItem::with_id(app, "lyrics", t("显示歌词", "Show lyrics"), true, settings.lyrics_enabled, None::<&str>)?,
            &CheckMenuItem::with_id(app, "lyrics-translation", t("显示歌词翻译", "Show lyric translations"), settings.lyrics_enabled, settings.lyrics_translation_enabled, None::<&str>)?,
            &CheckMenuItem::with_id(app, "notify-claude", t("Claude 忙完时提醒", "Alert when Claude finishes"), true, settings.notify_claude_idle, None::<&str>)?,
            &display,
            &language,
            &PredefinedMenuItem::separator(app)?,
            &CheckMenuItem::with_id(app, "autostart", t("开机启动", "Launch at login"), true, autostart, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "version", update::menu_label(app), true, None::<&str>)?,
            &MenuItem::with_id(app, "quit", t("退出", "Quit"), true, None::<&str>)?,
        ],
    )
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Bangs")
        .menu(&build_menu(app)?)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| handle_menu(app, event.id().as_ref()));

    #[cfg(target_os = "macos")]
    let builder = builder
        .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/tray-template.png"))?)
        .icon_as_template(true);
    #[cfg(not(target_os = "macos"))]
    let builder = match app.default_window_icon() {
        Some(icon) => builder.icon(icon.clone().to_owned()),
        None => builder,
    };

    builder.build(app)?;
    Ok(())
}

/// Rebuilds the menu so check marks and the display list stay current.
///
/// Always on the main thread. The monitors are named by AppKit, which answers
/// nowhere else, so a menu built on the version-check thread or the display
/// watcher came back with an empty display list and replaced the good one.
pub fn refresh(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let (Some(tray), Ok(menu)) = (handle.tray_by_id(TRAY_ID), build_menu(&handle)) {
            let _ = tray.set_menu(Some(menu));
        }
    });
}

fn handle_menu(app: &AppHandle, id: &str) {
    match id {
        "quit" => app.exit(0),
        "version" => {
            update::open_releases(app);
            // The menu may have been sitting open since the last check.
            let handle = app.clone();
            std::thread::spawn(move || update::refresh(&handle));
        }
        "visible" => {
            let (_, next) = settings::update(app, |settings| settings.visible = !settings.visible);
            crate::apply_visibility(app, next.visible);
        }
        "hover" => {
            settings::update(app, |settings| settings.expand_on_hover = !settings.expand_on_hover);
        }
        "icloud-sync" => {
            let on = !app.state::<SettingsState>().get().icloud_sync;
            let handle = app.clone();
            // Turning it on reads every panel's data and talks to iCloud; the
            // menu callback is no place for that.
            std::thread::spawn(move || sync::set_enabled(&handle, on));
        }
        "clipboard-history" => {
            settings::update(app, |settings| {
                settings.clipboard_history = !settings.clipboard_history;
            });
        }
        "lyrics" => {
            let (_, next) = settings::update(app, |settings| {
                settings.lyrics_enabled = !settings.lyrics_enabled;
            });
            crate::lyrics::set_enabled(app, next.lyrics_enabled);
        }
        "lyrics-translation" => {
            settings::update(app, |settings| {
                settings.lyrics_translation_enabled = !settings.lyrics_translation_enabled;
            });
        }
        "notify-claude" => {
            settings::update(app, |settings| {
                settings.notify_claude_idle = !settings.notify_claude_idle;
            });
        }
        "idle-handle" => {
            settings::update(app, |settings| settings.idle_handle = !settings.idle_handle);
        }
        "autostart" => {
            let launcher = app.autolaunch();
            let result = if launcher.is_enabled().unwrap_or(false) {
                launcher.disable()
            } else {
                launcher.enable()
            };
            if let Err(error) = result {
                eprintln!("[tray] failed to toggle autostart: {error}");
            }
        }
        other if other.starts_with(LANGUAGE_PREFIX) => {
            let choice = other.trim_start_matches(LANGUAGE_PREFIX).to_string();
            let (_, next) = settings::update(app, |settings| {
                settings.language = (!choice.is_empty()).then(|| choice.clone());
            });
            crate::i18n::apply(next.language.as_deref());
            let _ = tauri::Emitter::emit(app, "bangs://language", crate::i18n::code());
        }
        other => {
            let Some(name) = other.strip_prefix(DISPLAY_PREFIX) else { return };
            settings::update(app, |settings| {
                settings.display = (!name.is_empty()).then(|| name.to_string());
            });
            geometry::place_window(app);
        }
    }
    refresh(app);
}
