//! The clipboard panel. On macOS it mirrors the user's own Paste app; on
//! Windows, where there is no Paste, Bangs keeps the history itself.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::i18n::t;
use tauri::{AppHandle, Emitter, Manager};

#[cfg(target_os = "macos")]
mod mac;
#[cfg(windows)]
mod win;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClipKind {
    Text,
    Image,
    Files,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipItem {
    pub id: i64,
    pub kind: ClipKind,
    pub preview: String,
    /// The app the content came from.
    pub app: Option<String>,
    /// That app's icon as a data URL, when the source has one.
    pub icon: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    /// Unix milliseconds.
    pub created_at: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ClipSource {
    /// Read from the Paste app, which owns the history.
    Paste,
    /// Bangs records the history itself.
    Builtin,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardState {
    /// False on macOS when Paste is not installed yet.
    pub available: bool,
    pub source: ClipSource,
    pub items: Vec<ClipItem>,
}

impl Default for ClipboardState {
    fn default() -> Self {
        Self {
            available: false,
            source: if cfg!(target_os = "macos") { ClipSource::Paste } else { ClipSource::Builtin },
            items: Vec::new(),
        }
    }
}

/// How many entries the panel asks for before any scrolling.
pub const PAGE: usize = 24;
/// Scrolling stops loading here; the panel is a shortcut, not an archive.
const MAX_ITEMS: usize = 400;

pub struct ClipboardHub {
    state: Mutex<ClipboardState>,
    /// Grows as the user scrolls the panel.
    limit: Mutex<usize>,
}

impl Default for ClipboardHub {
    fn default() -> Self {
        Self { state: Mutex::new(ClipboardState::default()), limit: Mutex::new(PAGE) }
    }
}

impl ClipboardHub {
    pub fn current(&self) -> ClipboardState {
        self.state.lock().unwrap().clone()
    }

    pub fn limit(&self) -> usize {
        *self.limit.lock().unwrap()
    }
}

fn publish(app: &AppHandle, next: ClipboardState) {
    {
        let hub = app.state::<ClipboardHub>();
        let mut current = hub.state.lock().unwrap();
        if *current == next {
            return;
        }
        *current = next.clone();
    }
    if next.available {
        crate::sync::mirror_clips(app, &next.items);
    }
    let _ = app.emit("bangs://clipboard", next);
}

/// The picture of an image entry, for sync; `None` for anything else, and
/// everywhere but macOS (the Windows history keeps no pictures).
pub fn image_bytes(app: &AppHandle, id: i64) -> Option<Vec<u8>> {
    let _ = app;
    #[cfg(target_os = "macos")]
    return mac::image_bytes(id);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = id;
        None
    }
}

/// The whole text of these text entries, by id, for sync: the panel itself
/// only ever holds a preview. Pictures and files have no entry in the result.
/// `None` when the history cannot be read right now.
pub fn full_texts(app: &AppHandle, ids: &[i64]) -> Option<std::collections::HashMap<i64, String>> {
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        return mac::full_texts(ids);
    }
    #[cfg(windows)]
    return win::full_texts(app, ids);
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (app, ids);
        None
    }
}

pub fn start(app: AppHandle) {
    #[cfg(target_os = "macos")]
    mac::start(app);
    #[cfg(windows)]
    win::start(app);
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = app;
}

/// Puts an entry back on the clipboard.
#[tauri::command]
pub fn clipboard_use(app: AppHandle, id: i64) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return mac::copy(app, id);
    #[cfg(windows)]
    return win::copy(app, id);
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (app, id);
        Err(t("剪贴板在这个平台上不可用", "No clipboard history on this platform").into())
    }
}

/// Loads the next page; the panel calls this when it is scrolled to the end.
/// Returns false once everything is loaded, so the panel stops asking.
#[tauri::command]
pub fn clipboard_more(app: AppHandle) -> bool {
    let hub = app.state::<ClipboardHub>();
    let loaded = hub.current().items.len();
    let limit = hub.limit();
    // Nothing new arrived last time: the history is shorter than the limit.
    if loaded < limit || limit >= MAX_ITEMS {
        return false;
    }
    *hub.limit.lock().unwrap() = (limit + PAGE).min(MAX_ITEMS);
    refresh(&app);
    true
}

/// Re-reads the history right away, instead of waiting for the next poll.
fn refresh(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    mac::refresh(app);
    #[cfg(windows)]
    win::refresh(app);
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = app;
}

/// macOS only: hands over to Paste's own panel, which can paste for you.
#[tauri::command]
pub fn clipboard_open() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return mac::show_panel();
    #[cfg(not(target_os = "macos"))]
    Err(t("这个平台没有外部剪贴板面板", "No external clipboard panel on this platform").into())
}

/// Windows only: Bangs owns that history, so it can drop it.
#[tauri::command]
pub fn clipboard_clear(app: AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    return win::clear(app);
    #[cfg(not(windows))]
    {
        let _ = app;
        Err(t("历史由 Paste 管理，请在 Paste 里清空", "Paste owns this history; clear it there").into())
    }
}

/// macOS only: Paste is missing, so point at where to get it.
#[tauri::command]
pub fn clipboard_install() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return mac::open_download_page();
    #[cfg(not(target_os = "macos"))]
    Err(t("不需要安装", "Nothing to install").into())
}
