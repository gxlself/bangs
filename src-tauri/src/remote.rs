//! The Apple Watch app's way in: a small HTTP API on the local network.
//!
//! Unlike the loopback API in activities.rs this one leaves the machine, so it
//! is off until the tray turns it on, a watch pairs once with a six-digit code
//! the tray shows, and what a paired watch can see and do is a glance and a
//! remote: now playing and lyrics, the agent sessions, the to-do list and the
//! board. No paths, no clipboard, no files, no links to open.
//!
//! The traffic is plain HTTP — a watch app cannot be talked into trusting a
//! self-signed certificate — so the pairing code is single use, guesses are
//! throttled, and every paired watch can be forgotten from the tray.
//!
//! See docs/watch.md for the protocol the watch app codes against.

use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::ErrorKind;
use std::net::{Ipv4Addr, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Manager};

use crate::activities::ActivityHub;
use crate::dev::{Agent, DevHub, SessionStatus};
use crate::http::{self, reply};
use crate::lyrics::{LyricStatus, LyricsHub};
use crate::media::{self, MediaCommand, MediaHub};
use crate::settings::SettingsState;
use crate::todos::{self, Todo, TodoHub};
use crate::tray;

/// Next to the loopback API's 17650, and just as out of the ephemeral range.
pub const PORT: u16 = 17651;
/// Requests are a pairing code, a transport command or a line of text.
const MAX_BODY: usize = 8 * 1024;
/// Wrong codes before the code shown in the tray is replaced.
const MAX_FAILURES: u32 = 5;
/// After a wrong code nobody gets another guess for this long, which keeps a
/// six-digit code out of reach of anything that tries them all. The wait
/// doubles each time a run of wrong codes replaces the code, up to
/// `MAX_COOLDOWN`, until a watch pairs or the switch is flipped; someone who
/// fumbled their own code just turns the switch off and on.
const FAILURE_COOLDOWN: Duration = Duration::from_secs(2);
const MAX_COOLDOWN: Duration = Duration::from_secs(10 * 60);
/// More watches than anyone wears; the oldest one is forgotten first.
const MAX_DEVICES: usize = 8;
const MAX_NAME: usize = 60;
/// How long to wait between tries for a port the listener being replaced
/// has not let go of yet, and for how long to keep trying.
const BIND_PAUSE: Duration = Duration::from_millis(100);
const BIND_RETRY: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Device {
    token: String,
    name: String,
    /// Unix seconds.
    paired_at: u64,
}

#[derive(Default)]
struct Inner {
    /// Bumped on every switch on or off. A listener whose generation is no
    /// longer the current one closes the port and ends.
    generation: u64,
    /// Shown in the tray while the port is open; good for one pairing.
    code: Option<String>,
    failures: u32,
    /// Codes replaced for wrong guesses since the last pairing or switch.
    strikes: u32,
    last_failure: Option<Instant>,
    devices: Vec<Device>,
    /// Why the port could not be opened, for the tray.
    error: Option<String>,
}

#[derive(Default)]
pub struct RemoteHub(Mutex<Inner>);

/// What the tray says about the watch.
pub struct Status {
    pub enabled: bool,
    /// `ip:port` to type into the watch.
    pub address: Option<String>,
    pub code: Option<String>,
    pub devices: usize,
    pub error: Option<String>,
}

pub fn status(app: &AppHandle) -> Status {
    let enabled = app.state::<SettingsState>().get().watch_enabled;
    let inner = app.state::<RemoteHub>();
    let inner = inner.0.lock().unwrap();
    Status {
        enabled,
        address: enabled
            .then(lan_address)
            .flatten()
            .map(|ip| format!("{ip}:{PORT}")),
        code: inner.code.clone(),
        devices: inner.devices.len(),
        error: inner.error.clone(),
    }
}

fn devices_path(app: &AppHandle) -> Option<PathBuf> {
    Some(app.path().app_config_dir().ok()?.join("watch-devices.json"))
}

fn save(app: &AppHandle, devices: &[Device]) {
    let Some(path) = devices_path(app) else { return };
    let written = path
        .parent()
        .map(fs::create_dir_all)
        .transpose()
        .and_then(|_| fs::write(&path, serde_json::to_vec_pretty(devices).unwrap_or_default()));
    if let Err(error) = written {
        eprintln!("[remote] failed to save to {}: {error}", path.display());
    }
    // The tokens in it are as good as the watch itself.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
}

pub fn start(app: AppHandle) {
    let devices: Vec<Device> = devices_path(&app)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    app.state::<RemoteHub>().0.lock().unwrap().devices = devices;
    if app.state::<SettingsState>().get().watch_enabled {
        set_enabled(&app, true);
    }
}

/// Opens or closes the port. Opening always shows a fresh code.
pub fn set_enabled(app: &AppHandle, enabled: bool) {
    let generation = {
        let hub = app.state::<RemoteHub>();
        let mut inner = hub.0.lock().unwrap();
        inner.generation += 1;
        inner.code = enabled.then(new_code);
        inner.failures = 0;
        inner.strikes = 0;
        inner.last_failure = None;
        inner.error = None;
        inner.generation
    };
    // The listener the switch just retired is blocked in accept; one
    // connection wakes it to find it is no longer current and close the port.
    let _ = TcpStream::connect_timeout(&(Ipv4Addr::LOCALHOST, PORT).into(), Duration::from_millis(300));
    if enabled {
        let app = app.clone();
        thread::spawn(move || listen(app, generation));
    }
}

/// Unpairs every watch; each one has to pair again with a new code.
pub fn forget_all(app: &AppHandle) {
    app.state::<RemoteHub>().0.lock().unwrap().devices.clear();
    save(app, &[]);
}

/// How long after a wrong code the next guess has to wait.
fn cooldown(strikes: u32) -> Duration {
    FAILURE_COOLDOWN.saturating_mul(1 << strikes.min(16)).min(MAX_COOLDOWN)
}

fn new_code() -> String {
    format!("{:06}", http::random_u64() % 1_000_000)
}

fn is_current(app: &AppHandle, generation: u64) -> bool {
    app.state::<RemoteHub>().0.lock().unwrap().generation == generation
}

fn listen(app: AppHandle, generation: u64) {
    let deadline = Instant::now() + BIND_RETRY;
    let listener = loop {
        match TcpListener::bind((Ipv4Addr::UNSPECIFIED, PORT)) {
            Ok(listener) => break listener,
            Err(_) if Instant::now() < deadline && is_current(&app, generation) => thread::sleep(BIND_PAUSE),
            Err(error) => {
                eprintln!("[remote] cannot open port {PORT}: {error}");
                let hub = app.state::<RemoteHub>();
                let mut inner = hub.0.lock().unwrap();
                if inner.generation == generation {
                    inner.error = Some(error.to_string());
                    drop(inner);
                    tray::refresh(&app);
                }
                return;
            }
        }
    };
    // Blocking, so a request is answered the moment it arrives; set_enabled
    // wakes a retired listener with a connection of its own.
    for stream in listener.incoming() {
        if !is_current(&app, generation) {
            break;
        }
        match stream {
            Ok(stream) => {
                let app = app.clone();
                thread::spawn(move || handle(&app, stream));
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            // Out of file descriptors and the like: give it a moment.
            Err(_) => thread::sleep(BIND_PAUSE),
        }
    }
}

fn handle(app: &AppHandle, mut stream: TcpStream) {
    let Some(request) = http::read_request(&mut stream, MAX_BODY) else {
        return http::error(&mut stream, "400 Bad Request", "bad request");
    };
    // The watch never sends one; a web page on the network always does.
    if request.header("origin").is_some() {
        return http::error(&mut stream, "403 Forbidden", "not for browsers");
    }
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/v1/hello") => {
            let body = json!({ "app": "bangs", "version": env!("CARGO_PKG_VERSION"), "host": host_name() });
            return reply(&mut stream, "200 OK", &body.to_string());
        }
        ("POST", "/v1/pair") => return pair(app, &mut stream, &request.body),
        _ => {}
    }
    let Some(token) = authorized(app, &request) else {
        return http::error(&mut stream, "401 Unauthorized", "pair with the code in the Bangs menu first");
    };
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/v1/state") => {
            let body = serde_json::to_string(&snapshot(app)).unwrap_or_default();
            reply(&mut stream, "200 OK", &body);
        }
        ("GET", "/v1/artwork") => {
            let picture = app
                .state::<MediaHub>()
                .current()
                .and_then(|media| media.artwork)
                .and_then(|url| decode_data_url(&url));
            match picture {
                Some((mime, bytes)) => http::reply_bytes(&mut stream, "200 OK", &mime, &bytes),
                None => http::error(&mut stream, "404 Not Found", "no artwork"),
            }
        }
        ("GET", "/v1/lyrics") => {
            let body = serde_json::to_string(&lyrics(app)).unwrap_or_default();
            reply(&mut stream, "200 OK", &body);
        }
        ("POST", "/v1/media") => {
            let Ok(command) = serde_json::from_str::<MediaCommand>(&request.body) else {
                return http::error(&mut stream, "400 Bad Request", "body is not a media command");
            };
            match media::command(app, command) {
                Ok(()) => reply(&mut stream, "200 OK", r#"{"ok":true}"#),
                Err(error) => http::error(&mut stream, "503 Service Unavailable", &error),
            }
        }
        ("POST", "/v1/todos") => {
            #[derive(Deserialize)]
            struct NewTodo {
                text: String,
            }
            let Ok(todo) = serde_json::from_str::<NewTodo>(&request.body) else {
                return http::error(&mut stream, "400 Bad Request", "body needs a text");
            };
            // The notch says so instead of taking another; so does the watch.
            if todos::is_full(app) {
                return http::error(&mut stream, "409 Conflict", "the list is full");
            }
            todos::todo_add(app.clone(), todo.text);
            reply(&mut stream, "200 OK", r#"{"ok":true}"#);
        }
        // Ticked off, as on the notch: done, and off the watch's list.
        ("POST", path) if done_todo(path).is_some() => {
            todos::todo_done(app, done_todo(path).unwrap_or_default());
            reply(&mut stream, "200 OK", r#"{"ok":true}"#);
        }
        // The watch signing itself out.
        ("DELETE", "/v1/pair") => {
            let devices = {
                let hub = app.state::<RemoteHub>();
                let mut inner = hub.0.lock().unwrap();
                inner.devices.retain(|device| !http::same_secret(&device.token, &token));
                inner.devices.clone()
            };
            save(app, &devices);
            tray::refresh(app);
            reply(&mut stream, "200 OK", r#"{"ok":true}"#);
        }
        _ => http::error(&mut stream, "404 Not Found", "unknown endpoint"),
    }
}

/// The id in `/v1/todos/<id>/done`.
fn done_todo(path: &str) -> Option<&str> {
    path.strip_prefix("/v1/todos/")?.strip_suffix("/done").filter(|id| !id.is_empty() && !id.contains('/'))
}

/// The bearer token, when it belongs to a paired watch.
fn authorized(app: &AppHandle, request: &http::Request) -> Option<String> {
    let token = request.header("authorization")?.strip_prefix("Bearer ")?.trim();
    let hub = app.state::<RemoteHub>();
    let inner = hub.0.lock().unwrap();
    inner
        .devices
        .iter()
        .any(|device| http::same_secret(&device.token, token))
        .then(|| token.to_string())
}

fn pair(app: &AppHandle, stream: &mut TcpStream, body: &str) {
    #[derive(Deserialize)]
    struct Pairing {
        code: String,
        #[serde(default)]
        name: String,
    }
    let Ok(pairing) = serde_json::from_str::<Pairing>(body) else {
        return http::error(stream, "400 Bad Request", "body needs a code");
    };
    let attempt: String = pairing.code.chars().filter(char::is_ascii_digit).collect();
    let name: String = pairing.name.trim().chars().take(MAX_NAME).collect();

    enum Outcome {
        Off,
        TooSoon,
        Wrong { replaced: bool },
        Paired { token: String, devices: Vec<Device> },
    }
    let outcome = {
        let hub = app.state::<RemoteHub>();
        let mut inner = hub.0.lock().unwrap();
        match inner.code.clone() {
            None => Outcome::Off,
            Some(_) if inner.last_failure.is_some_and(|at| at.elapsed() < cooldown(inner.strikes)) => Outcome::TooSoon,
            Some(code) if !http::same_secret(&code, &attempt) => {
                inner.failures += 1;
                inner.last_failure = Some(Instant::now());
                let replaced = inner.failures >= MAX_FAILURES;
                if replaced {
                    inner.code = Some(new_code());
                    inner.failures = 0;
                    inner.strikes += 1;
                }
                Outcome::Wrong { replaced }
            }
            Some(_) => {
                let token = http::random_token();
                inner.devices.insert(
                    0,
                    Device {
                        token: token.clone(),
                        name: if name.is_empty() { "Apple Watch".into() } else { name },
                        paired_at: now_ms() / 1000,
                    },
                );
                inner.devices.truncate(MAX_DEVICES);
                // Single use: whoever reads the code over a shoulder later
                // gets a code that no longer works.
                inner.code = Some(new_code());
                inner.failures = 0;
                inner.strikes = 0;
                inner.last_failure = None;
                Outcome::Paired { token, devices: inner.devices.clone() }
            }
        }
    };
    match outcome {
        Outcome::Off => http::error(stream, "403 Forbidden", "pairing is off"),
        Outcome::TooSoon => http::error(stream, "429 Too Many Requests", "wait a moment and try again"),
        Outcome::Wrong { replaced } => {
            if replaced {
                tray::refresh(app);
            }
            http::error(stream, "401 Unauthorized", "wrong code")
        }
        Outcome::Paired { token, devices } => {
            save(app, &devices);
            tray::refresh(app);
            reply(stream, "200 OK", &json!({ "token": token, "host": host_name() }).to_string());
        }
    }
}

// MARK: - What the watch sees

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    /// The computer's name, for the watch's header.
    host: String,
    /// Unix milliseconds on this machine, so the watch can correct the
    /// playback position for the difference between the two clocks.
    now: f64,
    media: Option<RemoteMedia>,
    sessions: Vec<RemoteSession>,
    /// The open lines; what is done stays on the notch until it is cleared.
    todos: Vec<Todo>,
    activities: Vec<RemoteActivity>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoteMedia {
    title: String,
    artist: String,
    album: String,
    app_name: String,
    playing: bool,
    duration: Option<f64>,
    elapsed: Option<f64>,
    elapsed_at: f64,
    /// Changes whenever the picture does; fetch `/v1/artwork` when it does.
    artwork: Option<String>,
    /// Changes whenever the lines do; fetch `/v1/lyrics` when it does.
    lyrics: Option<String>,
}

/// A session without its path: the project name is enough on a wrist, and
/// the id is a fingerprint, since a Codex session's id is its file's path.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoteSession {
    id: String,
    agent: Agent,
    name: String,
    project: String,
    status: SessionStatus,
    detail: Option<String>,
    updated_at: f64,
}

/// A board row without its link, which would only open on the computer.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoteActivity {
    id: String,
    title: String,
    subtitle: Option<String>,
    icon: Option<String>,
    progress: Option<f64>,
    updated_at: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoteLyrics {
    lines: Vec<RemoteLyricLine>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoteLyricLine {
    at: f64,
    text: String,
    translation: Option<String>,
}

fn snapshot(app: &AppHandle) -> Snapshot {
    let media = app.state::<MediaHub>().current().map(|media| {
        let lyrics = lyrics(app);
        RemoteMedia {
            artwork: media.artwork.as_deref().map(fingerprint),
            lyrics: (!lyrics.lines.is_empty())
                .then(|| fingerprint(&serde_json::to_string(&lyrics).unwrap_or_default())),
            title: media.title,
            artist: media.artist,
            album: media.album,
            app_name: media.app_name,
            playing: media.playing,
            duration: media.duration,
            elapsed: media.elapsed,
            elapsed_at: media.elapsed_at,
        }
    });
    let sessions = app
        .state::<DevHub>()
        .current()
        .sessions
        .into_iter()
        .map(|session| RemoteSession {
            id: fingerprint(&session.id),
            agent: session.agent,
            name: session.name,
            project: session.project,
            status: session.status,
            detail: session.detail,
            updated_at: session.updated_at,
        })
        .collect();
    let activities = app
        .state::<ActivityHub>()
        .current()
        .into_iter()
        .map(|activity| RemoteActivity {
            id: activity.id,
            title: activity.title,
            subtitle: activity.subtitle,
            icon: activity.icon,
            progress: activity.progress,
            updated_at: activity.updated_at,
        })
        .collect();
    Snapshot {
        host: host_name(),
        now: now_ms() as f64,
        media,
        sessions,
        todos: app.state::<TodoHub>().current().into_iter().filter(|todo| !todo.done).collect(),
        activities,
    }
}

/// Timed lines for the track that is playing now, and nothing for one that
/// has moved on or for lines that have no timestamps to follow.
fn lyrics(app: &AppHandle) -> RemoteLyrics {
    let lyrics = app.state::<LyricsHub>().current();
    let playing = app.state::<MediaHub>().current().map(|media| media::track_identity(&media));
    let usable = lyrics.timed
        && matches!(lyrics.status, LyricStatus::Found | LyricStatus::Uncertain)
        && playing.as_deref() == Some(lyrics.track.as_str());
    let translations = app.state::<SettingsState>().get().lyrics_translation_enabled;
    RemoteLyrics {
        lines: if usable {
            lyrics
                .lines
                .into_iter()
                .map(|line| RemoteLyricLine {
                    at: line.at,
                    text: line.text,
                    translation: line.translation.filter(|_| translations),
                })
                .collect()
        } else {
            Vec::new()
        },
    }
}

fn fingerprint(text: &str) -> String {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// `data:image/png;base64,…` → the type and the bytes.
fn decode_data_url(url: &str) -> Option<(String, Vec<u8>)> {
    let (meta, data) = url.strip_prefix("data:")?.split_once(',')?;
    let mime = meta.strip_suffix(";base64")?;
    if !mime.starts_with("image/") {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).ok()?;
    Some((mime.to_string(), bytes))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default()
}

/// The name the computer goes by in Finder or Explorer.
fn host_name() -> String {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        #[cfg(target_os = "macos")]
        if let Ok(output) = std::process::Command::new("scutil").args(["--get", "ComputerName"]).output() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !name.is_empty() {
                return name;
            }
        }
        std::env::var("COMPUTERNAME")
            .ok()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Bangs".into())
    })
    .clone()
}

/// The address other devices on the network reach this machine at.
fn lan_address() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    // Connecting a UDP socket sends nothing: it only asks the OS which
    // interface it would route through.
    socket.connect((Ipv4Addr::new(1, 1, 1, 1), 80)).ok()?;
    match socket.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_urls_decode_to_pictures() {
        let (mime, bytes) = decode_data_url("data:image/png;base64,iVBORw==").unwrap();
        assert_eq!(mime, "image/png");
        assert_eq!(bytes, vec![0x89, b'P', b'N', b'G']);
        assert!(decode_data_url("data:text/html;base64,PGI+").is_none());
        assert!(decode_data_url("https://example.com/a.png").is_none());
    }

    #[test]
    fn the_wait_after_wrong_codes_grows_and_stops_growing() {
        assert_eq!(cooldown(0), FAILURE_COOLDOWN);
        assert_eq!(cooldown(1), FAILURE_COOLDOWN * 2);
        assert_eq!(cooldown(3), FAILURE_COOLDOWN * 8);
        assert_eq!(cooldown(9), MAX_COOLDOWN);
        assert_eq!(cooldown(u32::MAX), MAX_COOLDOWN);
    }

    #[test]
    fn done_paths_name_one_todo() {
        assert_eq!(done_todo("/v1/todos/18f-2/done"), Some("18f-2"));
        assert_eq!(done_todo("/v1/todos/done"), None);
        assert_eq!(done_todo("/v1/todos//done"), None);
        assert_eq!(done_todo("/v1/todos/a/b/done"), None);
        assert_eq!(done_todo("/v1/todos/a"), None);
    }

    #[test]
    fn codes_are_six_digits() {
        for _ in 0..50 {
            let code = new_code();
            assert_eq!(code.len(), 6);
            assert!(code.chars().all(|c| c.is_ascii_digit()));
        }
    }
}
