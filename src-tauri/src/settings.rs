use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Whether the notch window is shown at all.
    pub visible: bool,
    pub expand_on_hover: bool,
    /// On screens without a hardware notch, shrink to a thin bar when idle.
    pub idle_handle: bool,
    /// Pop the notch open when a Claude Code session stops working.
    pub notify_claude_idle: bool,
    /// Look up lyrics for the current track (sends title and artist to QQ Music).
    pub lyrics_enabled: bool,
    /// Show matched lyric translations when the provider returns them.
    pub lyrics_translation_enabled: bool,
    /// Windows only: record the clipboard into the panel's history.
    pub clipboard_history: bool,
    /// `platform::display_label` of the monitor to attach to; `None` follows
    /// the primary monitor.
    pub display: Option<String>,
    /// "zh" or "en"; `None` follows the system language.
    pub language: Option<String>,
    /// The panel's tabs in the order the user dragged them into. Ids the
    /// webview does not know are ignored, and tabs missing here keep their
    /// default place after the ones that are listed.
    pub tab_order: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            visible: true,
            expand_on_hover: true,
            // A resting black pill is intrusive wherever it is drawn rather
            // than hiding in a cutout, which is every screen without a notch —
            // the layout ignores this on the screens that have one.
            idle_handle: true,
            notify_claude_idle: true,
            lyrics_enabled: true,
            lyrics_translation_enabled: true,
            clipboard_history: true,
            display: None,
            language: None,
            tab_order: Vec::new(),
        }
    }
}

pub struct SettingsState(Mutex<Settings>);

impl SettingsState {
    pub fn get(&self) -> Settings {
        self.0.lock().unwrap().clone()
    }
}

fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("settings.json"))
}

pub fn init(app: &AppHandle) -> SettingsState {
    let settings = settings_path(app)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    SettingsState(Mutex::new(settings))
}

/// Most ids a tab order keeps; far above the tabs there are, and it stops a
/// confused caller from growing the file without end.
const MAX_TABS: usize = 32;

/// Remembers the order the tabs were dragged into.
#[tauri::command]
pub fn set_tab_order(app: AppHandle, order: Vec<String>) {
    let mut order: Vec<String> = order.into_iter().filter(|id| !id.is_empty() && id.len() <= 32).collect();
    let mut seen = std::collections::HashSet::new();
    order.retain(|id| seen.insert(id.clone()));
    order.truncate(MAX_TABS);
    if app.state::<SettingsState>().get().tab_order == order {
        return;
    }
    update(&app, |settings| settings.tab_order = order);
}

/// Applies `change`, persists the result and notifies the webview.
/// Returns the previous and the new settings.
pub fn update(app: &AppHandle, change: impl FnOnce(&mut Settings)) -> (Settings, Settings) {
    let state = app.state::<SettingsState>();
    let (previous, next) = {
        let mut guard = state.0.lock().unwrap();
        let previous = guard.clone();
        change(&mut guard);
        (previous, guard.clone())
    };

    if let Some(path) = settings_path(app) {
        let written = path
            .parent()
            .map(fs::create_dir_all)
            .transpose()
            .and_then(|_| fs::write(&path, serde_json::to_vec_pretty(&next).unwrap_or_default()));
        if let Err(error) = written {
            eprintln!("[settings] failed to save to {}: {error}", path.display());
        }
    }
    let _ = app.emit("settings://changed", &next);
    (previous, next)
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn old_settings_keep_the_default_tab_order() {
        let settings: Settings = serde_json::from_str(r#"{"lyricsEnabled":true}"#).unwrap();
        assert!(settings.tab_order.is_empty());
    }

    #[test]
    fn old_settings_enable_lyric_translations_by_default() {
        let settings: Settings = serde_json::from_str(r#"{"lyricsEnabled":true}"#).unwrap();
        assert!(settings.lyrics_translation_enabled);
    }
}
