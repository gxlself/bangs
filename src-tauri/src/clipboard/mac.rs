//! Read-only view of the Paste app's clipboard history (bundle id
//! `gxlself.paste-tool`). Its Core Data store is opened read-only; Paste
//! itself stays the only writer.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

use base64::Engine;
use objc2::rc::Retained;
use objc2::AllocAnyThread;
use objc2_app_kit::{NSImage, NSPasteboard, NSPasteboardTypePNG, NSPasteboardTypeTIFF};
use objc2_foundation::{NSArray, NSData};
use rusqlite::{Connection, OpenFlags};
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;

use super::{publish, ClipItem, ClipKind, ClipSource, ClipboardState};
use crate::i18n::t;

const POLL: Duration = Duration::from_secs(3);
const PREVIEW_CHARS: usize = 180;
/// Core Data stores dates as seconds since 2001-01-01.
const CORE_DATA_EPOCH: f64 = 978_307_200.0;
const PASTE_BUNDLE_ID: &str = "gxlself.paste-tool";
/// Paste's own numbering for what an entry holds.
const KIND_IMAGE: i64 = 1;
const KIND_FILES: i64 = 2;
const DOWNLOAD_PAGE: &str = "https://paste.gxlself.com";

fn store_path() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    [
        home.join("Library/Containers")
            .join(PASTE_BUNDLE_ID)
            .join("Data/Library/Application Support/Paste/PasteTool.sqlite"),
        home.join("Library/Application Support/Paste/PasteTool.sqlite"),
    ]
    .into_iter()
    .find(|path| path.exists())
}

fn connect() -> Option<Connection> {
    let path = store_path()?;
    Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .or_else(|_| {
            // Without access to the -shm file, fall back to the last
            // checkpoint instead of the live WAL.
            Connection::open_with_flags(
                format!("file:{}?immutable=1", path.display()),
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
            )
        })
        .ok()
}

fn read_icons(connection: &Connection) -> HashMap<String, String> {
    let Ok(mut statement) =
        connection.prepare("SELECT ZBUNDLEID, ZICONDATA FROM ZAPPICONENTITY WHERE ZICONDATA IS NOT NULL")
    else {
        return HashMap::new();
    };
    let rows = statement.query_map([], |row| {
        let bundle_id: String = row.get(0)?;
        let icon: Vec<u8> = row.get(1)?;
        Ok((bundle_id, icon))
    });
    let Ok(rows) = rows else { return HashMap::new() };

    rows.flatten()
        .map(|(bundle_id, icon)| {
            let encoded = base64::engine::general_purpose::STANDARD.encode(icon);
            (bundle_id, format!("data:image/png;base64,{encoded}"))
        })
        .collect()
}

fn preview_of(text: Option<String>, kind: ClipKind) -> String {
    let text = text.unwrap_or_default();
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if !collapsed.is_empty() {
        return collapsed.chars().take(PREVIEW_CHARS).collect();
    }
    match kind {
        ClipKind::Image => t("图片", "Image").to_string(),
        ClipKind::Files => t("文件", "Files").to_string(),
        ClipKind::Text => t("空白内容", "Blank").to_string(),
    }
}

fn read_items(
    connection: &Connection,
    icons: &HashMap<String, String>,
    limit: usize,
) -> rusqlite::Result<Vec<ClipItem>> {
    let mut statement = connection.prepare(
        "SELECT Z_PK, ZTYPE, ZISPINNED, ZCREATEDAT, ZAPPBUNDLEID, substr(ZPLAINTEXT, 1, 400)
         FROM ZCLIPBOARDITEMENTITY ORDER BY ZCREATEDAT DESC LIMIT ?1",
    )?;
    let rows = statement.query_map([limit as i64], |row| {
        let kind = match row.get::<_, i64>(1)? {
            1 => ClipKind::Image,
            2 => ClipKind::Files,
            _ => ClipKind::Text,
        };
        let app: Option<String> = row.get(4)?;
        Ok(ClipItem {
            id: row.get(0)?,
            kind,
            pinned: row.get::<_, i64>(2)? != 0,
            created_at: (row.get::<_, f64>(3).unwrap_or_default() + CORE_DATA_EPOCH) * 1000.0,
            icon: app.as_ref().and_then(|bundle_id| icons.get(bundle_id).cloned()),
            app,
            preview: preview_of(row.get(5)?, kind),
        })
    })?;
    rows.collect()
}

pub fn start(app: AppHandle) {
    thread::spawn(move || {
        let mut connection = None;
        let mut icons = HashMap::new();
        loop {
            if connection.is_none() {
                connection = connect();
                if let Some(connection) = &connection {
                    icons = read_icons(connection);
                }
            }

            let limit = app.state::<super::ClipboardHub>().limit();
            match connection.as_ref().map(|db| read_items(db, &icons, limit)) {
                Some(Ok(items)) => {
                    publish(&app, ClipboardState { available: true, source: ClipSource::Paste, items });
                }
                Some(Err(error)) => {
                    // Keep showing what was read last time and try again; a
                    // hiccup is not the same as Paste being missing.
                    eprintln!("[clipboard] read failed: {error}");
                    connection = None;
                }
                // Only an absent store means Paste is not installed.
                None if store_path().is_none() => publish(&app, ClipboardState::default()),
                None => {}
            }
            thread::sleep(POLL);
        }
    });
}

/// Reads the history immediately, for when the panel asks for another page.
pub fn refresh(app: &AppHandle) {
    let Some(connection) = connect() else { return };
    let icons = read_icons(&connection);
    let limit = app.state::<super::ClipboardHub>().limit();
    if let Ok(items) = read_items(&connection, &icons, limit) {
        publish(app, ClipboardState { available: true, source: ClipSource::Paste, items });
    }
}

/// The whole text of the text entries among `ids`, for sync.
pub fn full_texts(ids: &[i64]) -> HashMap<i64, String> {
    let Some(connection) = connect() else { return HashMap::new() };
    ids.iter()
        .filter_map(|id| {
            let text: Option<String> = connection
                .query_row(
                    "SELECT ZPLAINTEXT FROM ZCLIPBOARDITEMENTENTITY WHERE Z_PK = ?1 AND ZTYPE NOT IN (?2, ?3)",
                    rusqlite::params![id, KIND_IMAGE, KIND_FILES],
                    |row| row.get(0),
                )
                .ok()?;
            text.filter(|text| !text.is_empty()).map(|text| (*id, text))
        })
        .collect()
}

/// Puts a history entry back on the clipboard — the picture itself for an
/// image, the files for a file entry, the words for anything else.
pub fn copy(app: AppHandle, id: i64) -> Result<(), String> {
    let connection = connect().ok_or_else(|| t("Paste 数据库不可用", "Paste\u{2019}s database is unavailable"))?;
    let (kind, text, image): (i64, Option<String>, Option<Vec<u8>>) = connection
        .query_row(
            "SELECT ZTYPE, ZPLAINTEXT, ZIMAGEDATA FROM ZCLIPBOARDITEMENTITY WHERE Z_PK = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| error.to_string())?;

    match kind {
        KIND_IMAGE => write_image(&image.unwrap_or_default()),
        // Files are Paste's own: it holds them as promises the pasteboard
        // asks it for, and copying the paths as text is not the same thing.
        KIND_FILES => Err(t("文件交给 Paste 取", "Paste holds the files themselves").to_string()),
        _ => {
            let text = text
                .filter(|text| !text.is_empty())
                .ok_or_else(|| t("这条没有文本内容", "That entry has no text"))?;
            app.clipboard().write_text(text).map_err(|error| error.to_string())
        }
    }
}

/// Puts a picture on the pasteboard under both types apps ask for: the file as
/// Paste stored it, and the representation AppKit draws from.
fn write_image(stored: &[u8]) -> Result<(), String> {
    let (image, data) =
        decode_image(stored).ok_or_else(|| t("这张图片读不出来", "That image could not be read"))?;
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    unsafe {
        let types = NSArray::from_slice(&[NSPasteboardTypePNG, NSPasteboardTypeTIFF]);
        pasteboard.declareTypes_owner(&types, None);
        pasteboard.setData_forType(Some(&data), NSPasteboardTypePNG);
        if let Some(tiff) = image.TIFFRepresentation() {
            pasteboard.setData_forType(Some(&tiff), NSPasteboardTypeTIFF);
        }
    }
    Ok(())
}

/// Paste keeps a byte of its own in front of the file it stored, so the image
/// is read from wherever it actually starts.
fn decode_image(stored: &[u8]) -> Option<(Retained<NSImage>, Retained<NSData>)> {
    for offset in [0usize, 1] {
        let bytes = stored.get(offset..)?;
        let data = NSData::with_bytes(bytes);
        if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
            return Some((image, data));
        }
    }
    None
}

/// Paste registers this scheme for exactly this purpose (see its AppDelegate):
/// it toggles the clipboard panel without changing which app is in front, so
/// whatever the user picks still pastes where they were typing.
const PANEL_URL: &str = "pasteg://panel";

/// Asks Paste to show its own clipboard panel.
pub fn show_panel() -> Result<(), String> {
    let opened = Command::new("/usr/bin/open")
        .arg(PANEL_URL)
        .status()
        .map_err(|error| error.to_string())?;
    if opened.success() {
        return Ok(());
    }

    // Paste builds without the panel URL can only be launched, which is not
    // what was asked for — say so instead of silently doing something else.
    let launched = Command::new("/usr/bin/open")
        .args(["-b", PASTE_BUNDLE_ID])
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    Err(if launched {
        t("这个 Paste 版本还不支持直接弹面板，重新构建安装后即可", "This build of Paste cannot open its panel on request yet").to_string()
    } else {
        t("没找到 Paste", "Paste is not installed").to_string()
    })
}

/// Paste is not installed: send the user to its download page.
pub fn open_download_page() -> Result<(), String> {
    Command::new("/usr/bin/open")
        .arg(DOWNLOAD_PAGE)
        .status()
        .map_err(|error| error.to_string())
        .and_then(|status| status.success().then_some(()).ok_or_else(|| t("无法打开下载页", "Could not open the download page").to_string()))
}

