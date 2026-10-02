//! The short list of things to do, kept on the notch.
//!
//! It is the one panel that writes: the list lives in `<config>/todos.json`,
//! is loaded at start-up and saved after every change, so it survives a
//! restart without anything else having to know about it. Ticking a line off
//! marks it done (and ticking it again brings it back); deleting it — one at a
//! time, or every done line at once — is what takes it off the list.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::sync::{self, Record};

/// The panel is a glance, not a backlog: past this many lines the ones finished
/// longest ago drop off the end, and with this many still open the list takes
/// no more until some are done.
pub const MAX_TODOS: usize = 60;
/// One line of text. Anything longer belongs in a real task list.
const MAX_TEXT: usize = 200;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Todo {
    pub id: String,
    pub text: String,
    /// Unix milliseconds.
    pub created_at: u64,
    /// Ticked off. Lists saved before there was such a thing read as not done.
    #[serde(default)]
    pub done: bool,
    /// When it was ticked off, Unix milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_at: Option<u64>,
}

impl Todo {
    pub fn new(id: String, text: String, created_at: u64) -> Self {
        Self { id, text, created_at, done: false, done_at: None }
    }
}

/// The order the list is always kept in: what is still to do, newest first,
/// then what is done, most recently done first. The cap cuts from the end, so
/// the first lines to go are the ones finished longest ago.
fn sort(todos: &mut [Todo]) {
    todos.sort_by(|a, b| {
        a.done.cmp(&b.done).then_with(|| {
            if a.done {
                b.done_at.unwrap_or(b.created_at).cmp(&a.done_at.unwrap_or(a.created_at))
            } else {
                b.created_at.cmp(&a.created_at)
            }
        })
    });
}

/// The list as the webview sees it, in `sort` order.
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
/// here, sync: a line that is new or changed (ticked, unticked) is an upsert,
/// one that is gone is a delete.
///
/// The list is a glance, not a backlog: past `MAX_TODOS` lines fall off the
/// end, which `sort` makes the ones finished longest ago. That holds wherever
/// the line that pushed them off came from, and a line that falls off goes on
/// every device, so the phone and the notch keep showing the same list.
fn edit(app: &AppHandle, origin: Origin, change: impl FnOnce(&mut Vec<Todo>)) {
    let (changed, removed) = {
        let hub = app.state::<TodoHub>();
        let mut guard = hub.0.lock().unwrap();
        let before = guard.clone();
        change(&mut guard);
        sort(&mut guard);
        // Only done lines fall off: an open one is something still to do, and
        // the phone may have added more than the notch would have.
        let keep = MAX_TODOS.max(guard.iter().filter(|todo| !todo.done).count());
        let dropped: Vec<String> =
            if guard.len() > keep { guard.split_off(keep).into_iter().map(|todo| todo.id).collect() } else { Vec::new() };
        let changed: Vec<Todo> = match origin {
            Origin::Here => guard
                .iter()
                .filter(|todo| before.iter().find(|old| old.id == todo.id) != Some(*todo))
                .cloned()
                .collect(),
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
        (changed, removed)
    };

    for todo in &changed {
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
        // Full of open lines: the panel says so instead of taking another.
        if todos.iter().filter(|todo| !todo.done).count() >= MAX_TODOS {
            return;
        }
        todos.insert(0, Todo::new(next_id(created_at), text, created_at));
    });
}

/// Ticks a line off, or brings a done line back.
#[tauri::command]
pub fn todo_toggle(app: AppHandle, id: String) {
    let now = now_ms();
    edit(&app, Origin::Here, |todos| {
        if let Some(todo) = todos.iter_mut().find(|todo| todo.id == id) {
            todo.done = !todo.done;
            todo.done_at = todo.done.then_some(now);
        }
    });
}

/// Every done line, gone at once.
#[tauri::command]
pub fn todo_clear_done(app: AppHandle) {
    edit(&app, Origin::Here, |todos| todos.retain(|todo| !todo.done));
}

/// Unique for the life of the list: two items added in the same millisecond
/// still get ids of their own.
fn next_id(created_at: u64) -> String {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    format!("{created_at:x}-{:x}", COUNT.fetch_add(1, Ordering::Relaxed))
}

/// Deleted: the line is gone, here and on the phone.
#[tauri::command]
pub fn todo_remove(app: AppHandle, id: String) {
    edit(&app, Origin::Here, |todos| todos.retain(|todo| todo.id != id));
}

/// Takes in what came from the phone: lines added, ticked or unticked there,
/// lines deleted there.
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
    // `edit` sorts whatever order the records arrived in.
    sort(todos);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn todo(id: &str, text: &str, created_at: u64) -> Todo {
        Todo::new(id.into(), text.into(), created_at)
    }

    fn done(id: &str, created_at: u64, done_at: u64) -> Todo {
        Todo { done: true, done_at: Some(done_at), ..todo(id, "x", created_at) }
    }

    #[test]
    fn open_lines_come_first_then_the_most_recently_done() {
        let mut todos = vec![done("d-old", 1, 10), todo("a", "x", 5), done("d-new", 2, 20), todo("b", "x", 9)];
        sort(&mut todos);
        let ids: Vec<&str> = todos.iter().map(|todo| todo.id.as_str()).collect();
        assert_eq!(ids, ["b", "a", "d-new", "d-old"]);
    }

    #[test]
    fn a_ticked_line_from_the_phone_arrives_done() {
        let mut todos = vec![todo("a", "milk", 5)];
        merge_remote(&mut todos, &[record("a", false, json!({ "text": "milk", "createdAt": 5, "done": true, "doneAt": 30 }))]);
        assert_eq!(todos, vec![done("a", 5, 30).with_text("milk")]);
    }

    #[test]
    fn lists_saved_before_done_existed_still_load() {
        let old: Vec<Todo> = serde_json::from_str(r#"[{"id":"a","text":"milk","createdAt":5}]"#).unwrap();
        assert_eq!(old, vec![todo("a", "milk", 5)]);
        let written = serde_json::to_value(&old[0]).unwrap();
        assert!(written.get("doneAt").is_none(), "an open line writes no doneAt");
    }

    impl Todo {
        fn with_text(mut self, text: &str) -> Self {
            self.text = text.into();
            self
        }
    }

    fn record(id: &str, deleted: bool, body: serde_json::Value) -> Record {
        serde_json::from_value(json!({
            "kind": "todo", "id": id, "updatedAt": 9, "device": "phone", "deleted": deleted, "body": body
        }))
        .unwrap()
    }

    #[test]
    fn a_line_from_the_phone_joins_the_list_newest_first() {
        let mut todos = vec![todo("b", "newer", 30), todo("a", "old", 10)];
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
    fn a_line_deleted_on_the_phone_goes() {
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
