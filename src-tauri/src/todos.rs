//! The short list of things to do, kept on the notch.
//!
//! It is the one panel that writes: the list lives in `<config>/todos.json`,
//! is loaded at start-up and saved after every change, so it survives a
//! restart without anything else having to know about it. Nothing is kept
//! after it is done — ticking a line off takes it off the list for good.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::sync::{self, Record};

/// The panel is a glance, not a backlog: the oldest lines drop off the end.
const MAX_TODOS: usize = 60;
/// One line of text. Anything longer belongs in a real task list.
const MAX_TEXT: usize = 200;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Todo {
    pub id: String,
    pub text: String,
    /// Unix milliseconds.
    pub created_at: u64,
}

/// The list as the webview sees it, newest first.
#[derive(Default)]
pub struct TodoHub(Mutex<Vec<Todo>>);

impl TodoHub {
    pub fn current(&self) -> Vec<Todo> {
        self.0.lock().unwrap().clone()
    }
}

fn path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|dir| dir.join("todos.json"))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default()
}

/// Reads the saved list. A file that cannot be parsed is left alone rather
/// than overwritten, so a bad edit can still be rescued by hand.
pub fn start(app: AppHandle) {
    let todos: Vec<Todo> = path(&app)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    *app.state::<TodoHub>().0.lock().unwrap() = todos;
}

/// Where a change came from. What the user did here is told to sync; what
/// sync brought in must not be told back to it.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(all(target_os = "macos", feature = "icloud")), allow(dead_code))]
enum Origin {
    Here,
    Sync,
}

/// Applies a change, saves it and tells the webview — and, for a change made
/// here, sync: a line that is new is an upsert, one that is gone is a delete.
///
/// The list is a glance, not a backlog: past `MAX_TODOS` the oldest lines fall
/// off the end. That holds wherever the line that pushed them off came from,
/// and a line that falls off goes on every device, so the phone and the notch
/// keep showing the same list.
fn edit(app: &AppHandle, origin: Origin, change: impl FnOnce(&mut Vec<Todo>)) {
    let (added, removed) = {
        let hub = app.state::<TodoHub>();
        let mut guard = hub.0.lock().unwrap();
        let before = guard.clone();
        change(&mut guard);
        let dropped: Vec<String> =
            if guard.len() > MAX_TODOS { guard.split_off(MAX_TODOS).into_iter().map(|todo| todo.id).collect() } else { Vec::new() };
        let added: Vec<Todo> = match origin {
            Origin::Here => guard.iter().filter(|todo| !before.iter().any(|old| old.id == todo.id)).cloned().collect(),
            Origin::Sync => Vec::new(),
        };
        let mut removed: Vec<String> = match origin {
            // What sync took away is already gone everywhere else.
            Origin::Sync => Vec::new(),
            Origin::Here => before
                .iter()
                .filter(|old| !guard.iter().any(|todo| todo.id == old.id))
                .map(|old| old.id.clone())
                .collect(),
        };
        for id in dropped {
            if !removed.contains(&id) {
                removed.push(id);
            }
        }
        if *guard != before {
            // Still under the lock, so a change racing this one (the phone's,
            // arriving on another thread) cannot write an older list last.
            save(app, &guard);
            let _ = app.emit("bangs://todos", guard.clone());
        }
        (added, removed)
    };

    for todo in &added {
        sync::todo_upsert(app, todo);
    }
    for id in &removed {
        sync::todo_delete(app, id);
    }
}

fn save(app: &AppHandle, todos: &[Todo]) {
    let Some(path) = path(app) else { return };
    let written = path
        .parent()
        .map(fs::create_dir_all)
        .transpose()
        .and_then(|_| fs::write(&path, serde_json::to_vec_pretty(todos).unwrap_or_default()));
    if let Err(error) = written {
        eprintln!("[todos] failed to save to {}: {error}", path.display());
    }
}

/// One line of plain text, as long as a line may be.
fn one_line(text: &str) -> String {
    text.trim().replace(['\n', '\r', '\t'], " ").chars().take(MAX_TEXT).collect()
}

#[tauri::command]
pub fn todo_add(app: AppHandle, text: String) {
    let text = one_line(&text);
    if text.is_empty() {
        return;
    }
    let created_at = now_ms();
    edit(&app, Origin::Here, |todos| {
        todos.insert(0, Todo { id: next_id(created_at), text, created_at });
    });
}

/// Unique for the life of the list: two items added in the same millisecond
/// still get ids of their own.
fn next_id(created_at: u64) -> String {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    format!("{created_at:x}-{:x}", COUNT.fetch_add(1, Ordering::Relaxed))
}

/// Ticked off, or thought better of: either way the line is gone.
#[tauri::command]
pub fn todo_remove(app: AppHandle, id: String) {
    edit(&app, Origin::Here, |todos| todos.retain(|todo| todo.id != id));
}

/// Takes in what came from the phone: lines added or edited there, lines
/// ticked off there.
#[cfg_attr(not(all(target_os = "macos", feature = "icloud")), allow(dead_code))]
pub fn apply_remote(app: &AppHandle, records: Vec<Record>) {
    edit(app, Origin::Sync, |todos| merge_remote(todos, &records));
}

/// Lines the phone deleted that are still here — the app quit before it
/// heard, or sync was off. They go without telling sync again.
pub fn forget(app: &AppHandle, ids: &[String]) {
    edit(app, Origin::Sync, |todos| todos.retain(|todo| !ids.contains(&todo.id)));
}

fn merge_remote(todos: &mut Vec<Todo>, records: &[Record]) {
    for record in records {
        if record.deleted {
            todos.retain(|todo| todo.id != record.id);
            continue;
        }
        let Some(mut incoming) = sync::todo_from_record(record) else { continue };
        incoming.text = one_line(&incoming.text);
        if incoming.text.is_empty() {
            continue;
        }
        match todos.iter_mut().find(|todo| todo.id == incoming.id) {
            Some(existing) => *existing = incoming,
            None => todos.push(incoming),
        }
    }
    // Newest first, whatever order the records arrived in.
    todos.sort_by(|a, b| b.created_at.cmp(&a.created_at));
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn todo(id: &str, text: &str, created_at: u64) -> Todo {
        Todo { id: id.into(), text: text.into(), created_at }
    }

    fn record(id: &str, deleted: bool, body: serde_json::Value) -> Record {
        serde_json::from_value(json!({
            "kind": "todo", "id": id, "updatedAt": 9, "device": "phone", "deleted": deleted, "body": body
        }))
        .unwrap()
    }

    #[test]
    fn a_line_from_the_phone_joins_the_list_newest_first() {
        let mut todos = vec![todo("a", "old", 10), todo("b", "newer", 30)];
        merge_remote(&mut todos, &[record("c", false, json!({ "text": "from phone", "createdAt": 20 }))]);
        let ids: Vec<&str> = todos.iter().map(|todo| todo.id.as_str()).collect();
        assert_eq!(ids, ["b", "c", "a"]);
    }

    #[test]
    fn an_edit_from_the_phone_replaces_the_line() {
        let mut todos = vec![todo("a", "before", 10)];
        merge_remote(&mut todos, &[record("a", false, json!({ "text": "after", "createdAt": 10 }))]);
        assert_eq!(todos, vec![todo("a", "after", 10)]);
    }

    #[test]
    fn a_line_ticked_off_on_the_phone_goes() {
        let mut todos = vec![todo("a", "x", 10), todo("b", "y", 20)];
        merge_remote(&mut todos, &[record("a", true, json!({}))]);
        assert_eq!(todos, vec![todo("b", "y", 20)]);
    }

    #[test]
    fn what_the_phone_sends_is_made_into_one_short_line() {
        let mut todos = Vec::new();
        let long = format!("first\nsecond\t{}", "x".repeat(300));
        merge_remote(
            &mut todos,
            &[
                record("a", false, json!({ "text": long, "createdAt": 1 })),
                record("b", false, json!({ "text": "   \n ", "createdAt": 2 })),
                record("c", false, json!({ "createdAt": 3 })),
            ],
        );
        assert_eq!(todos.len(), 1, "empty and malformed lines are dropped");
        assert_eq!(todos[0].text.chars().count(), MAX_TEXT);
        assert!(todos[0].text.starts_with("first second "));
    }
}
