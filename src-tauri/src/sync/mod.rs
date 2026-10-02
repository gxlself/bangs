//! Sync with iPhone and iPad through iCloud (CloudKit). The contract — what is
//! synced, the record format, the merge rules — is docs/sync.md.
//!
//! The shape of it: the panels keep their own data and tell this module when
//! something changes here (`todo_upsert`, `mirror_sessions`, …). The ledger
//! turns that into records to send and decides what is newer when records come
//! back; a thread flushes the queue about once a second, which also batches
//! the bursts. Everything is a no-op until the user turns sync on in the tray
//! menu, and nothing happens at all on a build without the iCloud entitlements.

// Without the `icloud` feature on macOS nothing calls what the library would
// answer through, and that is meant.
#![cfg_attr(not(all(target_os = "macos", feature = "icloud")), allow(dead_code))]

mod cloud;
mod ledger;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::clipboard::{ClipItem, ClipKind};
use crate::dev::AgentSession;
use crate::i18n::t;
use crate::settings::{self, SettingsState};
use crate::todos::Todo;
use ledger::{clean_id, fnv_hex, Desired, Ledger, CLIP, DEVICE, SESSION, SHELF, TODO};
pub use ledger::Record;

/// How many clipboard entries the phone gets: the panel's first page, which is
/// what the clipboard poll reads whether or not anyone scrolls.
const CLIP_WINDOW: usize = crate::clipboard::PAGE;
/// The longest clipboard text that is sent whole.
const CLIP_CHARS: usize = 8_000;
/// A copied picture goes to the phone as a JPEG of at most this many bytes. It
/// travels inside the record's end-to-end encrypted values, which share the
/// record's 1 MB limit; one that cannot be made this small stays on the Mac.
const CLIP_IMAGE_BYTES: u64 = 900_000;
/// Bigger files stay on the Mac; the phone still sees that they exist.
const SHELF_FILE_BYTES: u64 = 25_000_000;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;
/// The library's change token in the sync directory (CloudEngine.tokenFileName).
const TOKEN_FILE: &str = "token.bin";
/// Seconds between pulls; there is no push notification on this side, so this
/// is how long a change made on the phone can take to show up here (the tray's
/// "Sync now" does not wait). A zone-changes fetch is cheap; CloudKit allows
/// far more than one every 20 s.
const PULL_EVERY: u64 = 20;
/// Seconds between attempts to reconnect while there is no usable account:
/// signing in to iCloud later should be enough, no restart.
const RECONNECT_EVERY: u64 = 300;
/// The same while connecting failed for a reason that passes — the network,
/// iCloud having a moment: try again soon rather than in five minutes.
const RECONNECT_SOON: u64 = 30;
/// Milliseconds between heartbeats: the phone treats a Mac it has not heard
/// from in a while (asleep, quit) as offline instead of believing its sessions
/// are still running.
const HEARTBEAT_EVERY_MS: u64 = 10 * 60 * 1000;
/// How long the queue waits for the first pull before going out anyway.
const PULL_GRACE_MS: u64 = 15_000;
const DEFAULT_RETRY_SECS: u64 = 30;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    /// This build can talk to iCloud (macOS, signed with the entitlements).
    pub supported: bool,
    pub enabled: bool,
    /// off | starting | ready | syncing | idle | noAccount | restricted | unavailable | error
    pub state: String,
    pub message: Option<String>,
}

#[derive(Default)]
struct Inner {
    ledger: Option<Ledger>,
    ledger_path: Option<PathBuf>,
    /// Where the library keeps its change token and downloaded files.
    dir: Option<PathBuf>,
    state: String,
    message: Option<String>,
    host: String,
    /// The account is usable and the zone exists.
    ready: bool,
    ready_at: u64,
    /// The first pull after `ready` has finished.
    pulled: bool,
    /// Do not send before this (ms), after a failed push.
    retry_at: u64,
    /// What the panels last said, kept while sync is off: turning it on has
    /// something to send, and `None` means the panel has not said anything yet
    /// — which is not the same as saying it is empty, and must not make the
    /// records of the last run look as if they had gone.
    sessions: Option<Vec<AgentSession>>,
    clips: Option<Vec<ClipItem>>,
    shelf: Option<Vec<ShelfEntry>>,
    /// Record id → the status and detail a session was last sent with, and
    /// since when. A Codex session's time is its log's mtime, which moves
    /// with every line it writes; only a new status or question is news.
    session_since: HashMap<String, (Value, Option<String>, u64)>,
    /// When the last heartbeat was queued (ms), by the wall clock: after the
    /// Mac wakes from sleep the next one goes at once.
    last_heartbeat: u64,
}

#[derive(Default)]
pub struct SyncHub {
    inner: Mutex<Inner>,
    enabled: AtomicBool,
    /// Stops the flush thread of a previous `enable`.
    generation: AtomicU64,
}

/// What the shelf panel reports for each file on it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfEntry {
    path: String,
    name: String,
    #[serde(default)]
    extension: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    is_dir: bool,
    #[serde(default)]
    is_image: bool,
    #[serde(default)]
    added_at: f64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default()
}

/// A debug build (scripts/dev-icloud.sh) talks to CloudKit's Development
/// environment and a release to Production: two databases, so two ledgers and
/// two change tokens, though both builds share the app's config directory.
fn sync_dir(app: &AppHandle) -> Option<PathBuf> {
    let name = if cfg!(debug_assertions) { "sync-dev" } else { "sync" };
    app.path().app_config_dir().ok().map(|dir| dir.join(name))
}

pub fn status(app: &AppHandle) -> SyncStatus {
    let hub = app.state::<SyncHub>();
    let enabled = hub.enabled.load(Ordering::Relaxed);
    let inner = hub.inner.lock().unwrap();
    SyncStatus {
        supported: cloud::supported(),
        enabled,
        state: if enabled && !inner.state.is_empty() { inner.state.clone() } else { "off".into() },
        message: if enabled { inner.message.clone() } else { None },
    }
}

/// The line under the tray menu's sync switch.
pub fn status_label(status: &SyncStatus) -> String {
    if !status.supported {
        return t("这个版本没有 iCloud 权限", "This build has no iCloud entitlement").into();
    }
    if !status.enabled {
        return t("已关闭", "Off").into();
    }
    match status.state.as_str() {
        "starting" => t("正在连接 iCloud…", "Connecting to iCloud…").into(),
        // Every pull is a syncing → idle; the menu does not need to know.
        "ready" | "idle" | "syncing" => t("已连接，改动会自动同步", "Connected; changes sync on their own").into(),
        "noAccount" => t("这台 Mac 没有登录 iCloud", "This Mac is not signed in to iCloud").into(),
        "restricted" => t("iCloud 被限制", "iCloud is restricted").into(),
        "unavailable" => t("iCloud 暂时不可用", "iCloud is temporarily unavailable").into(),
        // The detail is CloudKit's English and is in the log already; the menu
        // only needs to say that it will sort itself out.
        _ => t("暂时没能同步，稍后会自动重试", "Couldn't sync; trying again shortly").into(),
    }
}

/// Called once at start-up, after the panels have loaded their data.
pub fn start(app: AppHandle) {
    if app.state::<SettingsState>().get().icloud_sync {
        enable(&app, false);
    }
}

/// The tray switch. Turning it on is refused on a build that cannot reach
/// iCloud; the menu does not offer it there, this is the second lock.
pub fn set_enabled(app: &AppHandle, on: bool) {
    if on && !cloud::supported() {
        return;
    }
    settings::update(app, |settings| settings.icloud_sync = on);
    if on {
        // Turned on by hand: whatever arrived while it was off was dropped
        // unread, so the change token cannot be trusted. Start from scratch;
        // what is already known comes back as echoes and is ignored.
        enable(app, true);
    } else {
        disable(app);
    }
    crate::tray::refresh(app);
}

fn enable(app: &AppHandle, from_scratch: bool) {
    let hub = app.state::<SyncHub>();
    if hub.enabled.load(Ordering::Relaxed) {
        return;
    }
    if !cloud::supported() {
        // A development build runs without the entitlements; the setting is
        // left as it is for the day this binary is a signed one.
        return;
    }
    let Some(dir) = sync_dir(app) else { return };
    if from_scratch {
        let _ = std::fs::remove_file(dir.join(TOKEN_FILE));
    }
    let ledger_path = dir.join("ledger.json");
    let mut ledger = Ledger::load(&ledger_path).unwrap_or_else(|| {
        if ledger_path.exists() {
            // Unreadable, so it is set aside rather than overwritten.
            let _ = std::fs::rename(&ledger_path, dir.join("ledger.json.bad"));
        }
        Ledger::new(random_device_id())
    });
    ledger.purge(now_ms(), tombstone_ttl);

    let todos = app.state::<crate::todos::TodoHub>().current();
    let gone_elsewhere = catch_up_todos(&mut ledger, &todos, now_ms());

    {
        let mut inner = hub.inner.lock().unwrap();
        inner.host = host_name();
        inner.ledger = Some(ledger);
        inner.ledger_path = Some(ledger_path);
        inner.dir = Some(dir.clone());
        inner.ready = false;
        inner.pulled = false;
        inner.retry_at = 0;
        inner.last_heartbeat = 0;
        inner.state = "starting".into();
        inner.message = None;
        save(&inner);
    }
    hub.enabled.store(true, Ordering::Relaxed);
    if !gone_elsewhere.is_empty() {
        crate::todos::forget(app, &gone_elsewhere);
    }
    // What the panels have said so far.
    reconcile_sessions(app);
    reconcile_clips(app);
    reconcile_shelf(app);

    let generation = hub.generation.fetch_add(1, Ordering::Relaxed) + 1;
    cloud::start(app, &dir);
    let handle = app.clone();
    thread::spawn(move || pump(handle, generation));
}

/// Brings the ledger up to date with the to-do list as it is now, which can
/// have moved on without it: sync was off, or the app quit between saving the
/// one and the other. Returns the ids the ledger knows were deleted (from the
/// phone) that are still on the list, for the caller to take off.
fn catch_up_todos(ledger: &mut Ledger, todos: &[Todo], now: u64) -> Vec<String> {
    let here: std::collections::HashSet<&str> = todos.iter().map(|todo| todo.id.as_str()).collect();
    // Deleted while nobody was listening: the phone still has to hear it.
    for id in ledger.live_ids(TODO) {
        if !here.contains(id.as_str()) {
            ledger.local_delete(TODO, &id, now);
        }
    }
    // New or changed since (added, ticked, unticked): stamped with when that
    // happened rather than now, so it never beats a later edit made elsewhere.
    // An unchanged line is a no-op.
    let deleted = ledger.deleted_ids(TODO);
    for todo in todos {
        if todo.id != clean_id(&todo.id) || deleted.contains(&todo.id) {
            continue;
        }
        ledger.local_upsert(TODO, &todo.id, todo_body(todo), None, todo.done_at.unwrap_or(todo.created_at));
    }
    todos.iter().filter(|todo| deleted.contains(&todo.id)).map(|todo| todo.id.clone()).collect()
}

/// How long a tombstone is remembered. A to-do must not come back from a
/// device that was away for weeks; a mirrored clip or session is only ever
/// written by this Mac, and once it is gone it is never sent again.
fn tombstone_ttl(kind: &str) -> u64 {
    if kind == TODO {
        90 * DAY_MS
    } else {
        DAY_MS
    }
}

/// The cloud lost what this Mac had put there — the zone was deleted (or the
/// development environment reset), or another iCloud account is signed in.
/// Everything this Mac knows goes up again, as if sync had just been turned on.
fn reset(app: &AppHandle) {
    let todos = app.state::<crate::todos::TodoHub>().current();
    let hub = app.state::<SyncHub>();
    {
        let mut inner = hub.inner.lock().unwrap();
        let Some(ledger) = inner.ledger.as_mut() else { return };
        let mut fresh = Ledger::new(ledger.device.clone());
        catch_up_todos(&mut fresh, &todos, now_ms());
        *ledger = fresh;
        save(&inner);
    }
    reconcile_sessions(app);
    reconcile_clips(app);
    reconcile_shelf(app);
}

/// The tray's "Sync now": pull right away, and send whatever is waiting
/// without sitting out a retry delay.
pub fn sync_now(app: &AppHandle) {
    let hub = app.state::<SyncHub>();
    if !hub.enabled.load(Ordering::Relaxed) {
        return;
    }
    let (ready, dir) = {
        let mut inner = hub.inner.lock().unwrap();
        inner.retry_at = 0;
        (inner.ready, inner.dir.clone())
    };
    if ready {
        cloud::pull();
        flush(app);
    } else if let Some(dir) = dir {
        // No usable account last time: look again.
        cloud::start(app, &dir);
    }
}

fn disable(app: &AppHandle) {
    let hub = app.state::<SyncHub>();
    if !hub.enabled.swap(false, Ordering::Relaxed) {
        return;
    }
    hub.generation.fetch_add(1, Ordering::Relaxed);
    {
        let mut inner = hub.inner.lock().unwrap();
        save(&inner);
        inner.ledger = None;
        inner.ready = false;
        inner.state.clear();
        inner.message = None;
    }
    cloud::stop();
}

/// Once a second: pull now and then, send what has piled up.
fn pump(app: AppHandle, generation: u64) {
    let mut tick: u64 = 0;
    loop {
        thread::sleep(Duration::from_secs(1));
        let hub = app.state::<SyncHub>();
        if hub.generation.load(Ordering::Relaxed) != generation || !hub.enabled.load(Ordering::Relaxed) {
            return;
        }
        tick += 1;
        let (ready, dir, state, last_heartbeat) = {
            let inner = hub.inner.lock().unwrap();
            (inner.ready, inner.dir.clone(), inner.state.clone(), inner.last_heartbeat)
        };
        if ready && tick % PULL_EVERY == 0 {
            cloud::pull();
        }
        // The first one right after start-up, then every ten minutes of real
        // time — sleep included, so a Mac that just woke up says so at once.
        if now_ms().saturating_sub(last_heartbeat) >= HEARTBEAT_EVERY_MS {
            hub.inner.lock().unwrap().last_heartbeat = now_ms();
            heartbeat(&app);
        }
        let reconnect_every = match state.as_str() {
            "error" | "unavailable" => RECONNECT_SOON,
            _ => RECONNECT_EVERY,
        };
        if !ready && tick % reconnect_every == 0 {
            if let Some(dir) = dir {
                cloud::start(&app, &dir);
            }
        }
        if tick % (DAY_MS / 1000) == 0 {
            let mut inner = hub.inner.lock().unwrap();
            if let Some(ledger) = inner.ledger.as_mut() {
                ledger.purge(now_ms(), tombstone_ttl);
            }
            save(&inner);
        }
        flush(&app);
    }
}

fn flush(app: &AppHandle) {
    let batch = {
        let hub = app.state::<SyncHub>();
        let mut inner = hub.inner.lock().unwrap();
        let now = now_ms();
        // The queue waits for the first pull, so a stale offline change meets
        // the newer version from the other device before it goes out.
        let waiting_for_pull = !inner.pulled && now < inner.ready_at + PULL_GRACE_MS;
        if !inner.ready || waiting_for_pull || now < inner.retry_at {
            return;
        }
        match inner.ledger.as_mut() {
            Some(ledger) => ledger.take_outbox(),
            None => return,
        }
    };
    if batch.is_empty() {
        return;
    }
    match serde_json::to_string(&batch) {
        Ok(json) => cloud::push(&json),
        Err(error) => {
            eprintln!("[sync] cannot encode the queue: {error}");
            let keys: Vec<String> = batch.iter().map(Record::key).collect();
            let hub = app.state::<SyncHub>();
            let mut inner = hub.inner.lock().unwrap();
            if let Some(ledger) = inner.ledger.as_mut() {
                ledger.release(&keys);
            }
        }
    }
}

fn save(inner: &Inner) {
    if let (Some(ledger), Some(path)) = (&inner.ledger, &inner.ledger_path) {
        if let Err(error) = ledger.save(path) {
            eprintln!("[sync] failed to save {}: {error}", path.display());
        }
    }
}

/// What the library reports (see the events in docs/sync.md).
#[derive(Debug, Deserialize)]
#[serde(tag = "event", rename_all = "camelCase")]
enum Event {
    Status {
        state: String,
        #[serde(default)]
        message: Option<String>,
    },
    Records {
        records: Vec<Record>,
    },
    Pushed {
        keys: Vec<String>,
    },
    Rejected {
        keys: Vec<String>,
    },
    Failed {
        keys: Vec<String>,
        #[serde(default)]
        message: String,
        #[serde(default, rename = "retryAfter")]
        retry_after: Option<u64>,
    },
    /// The cloud no longer has what was put there: "zone" or "account".
    Reset {
        #[serde(default)]
        reason: String,
    },
}

/// Entry point for everything the library says. Runs on whatever thread it
/// likes.
pub(crate) fn on_event(app: &AppHandle, raw: &str) {
    let event = match serde_json::from_str::<Event>(raw) {
        Ok(event) => event,
        Err(error) => {
            eprintln!("[sync] unreadable event ({error}): {raw}");
            return;
        }
    };
    let hub = app.state::<SyncHub>();
    if !hub.enabled.load(Ordering::Relaxed) {
        return;
    }
    match event {
        Event::Status { state, message } => {
            let (just_ready, label_changed) = {
                let mut inner = hub.inner.lock().unwrap();
                match state.as_str() {
                    "ready" => {
                        inner.ready = true;
                        inner.ready_at = now_ms();
                        inner.pulled = false;
                        // A (re)started library holds nothing of what was sent before.
                        if let Some(ledger) = inner.ledger.as_mut() {
                            ledger.release_all();
                        }
                    }
                    "idle" => {
                        if inner.ready {
                            inner.pulled = true;
                        }
                    }
                    // Nothing can be done until the account is back; the flush
                    // thread tries to connect again now and then.
                    "noAccount" | "restricted" | "unavailable" => inner.ready = false,
                    // `error` ends a pull or push that failed: the account is
                    // fine, and the next pull or the retry time takes care of it.
                    _ => {}
                }
                let label = |state: &str, message: &Option<String>| {
                    status_label(&SyncStatus { supported: true, enabled: true, state: state.into(), message: message.clone() })
                };
                let before = label(&inner.state, &inner.message);
                inner.state = state.clone();
                inner.message = message;
                (state == "ready", before != label(&inner.state, &inner.message))
            };
            if just_ready {
                cloud::pull();
            }
            if label_changed {
                crate::tray::refresh(app);
            }
        }
        Event::Records { records } => {
            let applied = {
                let mut inner = hub.inner.lock().unwrap();
                let Some(ledger) = inner.ledger.as_mut() else { return };
                ledger.apply_remote(records)
            };
            if applied.is_empty() {
                return;
            }
            // Only the to-do list is edited from the phone; the other kinds
            // are the Mac's own mirror coming back, and need nothing here.
            let todos: Vec<Record> = applied.into_iter().filter(|record| record.kind == TODO).collect();
            if !todos.is_empty() {
                crate::todos::apply_remote(app, todos);
            }
            // The list is on disk before the ledger says it was applied: if the
            // app dies in between, the next start sees the difference and
            // settles it (`catch_up_todos`) instead of losing the change.
            save(&hub.inner.lock().unwrap());
        }
        Event::Pushed { keys } | Event::Rejected { keys } => {
            let mut inner = hub.inner.lock().unwrap();
            if let Some(ledger) = inner.ledger.as_mut() {
                ledger.acknowledge(&keys);
            }
            save(&inner);
        }
        Event::Failed { keys, message, retry_after } => {
            eprintln!("[sync] push failed ({} records): {message}", keys.len());
            let mut inner = hub.inner.lock().unwrap();
            if let Some(ledger) = inner.ledger.as_mut() {
                ledger.release(&keys);
            }
            inner.retry_at = now_ms() + retry_after.unwrap_or(DEFAULT_RETRY_SECS) * 1000;
        }
        Event::Reset { reason } => {
            eprintln!("[sync] the cloud lost this Mac's records ({reason}); sending everything again");
            reset(app);
        }
    }
}

// ---------------------------------------------------------------- to-dos

fn todo_body(todo: &Todo) -> Value {
    json!({ "text": todo.text, "createdAt": todo.created_at, "done": todo.done, "doneAt": todo.done_at })
}

/// A line was added here.
pub fn todo_upsert(app: &AppHandle, todo: &Todo) {
    // An id CloudKit would refuse cannot be synced; ids made by Bangs and by
    // the iOS app are always fine.
    if !enabled(app) || todo.id != clean_id(&todo.id) {
        return;
    }
    change(app, |ledger| ledger.local_upsert(TODO, &todo.id, todo_body(todo), None, now_ms()).is_some());
}

/// A line was deleted here.
pub fn todo_delete(app: &AppHandle, id: &str) {
    if !enabled(app) {
        return;
    }
    change(app, |ledger| ledger.local_delete(TODO, id, now_ms()).is_some());
}

/// The to-do list as a `Todo` again, from a record that came from the phone.
/// A record from before there was a done state reads as not done.
pub fn todo_from_record(record: &Record) -> Option<Todo> {
    let text = record.body.get("text")?.as_str()?.to_string();
    let created_at = record.body.get("createdAt").and_then(Value::as_u64).unwrap_or(record.updated_at);
    let done = record.body.get("done").and_then(Value::as_bool).unwrap_or(false);
    let done_at = if done { Some(record.body.get("doneAt").and_then(Value::as_u64).unwrap_or(record.updated_at)) } else { None };
    Some(Todo { id: record.id.clone(), text, created_at, done, done_at })
}

// -------------------------------------------------- what the Mac mirrors

/// Claude Code and Codex sessions, for the phone's Code tab.
pub fn mirror_sessions(app: &AppHandle, sessions: &[AgentSession]) {
    app.state::<SyncHub>().inner.lock().unwrap().sessions = Some(sessions.to_vec());
    reconcile_sessions(app);
}

fn reconcile_sessions(app: &AppHandle) {
    let Some((device, host)) = identity(app) else { return };
    let hub = app.state::<SyncHub>();
    let desired = {
        let mut inner = hub.inner.lock().unwrap();
        let Some(sessions) = inner.sessions.clone() else { return };
        let mut since = HashMap::new();
        let desired = sessions
            .iter()
            .map(|session| {
                let id = format!("{device}-{}", clean_id(&session.id));
                let status = json!(session.status);
                let status_at = match inner.session_since.get(&id) {
                    Some((was, detail, at)) if *was == status && *detail == session.detail => *at,
                    _ => session.updated_at as u64,
                };
                since.insert(id.clone(), (status.clone(), session.detail.clone(), status_at));
                Desired {
                    id,
                    body: json!({
                        "agent": session.agent,
                        "name": session.name,
                        "project": session.project,
                        "path": session.path,
                        "status": status,
                        "detail": session.detail,
                        "statusAt": status_at,
                        "host": host,
                    }),
                    asset: None,
                }
            })
            .collect::<Vec<_>>();
        inner.session_since = since;
        desired
    };
    reconcile(app, SESSION, desired);
}

/// The newest clipboard entries.
pub fn mirror_clips(app: &AppHandle, items: &[ClipItem]) {
    // The icons are the bulk of an item and the phone never sees them.
    let slim = items
        .iter()
        .take(CLIP_WINDOW)
        .map(|item| ClipItem { icon: None, ..item.clone() })
        .collect();
    app.state::<SyncHub>().inner.lock().unwrap().clips = Some(slim);
    reconcile_clips(app);
}

fn reconcile_clips(app: &AppHandle) {
    let Some((device, host)) = identity(app) else { return };
    let _pictures = CLIP_IMAGES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(items) = app.state::<SyncHub>().inner.lock().unwrap().clips.clone() else { return };
    let text_ids: Vec<i64> = items.iter().filter(|item| item.kind == ClipKind::Text).map(|item| item.id).collect();
    // Without the texts every id would change (they are hashes of the text),
    // which would delete and resend the whole window; try again next time.
    let Some(texts) = crate::clipboard::full_texts(app, &text_ids) else { return };

    let mut seen = std::collections::HashSet::new();
    let mut desired = Vec::new();
    for item in &items {
        let text = texts.get(&item.id).map(|text| text.chars().take(CLIP_CHARS).collect::<String>());
        // The same words copied twice are one entry; a picture or a file has
        // nothing but its preview, so its time tells two of them apart.
        let basis = match &text {
            Some(text) => text.clone(),
            None => format!("{:?}|{}|{}|{}", item.kind, item.preview, item.app.as_deref().unwrap_or(""), item.created_at as u64),
        };
        let id = format!("{device}-{}", fnv_hex(&basis));
        if !seen.insert(id.clone()) {
            continue;
        }
        let image = if item.kind == ClipKind::Image { clip_image(app, item.id, &id) } else { None };
        desired.push(Desired {
            body: json!({
                "type": item.kind,
                "preview": item.preview,
                "text": text,
                "app": item.app,
                "pinned": item.pinned,
                "createdAt": item.created_at as u64,
                "host": host,
                // Also what makes a picture that only now could be made go out again.
                "hasImage": image.is_some(),
            }),
            asset: image,
            id,
        });
    }
    prune_clip_images(app, &seen);
    reconcile(app, CLIP, desired);
}

/// Clipboard pictures are made and pruned one reconcile at a time: the
/// clipboard poll and turning sync on can both get here at once.
static CLIP_IMAGES: Mutex<()> = Mutex::new(());

/// The picture the phone gets for a copied image, made once and kept in
/// `<sync dir>/clips/<record id>.img` while the entry is in the window: the
/// original when it is small enough and in a format the phone opens (a
/// screenshot of text stays sharp), otherwise a JPEG made small enough.
fn clip_image(app: &AppHandle, clip_id: i64, record_id: &str) -> Option<String> {
    let dir = sync_dir(app)?.join("clips");
    let picture = dir.join(format!("{record_id}.img"));
    if !picture.exists() {
        let bytes = crate::clipboard::image_bytes(app, clip_id)?;
        std::fs::create_dir_all(&dir).ok()?;
        // Made under another name and renamed: a half-written file would
        // otherwise go out as it is, for as long as the entry is in the window.
        let part = dir.join(format!("{record_id}.part"));
        let made = if bytes.len() as u64 <= CLIP_IMAGE_BYTES && phone_opens(&bytes) {
            std::fs::write(&part, &bytes).is_ok()
        } else {
            let raw = dir.join(format!("{record_id}.raw"));
            let made = std::fs::write(&raw, &bytes).is_ok() && shrink_to_jpeg(&raw, &part);
            let _ = std::fs::remove_file(&raw);
            made
        };
        if !made || std::fs::rename(&part, &picture).is_err() {
            let _ = std::fs::remove_file(&part);
            return None;
        }
    }
    Some(picture.to_string_lossy().into_owned())
}

/// PNG, JPEG, GIF, TIFF or HEIC: what the phone's UIImage opens as it is.
fn phone_opens(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, b'P', b'N', b'G'])
        || bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"GIF8")
        || bytes.starts_with(b"II*\0")
        || bytes.starts_with(b"MM\0*")
        || (bytes.len() >= 12 && &bytes[4..8] == b"ftyp" && matches!(&bytes[8..12], b"heic" | b"heix" | b"mif1" | b"hevc"))
}

/// Turns any picture into a JPEG under `CLIP_IMAGE_BYTES`, smaller and rougher
/// until it fits, with macOS's own `sips`. Never larger than it was.
fn shrink_to_jpeg(source: &Path, target: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        let longest = longest_side(source);
        for (side, quality) in [(1600u32, 75), (1024, 60), (640, 50)] {
            let mut command = std::process::Command::new("/usr/bin/sips");
            command.args(["-s", "format", "jpeg", "-s", "formatOptions", &quality.to_string()]);
            // Only ever smaller: no `-Z` for a picture already within the side.
            if longest.map_or(true, |longest| longest > side) {
                command.args(["-Z", &side.to_string()]);
            }
            let made = command
                .arg(source)
                .arg("--out")
                .arg(target)
                .output()
                .is_ok_and(|output| output.status.success());
            let size = std::fs::metadata(target).map(|meta| meta.len()).unwrap_or(u64::MAX);
            if made && size <= CLIP_IMAGE_BYTES {
                return true;
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = source;
    let _ = std::fs::remove_file(target);
    false
}

/// The longer of a picture's width and height, as `sips` reads it.
#[cfg(target_os = "macos")]
fn longest_side(path: &Path) -> Option<u32> {
    let output = std::process::Command::new("/usr/bin/sips")
        .args(["-g", "pixelWidth", "-g", "pixelHeight"])
        .arg(path)
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (key, value) = line.trim().split_once(':')?;
            matches!(key.trim(), "pixelWidth" | "pixelHeight").then(|| value.trim().parse::<u32>().ok())?
        })
        .max()
}

/// Pictures of entries that have left the window are not needed any more, nor
/// anything a conversion left behind.
fn prune_clip_images(app: &AppHandle, keep: &std::collections::HashSet<String>) {
    let Some(dir) = sync_dir(app).map(|dir| dir.join("clips")) else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let stem = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
        let picture = path.extension().is_some_and(|extension| extension == "img");
        if !picture || !keep.contains(&stem) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The shelf panel lives in the webview; it reports the whole list whenever
/// it changes.
#[tauri::command]
pub fn shelf_sync(app: AppHandle, items: Vec<ShelfEntry>) {
    app.state::<SyncHub>().inner.lock().unwrap().shelf = Some(items);
    reconcile_shelf(&app);
}

fn reconcile_shelf(app: &AppHandle) {
    let Some((device, host)) = identity(app) else { return };
    let Some(items) = app.state::<SyncHub>().inner.lock().unwrap().shelf.clone() else { return };
    let desired = items
        .iter()
        .filter(|entry| !entry.is_dir)
        .map(|entry| Desired {
            id: format!("{device}-{}", fnv_hex(&entry.path)),
            body: json!({
                "name": entry.name,
                "extension": entry.extension,
                "size": entry.size,
                "isImage": entry.is_image,
                "addedAt": entry.added_at as u64,
                "host": host,
            }),
            asset: (entry.size <= SHELF_FILE_BYTES && Path::new(&entry.path).is_file()).then(|| entry.path.clone()),
        })
        .collect();
    reconcile(app, SHELF, desired);
}

/// "This Mac is here": its name and the time, so the phone can tell a Mac that
/// went to sleep or quit Bangs from one whose sessions are just quiet.
fn heartbeat(app: &AppHandle) {
    let Some((device, host)) = identity(app) else { return };
    let now = now_ms();
    change(app, |ledger| {
        ledger.local_upsert(DEVICE, &device, json!({ "host": host, "seenAt": now, "platform": "macOS" }), None, now).is_some()
    });
}

// --------------------------------------------------------------- helpers

fn enabled(app: &AppHandle) -> bool {
    app.state::<SyncHub>().enabled.load(Ordering::Relaxed)
}

/// The short device id that prefixes mirrored record ids, and the computer's
/// name; `None` while sync is off.
fn identity(app: &AppHandle) -> Option<(String, String)> {
    let hub = app.state::<SyncHub>();
    if !hub.enabled.load(Ordering::Relaxed) {
        return None;
    }
    let inner = hub.inner.lock().unwrap();
    let ledger = inner.ledger.as_ref()?;
    let short: String = ledger.device.chars().filter(|c| c.is_ascii_alphanumeric()).take(8).collect();
    Some((short, inner.host.clone()))
}

fn reconcile(app: &AppHandle, kind: &str, desired: Vec<Desired>) {
    change(app, |ledger| !ledger.reconcile(kind, desired, now_ms()).is_empty());
}

/// Applies `edit` to the ledger and saves it when `edit` says it changed
/// something — the dev panel asks about every poll that changes anything.
fn change(app: &AppHandle, edit: impl FnOnce(&mut Ledger) -> bool) {
    let hub = app.state::<SyncHub>();
    let mut inner = hub.inner.lock().unwrap();
    let Some(ledger) = inner.ledger.as_mut() else { return };
    if edit(ledger) {
        save(&inner);
    }
}

fn host_name() -> String {
    #[cfg(target_os = "macos")]
    {
        // "gxl 的 MacBook Pro", as System Settings shows it.
        if let Ok(output) = std::process::Command::new("scutil").args(["--get", "ComputerName"]).output() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if output.status.success() && !name.is_empty() {
                return name;
            }
        }
    }
    sysinfo::System::host_name().unwrap_or_else(|| "Mac".into())
}

/// A random UUID, lower case. `/dev/urandom` where there is one; the clock
/// and the process id stirred together where there is not.
fn random_device_id() -> String {
    use std::io::Read;

    let mut bytes = [0u8; 16];
    let filled = std::fs::File::open("/dev/urandom").and_then(|mut file| file.read_exact(&mut bytes)).is_ok();
    if !filled {
        let mut seed = now_ms() ^ (u64::from(std::process::id()) << 32);
        for chunk in bytes.chunks_mut(8) {
            // splitmix64
            seed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = seed;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^= z >> 31;
            chunk.copy_from_slice(&z.to_le_bytes());
        }
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_id_is_a_uuid() {
        let id = random_device_id();
        assert_eq!(id.len(), 36);
        assert_eq!(id.matches('-').count(), 4);
        assert_eq!(&id[14..15], "4");
        assert_ne!(id, random_device_id());
    }

    #[test]
    fn small_pictures_the_phone_opens_go_as_they_are() {
        assert!(phone_opens(b"\x89PNG\r\n\x1a\n...."));
        assert!(phone_opens(b"\xff\xd8\xff\xe0..JFIF"));
        assert!(phone_opens(b"GIF89a......"));
        assert!(phone_opens(b"MM\0*........"));
        assert!(phone_opens(b"\0\0\0\x18ftypheic...."));
        // A PDF or a bitmap goes through sips first.
        assert!(!phone_opens(b"%PDF-1.7...."));
        assert!(!phone_opens(b"BM.........."));
        assert!(!phone_opens(b"\0\0\0\x18ftypisom...."));
        assert!(!phone_opens(b""));
    }

    #[test]
    fn a_record_from_the_phone_becomes_a_todo() {
        let record: Record = serde_json::from_value(json!({
            "kind": "todo", "id": "ABC-1", "updatedAt": 7, "device": "phone",
            "body": { "text": "买牛奶", "createdAt": 5 }
        }))
        .unwrap();
        let todo = todo_from_record(&record).unwrap();
        assert_eq!((todo.id.as_str(), todo.text.as_str(), todo.created_at), ("ABC-1", "买牛奶", 5));
        assert!(!todo.done && todo.done_at.is_none(), "no done field means not done");
        let ticked: Record = serde_json::from_value(json!({
            "kind": "todo", "id": "x", "updatedAt": 9, "device": "phone",
            "body": { "text": "t", "createdAt": 5, "done": true, "doneAt": 8 }
        }))
        .unwrap();
        let todo = todo_from_record(&ticked).unwrap();
        assert_eq!((todo.done, todo.done_at), (true, Some(8)));
        assert_eq!(todo_body(&todo)["done"], true);
        let broken: Record =
            serde_json::from_value(json!({ "kind": "todo", "id": "x", "updatedAt": 1, "device": "d", "body": {} })).unwrap();
        assert!(todo_from_record(&broken).is_none());
    }

    #[test]
    fn events_are_read_the_way_the_library_writes_them() {
        let status: Event = serde_json::from_str(r#"{"event":"status","state":"ready"}"#).unwrap();
        assert!(matches!(status, Event::Status { ref state, message: None } if state == "ready"));
        let failed: Event =
            serde_json::from_str(r#"{"event":"failed","keys":["todo:1"],"message":"offline","retryAfter":30}"#).unwrap();
        assert!(matches!(failed, Event::Failed { retry_after: Some(30), .. }));
        let records: Event = serde_json::from_str(
            r#"{"event":"records","records":[{"kind":"todo","id":"1","updatedAt":5,"device":"d","deleted":true,"body":{},"asset":null}]}"#,
        )
        .unwrap();
        assert!(matches!(records, Event::Records { ref records } if records.len() == 1 && records[0].deleted));
        let pushed: Event = serde_json::from_str(r#"{"event":"pushed","keys":["a:b"]}"#).unwrap();
        assert!(matches!(pushed, Event::Pushed { .. }));
        let reset: Event = serde_json::from_str(r#"{"event":"reset","reason":"zone"}"#).unwrap();
        assert!(matches!(reset, Event::Reset { ref reason } if reason == "zone"));
    }

    fn todo(id: &str, created_at: u64) -> Todo {
        Todo::new(id.into(), format!("line {id}"), created_at)
    }

    #[test]
    fn catching_up_adopts_new_lines_at_the_time_they_were_written() {
        let mut ledger = Ledger::new("mac".into());
        let gone = catch_up_todos(&mut ledger, &[todo("a", 5), todo("b", 7)], 1_000);
        assert!(gone.is_empty());
        let mut queued: Vec<(String, u64)> =
            ledger.take_outbox().into_iter().map(|record| (record.id, record.updated_at)).collect();
        queued.sort();
        assert_eq!(queued, vec![("a".to_string(), 5), ("b".to_string(), 7)]);
    }

    #[test]
    fn a_line_deleted_while_sync_was_off_is_deleted_everywhere() {
        let mut ledger = Ledger::new("mac".into());
        catch_up_todos(&mut ledger, &[todo("a", 5), todo("b", 7)], 1_000);
        let sent: Vec<String> = ledger.take_outbox().iter().map(Record::key).collect();
        ledger.acknowledge(&sent);
        // Sync off; "a" deleted; sync on again.
        catch_up_todos(&mut ledger, &[todo("b", 7)], 2_000);
        let queued = ledger.take_outbox();
        assert_eq!(queued.len(), 1);
        assert_eq!((queued[0].id.as_str(), queued[0].deleted), ("a", true));
    }

    #[test]
    fn a_line_ticked_while_sync_was_off_reaches_the_phone() {
        let mut ledger = Ledger::new("mac".into());
        catch_up_todos(&mut ledger, &[todo("a", 5)], 1_000);
        let sent: Vec<String> = ledger.take_outbox().iter().map(Record::key).collect();
        ledger.acknowledge(&sent);
        let ticked = Todo { done: true, done_at: Some(900), ..todo("a", 5) };
        catch_up_todos(&mut ledger, &[ticked], 2_000);
        let queued = ledger.take_outbox();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].body["done"], true);
        // Unchanged since: nothing to send.
        ledger.acknowledge(&[queued[0].key()]);
        catch_up_todos(&mut ledger, &[Todo { done: true, done_at: Some(900), ..todo("a", 5) }], 3_000);
        assert!(ledger.take_outbox().is_empty());
    }

    #[test]
    fn a_line_deleted_on_the_phone_but_still_here_is_handed_back() {
        let mut ledger = Ledger::new("mac".into());
        catch_up_todos(&mut ledger, &[todo("a", 5)], 1_000);
        // The phone's tombstone was taken in, then the app died before the list was saved.
        ledger.apply_remote(vec![Record {
            kind: TODO.into(),
            id: "a".into(),
            updated_at: 1_500,
            device: "phone".into(),
            deleted: true,
            body: json!({}),
            asset: None,
        }]);
        let gone = catch_up_todos(&mut ledger, &[todo("a", 5)], 2_000);
        assert_eq!(gone, vec!["a".to_string()]);
    }

    #[test]
    fn the_status_line_follows_the_state() {
        let mut status = SyncStatus { supported: true, enabled: true, state: "noAccount".into(), message: None };
        assert!(status_label(&status).contains("iCloud"));
        status.state = "error".into();
        status.message = Some("The Internet connection appears to be offline.".into());
        assert!(!status_label(&status).contains("Internet"), "CloudKit's own words stay in the log");
        status.supported = false;
        assert!(!status_label(&status).is_empty());
    }
}
