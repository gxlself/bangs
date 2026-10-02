//! Windows has no Paste app, so Bangs keeps the clipboard history itself:
//! poll the clipboard sequence number, store what changed, and put entries
//! back when the user picks one. Content that apps mark as private (password
//! managers do) is skipped.

use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System, UpdateKind};
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, GetClipboardOwner, GetClipboardSequenceNumber,
    IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

use super::{publish, ClipItem, ClipKind, ClipSource, ClipboardState};
use crate::i18n::t;
use crate::settings::SettingsState;

const POLL: Duration = Duration::from_millis(600);
const MAX_ITEMS: usize = 200;
/// Anything larger is a document, not something to paste from a notch.
const MAX_TEXT_BYTES: usize = 256 * 1024;
const PREVIEW_CHARS: usize = 180;
const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;

/// The panel only needs a preview; the full payload stays on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredClip {
    item: ClipItem,
    #[serde(default)]
    text: String,
}

fn history_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|dir| dir.join("clipboard.json"))
}

fn load(app: &AppHandle) -> Vec<StoredClip> {
    history_path(app)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save(app: &AppHandle, clips: &[StoredClip]) {
    let Some(path) = history_path(app) else { return };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(raw) = serde_json::to_string(clips) {
        let _ = fs::write(path, raw);
    }
}

fn state_of(clips: &[StoredClip], limit: usize) -> ClipboardState {
    ClipboardState {
        available: true,
        source: ClipSource::Builtin,
        items: clips.iter().take(limit).map(|clip| clip.item.clone()).collect(),
    }
}

/// Re-publishes with the current limit, for when the panel asks for more.
pub fn refresh(app: &AppHandle) {
    let clips = load(app);
    let limit = app.state::<super::ClipboardHub>().limit();
    publish(app, state_of(&clips, limit));
}

pub fn start(app: AppHandle) {
    thread::spawn(move || {
        let mut clips = load(&app);
        publish(&app, state_of(&clips, app.state::<super::ClipboardHub>().limit()));

        let mut system = System::new_with_specifics(
            RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing().with_exe(UpdateKind::Always)),
        );
        let mut seen_sequence = unsafe { GetClipboardSequenceNumber() };

        loop {
            thread::sleep(POLL);
            if !app.state::<SettingsState>().get().clipboard_history {
                continue;
            }

            let sequence = unsafe { GetClipboardSequenceNumber() };
            if sequence == seen_sequence {
                continue;
            }
            seen_sequence = sequence;

            let Some(capture) = read_clipboard(&mut system) else { continue };
            // Re-copying the same thing should not fill the list.
            if clips.first().is_some_and(|first| first.text == capture.text) {
                continue;
            }

            clips.insert(0, capture);
            clips.truncate(MAX_ITEMS);
            save(&app, &clips);
            publish(&app, state_of(&clips, app.state::<super::ClipboardHub>().limit()));
        }
    });
}

pub fn copy(app: AppHandle, id: i64) -> Result<(), String> {
    let clips = load(&app);
    let clip = clips
        .iter()
        .find(|clip| clip.item.id == id)
        .ok_or_else(|| t("这条记录已经不在了", "That entry is gone"))?;
    app.clipboard()
        .write_text(clip.text.clone())
        .map_err(|error| error.to_string())
}

/// The whole text of the text entries among `ids`, for sync.
pub fn full_texts(app: &AppHandle, ids: &[i64]) -> Option<std::collections::HashMap<i64, String>> {
    let texts = load(app)
        .into_iter()
        .filter(|clip| ids.contains(&clip.item.id) && clip.item.kind == ClipKind::Text && !clip.text.is_empty())
        .map(|clip| (clip.item.id, clip.text))
        .collect();
    Some(texts)
}

pub fn clear(app: AppHandle) -> Result<(), String> {
    save(&app, &[]);
    publish(&app, state_of(&[], super::PAGE));
    Ok(())
}

/// RAII around the clipboard, which must not stay open.
struct Clipboard;

impl Clipboard {
    fn open() -> Option<Self> {
        unsafe { OpenClipboard(None) }.ok().map(|()| Self)
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

fn read_clipboard(system: &mut System) -> Option<StoredClip> {
    let _clipboard = Clipboard::open()?;
    if is_private() {
        return None;
    }

    let (kind, text) = if let Some(text) = read_text() {
        (ClipKind::Text, text)
    } else if let Some(paths) = read_files() {
        (ClipKind::Files, paths.join("\n"))
    } else {
        return None;
    };

    if text.trim().is_empty() || text.len() > MAX_TEXT_BYTES {
        return None;
    }

    let app = owner_app(system);
    Some(StoredClip {
        item: ClipItem {
            id: now_ms() as i64,
            kind,
            preview: preview_of(&text),
            app,
            icon: None,
            pinned: false,
            created_at: now_ms(),
        },
        text,
    })
}

/// Password managers ask not to be recorded; honour that.
fn is_private() -> bool {
    let excluded = format_available("ExcludeClipboardContentFromMonitorProcessing");
    let history = format_id("CanIncludeInClipboardHistory")
        .map(|format| unsafe { IsClipboardFormatAvailable(format) }.is_ok())
        .unwrap_or(false);
    excluded || (history && read_flag("CanIncludeInClipboardHistory") == Some(0))
}

fn format_id(name: &str) -> Option<u32> {
    let name = HSTRING::from(name);
    let format = unsafe { RegisterClipboardFormatW(PCWSTR(name.as_ptr())) };
    (format != 0).then_some(format)
}

fn format_available(name: &str) -> bool {
    format_id(name)
        .map(|format| unsafe { IsClipboardFormatAvailable(format) }.is_ok())
        .unwrap_or(false)
}

fn read_flag(name: &str) -> Option<u32> {
    let format = format_id(name)?;
    let handle = unsafe { GetClipboardData(format) }.ok()?;
    let memory = HGLOBAL(handle.0);
    let pointer = unsafe { GlobalLock(memory) } as *const u32;
    if pointer.is_null() {
        return None;
    }
    let value = unsafe { *pointer };
    let _ = unsafe { GlobalUnlock(memory) };
    Some(value)
}

fn read_text() -> Option<String> {
    if unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) }.is_err() {
        return None;
    }
    let handle = unsafe { GetClipboardData(CF_UNICODETEXT) }.ok()?;
    let memory = HGLOBAL(handle.0);
    let pointer = unsafe { GlobalLock(memory) } as *const u16;
    if pointer.is_null() {
        return None;
    }
    let units = unsafe { GlobalSize(memory) } / 2;
    let slice = unsafe { std::slice::from_raw_parts(pointer, units) };
    let end = slice.iter().position(|unit| *unit == 0).unwrap_or(slice.len());
    let text = String::from_utf16_lossy(&slice[..end]);
    let _ = unsafe { GlobalUnlock(memory) };
    Some(text)
}

fn read_files() -> Option<Vec<String>> {
    if unsafe { IsClipboardFormatAvailable(CF_HDROP) }.is_err() {
        return None;
    }
    let handle = unsafe { GetClipboardData(CF_HDROP) }.ok()?;
    let drop = HDROP(handle.0);
    let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };

    let mut paths = Vec::new();
    for index in 0..count {
        let length = unsafe { DragQueryFileW(drop, index, None) } as usize;
        if length == 0 {
            continue;
        }
        let mut buffer = vec![0u16; length + 1];
        let written = unsafe { DragQueryFileW(drop, index, Some(&mut buffer)) } as usize;
        paths.push(String::from_utf16_lossy(&buffer[..written]));
    }
    (!paths.is_empty()).then_some(paths)
}

/// The app that owns the clipboard right now is the one that copied.
fn owner_app(system: &mut System) -> Option<String> {
    let owner: HWND = unsafe { GetClipboardOwner() }.ok()?;
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(owner, Some(&mut pid)) };
    if pid == 0 {
        return None;
    }
    system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
    let name = system.process(Pid::from_u32(pid))?.name().to_string_lossy().into_owned();
    Some(name.trim_end_matches(".exe").to_string())
}

fn preview_of(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(PREVIEW_CHARS).collect()
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs_f64() * 1000.0)
        .unwrap_or_default()
}
