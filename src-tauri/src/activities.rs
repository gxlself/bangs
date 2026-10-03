//! What anything else on this machine wants the notch to say.
//!
//! A plugin is whatever writes a JSON file into `<config>/activities/`; the
//! loopback API in this module is a convenience that writes the same files, so
//! there is only ever one place to read from. Rows carry text and at most a
//! link — never a command — because writing that directory is not a privilege
//! anyone should have to think twice about.
//!
//! See docs/plugins.md for the contract other programs code against.

use std::fs;
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

use crate::http::{self, reply};

/// The directory is cheap to stat, and a plugin that just wrote a file should
/// not wait for the notch to notice.
const POLL: Duration = Duration::from_millis(700);
/// Loopback only. Picked to be memorable and out of the ephemeral range.
const PORT: u16 = 17650;
/// A row is a line of text; anything longer is a mistake on the other side.
const MAX_FIELD: usize = 200;
const MAX_BODY: usize = 32 * 1024;
/// The panel scrolls, but a plugin cannot be allowed to bury everything else.
const MAX_ACTIVITIES: usize = 12;

/// One line on the notch, as a plugin describes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    /// Stable across updates; the file name is used when it is left out.
    #[serde(default)]
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    /// A glyph name the notch knows (see docs/plugins.md); anything else
    /// draws the neutral dot.
    #[serde(default)]
    pub icon: Option<String>,
    /// 0…1, drawn as a bar under the title.
    #[serde(default)]
    pub progress: Option<f64>,
    /// Opened with the system handler when the row is clicked. http(s) only —
    /// anything else is dropped on the way in.
    #[serde(default)]
    pub url: Option<String>,
    /// Unix seconds. The row leaves on its own once this passes, so a plugin
    /// that dies does not leave litter on the screen.
    #[serde(default)]
    pub expires_at: Option<u64>,
    /// Sort key and tie-breaker: when the file was last written.
    #[serde(default, skip_deserializing)]
    pub updated_at: u64,
}

impl Activity {
    /// Everything a plugin sends is treated as a suggestion.
    fn sanitize(mut self, id: String, updated_at: u64) -> Option<Self> {
        self.id = clip(if self.id.is_empty() { id } else { self.id });
        self.title = clip(self.title);
        if self.id.is_empty() || self.title.is_empty() {
            return None;
        }
        self.subtitle = self.subtitle.map(clip).filter(|text| !text.is_empty());
        self.icon = self.icon.map(clip).filter(|text| !text.is_empty());
        self.progress = self.progress.filter(|value| value.is_finite()).map(|value| value.clamp(0.0, 1.0));
        self.url = self.url.filter(|url| {
            // A link opens with the system handler, so only the two schemes
            // that mean "a web page" get through — no file://, no custom app
            // scheme, no javascript:.
            let url = url.to_ascii_lowercase();
            url.starts_with("http://") || url.starts_with("https://")
        });
        self.updated_at = updated_at;
        Some(self)
    }

    fn expired(&self, now: u64) -> bool {
        self.expires_at.is_some_and(|deadline| deadline <= now)
    }
}

fn clip(text: String) -> String {
    let text = text.trim().replace(['\n', '\r', '\t'], " ");
    text.chars().take(MAX_FIELD).collect()
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

/// The last list handed to the webview, so the poll loop can stay quiet.
#[derive(Default)]
pub struct ActivityHub(Mutex<Vec<Activity>>);

impl ActivityHub {
    pub fn current(&self) -> Vec<Activity> {
        self.0.lock().unwrap().clone()
    }
}

fn dir(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_config_dir().ok()?.join("activities");
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn token_path(app: &AppHandle) -> Option<PathBuf> {
    Some(app.path().app_config_dir().ok()?.join("api-token"))
}

/// Reads every plugin's file. A file that cannot be parsed is skipped rather
/// than breaking the rest of the list.
fn read_all(app: &AppHandle) -> Vec<Activity> {
    let Some(dir) = dir(app) else {
        return Vec::new();
    };
    let now = now_seconds();
    let mut activities: Vec<Activity> = Vec::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        return activities;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let stem = path.file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_default();
        let updated_at = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|since| since.as_secs())
            .unwrap_or(now);
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(activity) = serde_json::from_str::<Activity>(&text) else {
            continue;
        };
        let Some(activity) = activity.sanitize(stem, updated_at) else {
            continue;
        };
        // An expired row is litter: drop the file too, so the directory does
        // not grow forever when a plugin stops running.
        if activity.expired(now) {
            let _ = fs::remove_file(&path);
            continue;
        }
        activities.push(activity);
    }
    activities.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| a.id.cmp(&b.id)));
    activities.truncate(MAX_ACTIVITIES);
    activities
}

/// Watches the directory and tells the webview when the list changes.
pub fn start(app: AppHandle) {
    let _ = dir(&app);
    let _ = token(&app);
    spawn_api(app.clone());
    thread::spawn(move || loop {
        let next = read_all(&app);
        let changed = {
            let hub = app.state::<ActivityHub>();
            let mut current = hub.0.lock().unwrap();
            let changed = *current != next;
            if changed {
                *current = next.clone();
            }
            changed
        };
        if changed {
            let _ = app.emit("bangs://activities", next);
        }
        thread::sleep(POLL);
    });
}

/// Opens the link on a row, looked up here rather than taken from the webview.
#[tauri::command]
pub fn activity_open(app: AppHandle, id: String) -> Result<(), String> {
    let url = app
        .state::<ActivityHub>()
        .current()
        .into_iter()
        .find(|activity| activity.id == id)
        .and_then(|activity| activity.url)
        .ok_or_else(|| format!("no link on {id}"))?;
    app.opener().open_url(url, None::<&str>).map_err(|error| error.to_string())
}

// MARK: - The loopback API

/// The shared secret, made once and kept next to the settings. Anything that
/// can read it can already read the settings and write the directory, so it
/// is not a wall — it stops a web page in a browser from posting to the port.
fn token(app: &AppHandle) -> Option<String> {
    let path = token_path(app)?;
    if let Ok(existing) = fs::read_to_string(&path) {
        let existing = existing.trim().to_string();
        if !existing.is_empty() {
            return Some(existing);
        }
    }
    let token = http::random_token();
    fs::write(&path, &token).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Some(token)
}

fn spawn_api(app: AppHandle) {
    thread::spawn(move || {
        let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, PORT)) else {
            eprintln!("[activities] port {PORT} is taken; the file directory still works");
            return;
        };
        for stream in listener.incoming().flatten() {
            let app = app.clone();
            // One thread per request: a client that connects and then says
            // nothing would otherwise hold the whole API for its timeout.
            thread::spawn(move || handle(&app, stream));
        }
    });
}

fn handle(app: &AppHandle, mut stream: TcpStream) {
    let Some(request) = http::read_request(&mut stream, MAX_BODY) else {
        return reply(&mut stream, "400 Bad Request", r#"{"error":"bad request"}"#);
    };
    // A page in a browser can reach loopback; it cannot read the token file,
    // but it also has no business here at all.
    // Browsers set Origin; command line clients do not.
    if request.header("origin").is_some() {
        return reply(&mut stream, "403 Forbidden", r#"{"error":"not for browsers"}"#);
    }
    if token(app).as_deref() != request.header("x-bangs-token") {
        return reply(&mut stream, "401 Unauthorized", r#"{"error":"bad or missing X-Bangs-Token"}"#);
    }
    let Some(directory) = dir(app) else {
        return reply(&mut stream, "500 Internal Server Error", r#"{"error":"no config directory"}"#);
    };
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/activities") => {
            let body = serde_json::to_string(&read_all(app)).unwrap_or_else(|_| "[]".into());
            reply(&mut stream, "200 OK", &body);
        }
        ("POST", "/activity") => {
            let Ok(activity) = serde_json::from_str::<Activity>(&request.body) else {
                return reply(&mut stream, "400 Bad Request", r#"{"error":"body is not an activity"}"#);
            };
            let Some(activity) = activity.sanitize(String::new(), now_seconds()) else {
                return reply(&mut stream, "400 Bad Request", r#"{"error":"id and title are required"}"#);
            };
            let Some(file) = safe_name(&activity.id) else {
                return reply(&mut stream, "400 Bad Request", r#"{"error":"id must be letters, digits, - or _"}"#);
            };
            let body = serde_json::to_string(&activity).unwrap_or_default();
            match fs::write(directory.join(format!("{file}.json")), body) {
                Ok(()) => reply(&mut stream, "200 OK", r#"{"ok":true}"#),
                Err(error) => reply(
                    &mut stream,
                    "500 Internal Server Error",
                    &format!(r#"{{"error":{}}}"#, serde_json::to_string(&error.to_string()).unwrap_or_default()),
                ),
            }
        }
        ("DELETE", rest) if rest.starts_with("/activity/") => {
            let Some(file) = safe_name(rest.trim_start_matches("/activity/")) else {
                return reply(&mut stream, "400 Bad Request", r#"{"error":"bad id"}"#);
            };
            let _ = fs::remove_file(directory.join(format!("{file}.json")));
            reply(&mut stream, "200 OK", r#"{"ok":true}"#);
        }
        _ => reply(&mut stream, "404 Not Found", r#"{"error":"try POST /activity"}"#),
    }
}

/// Ids become file names, so they may not wander out of the directory.
fn safe_name(id: &str) -> Option<String> {
    let id = id.trim();
    let clean = id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    (clean && !id.is_empty() && id.len() <= 64).then(|| id.to_string())
}
