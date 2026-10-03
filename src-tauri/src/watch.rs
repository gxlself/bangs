//! The watch app (`watch/` in this repo), reaching Bangs over the local network.
//!
//! Off until the tray turns it on. Then Bangs listens on `PORT` on every
//! interface and the tray shows this machine's address and a six digit pairing
//! code. The watch trades the code for a long-lived token once; every other
//! request carries that token. A code survives `MAX_ATTEMPTS` wrong guesses and
//! one successful pairing, then a new one is drawn, so guessing it from
//! somewhere else on the network gets nowhere.
//!
//! The watch only sees what its screens need — the track and its timed lyric
//! lines, coding sessions by project name, the to-do list — and can only do
//! what those screens do: play, pause and skip, add and tick off a to-do.
//! Never the clipboard, the shelf, file paths or the board.
//!
//! A watch signed in to the same Apple ID as this Mac does not need the code:
//! a build that carries the iCloud entitlement also writes the address and a
//! token of its own into the user's private CloudKit database (see
//! `native/macos/icloud_bridge.m` and watch/README.md), where the watch finds
//! it. The code stays for everything else — Windows, or iCloud turned off.

use std::collections::hash_map::{DefaultHasher, RandomState};
use std::fs;
use std::hash::{BuildHasher, Hash, Hasher};
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::dev::{Agent, DevHub, SessionStatus};
use crate::http::{read_request, reply, reply_bytes, Request};
use crate::lyrics::{self, LyricsHub};
use crate::media::{self, MediaCommand, MediaHub, MediaState};
use crate::settings::SettingsState;
use crate::todos::{self, Todo, TodoHub};

/// One past the board API's loopback port, so the two are easy to tell apart.
pub const PORT: u16 = 17651;
/// Wrong codes a pairing code survives before it is replaced.
const MAX_ATTEMPTS: u32 = 5;
/// Paired watches remembered at once; the oldest is forgotten first.
const MAX_TOKENS: usize = 8;
/// How long `GET /watch/state?since=` holds on to a request with nothing new.
/// Short enough that a phone or router in between does not give up first.
const LONG_POLL: Duration = Duration::from_secs(20);
const POLL_STEP: Duration = Duration::from_millis(250);
const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// How often the listener looks at the switch while nobody is connecting.
const IDLE_STEP: Duration = Duration::from_millis(100);
/// A song has a few hundred lines at most; this only stops a broken source.
const MAX_LYRICS: usize = 400;
const MAX_SESSIONS: usize = 12;
/// The CloudKit container both the Mac and the watch app are entitled to.
#[cfg(target_os = "macos")]
const ICLOUD_CONTAINER: &str = "iCloud.com.gxlself.bangs";
/// How often the iCloud record is compared with what it should say; it is
/// only written when that changed (a new address, a new token).
#[cfg(target_os = "macos")]
const ICLOUD_STEP: Duration = Duration::from_secs(5);

// MARK: - What the watch sees

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct WatchMedia {
    title: String,
    artist: String,
    app_name: String,
    playing: bool,
    duration: Option<f64>,
    elapsed: Option<f64>,
    /// Unix milliseconds, on this machine's clock; `serverTime` lets the watch
    /// correct for its own.
    elapsed_at: f64,
    /// Changes with the picture. The watch fetches `GET /watch/artwork` only
    /// when it does, rather than carrying the image in every state.
    artwork_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct WatchLyric {
    /// Seconds into the track.
    at: f64,
    text: String,
    translation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct WatchSession {
    id: String,
    agent: Agent,
    project: String,
    name: String,
    status: SessionStatus,
    detail: Option<String>,
    /// Unix milliseconds of the last status change.
    updated_at: f64,
}

/// Everything on the watch's screens. Its hash is the `rev` the watch sends
/// back to wait for the next change.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    media: Option<WatchMedia>,
    /// Only timed lines that belong to the track playing; empty otherwise.
    lyrics: Vec<WatchLyric>,
    sessions: Vec<WatchSession>,
    todos: Vec<Todo>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Envelope<'a> {
    rev: String,
    host: &'a str,
    /// Unix milliseconds when this was sent.
    server_time: f64,
    #[serde(flatten)]
    snapshot: &'a Snapshot,
}

fn snapshot(app: &AppHandle) -> Snapshot {
    let media = app.state::<MediaHub>().current();
    let lyrics = app.state::<LyricsHub>().current();
    let lines = match &media {
        Some(media) if lyrics.timed && lyrics.track == lyrics::track_key(media) => lyrics
            .lines
            .iter()
            .take(MAX_LYRICS)
            .map(|line| WatchLyric { at: line.at, text: line.text.clone(), translation: line.translation.clone() })
            .collect(),
        _ => Vec::new(),
    };
    let sessions = app
        .state::<DevHub>()
        .current()
        .sessions
        .into_iter()
        .take(MAX_SESSIONS)
        .map(|session| WatchSession {
            id: session.id,
            agent: session.agent,
            project: session.project,
            name: session.name,
            status: session.status,
            detail: session.detail,
            updated_at: session.updated_at,
        })
        .collect();
    Snapshot {
        media: media.as_ref().map(watch_media),
        lyrics: lines,
        sessions,
        todos: app.state::<TodoHub>().current(),
    }
}

fn watch_media(media: &MediaState) -> WatchMedia {
    WatchMedia {
        title: media.title.clone(),
        artist: media.artist.clone(),
        app_name: media.app_name.clone(),
        playing: media.playing,
        duration: media.duration,
        elapsed: media.elapsed,
        elapsed_at: media.elapsed_at,
        artwork_id: media.artwork.as_deref().map(hash),
    }
}

fn revision(snapshot: &Snapshot) -> String {
    hash(&serde_json::to_string(snapshot).unwrap_or_default())
}

/// Stable within a run, which is all `rev` and `artworkId` need.
fn hash(text: &str) -> String {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Splits a `data:image/png;base64,…` URL into its type and bytes.
fn decode_data_url(url: &str) -> Option<(String, Vec<u8>)> {
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let content_type = meta.strip_suffix(";base64")?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).ok()?;
    Some((if content_type.is_empty() { "application/octet-stream".into() } else { content_type.to_string() }, bytes))
}

// MARK: - Pairing

/// The switch, the pairing code and the tokens of paired watches.
pub struct WatchHub {
    enabled: AtomicBool,
    pairing: Mutex<Pairing>,
    tokens: Mutex<Vec<String>>,
    /// The token published to iCloud, kept apart from paired watches': every
    /// watch on the Apple ID shares it, and it is not counted or trimmed.
    icloud_token: Mutex<Option<String>>,
    /// This build can write to iCloud, so same-account watches pair on their own.
    icloud_ready: AtomicBool,
}

struct Pairing {
    code: String,
    failures: u32,
}

impl Default for WatchHub {
    fn default() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            pairing: Mutex::new(Pairing { code: new_code(), failures: 0 }),
            tokens: Mutex::new(Vec::new()),
            icloud_token: Mutex::new(None),
            icloud_ready: AtomicBool::new(false),
        }
    }
}

impl WatchHub {
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn code(&self) -> String {
        self.pairing.lock().unwrap().code.clone()
    }

    pub fn paired(&self) -> usize {
        self.tokens.lock().unwrap().len()
    }

    pub fn icloud_ready(&self) -> bool {
        self.icloud_ready.load(Ordering::Relaxed)
    }

    /// Draws a code nobody has seen yet.
    pub fn renew_code(&self) {
        *self.pairing.lock().unwrap() = Pairing { code: new_code(), failures: 0 };
    }

    /// Trades `code` for a fresh token. Either way the attempt is counted
    /// against the code, which is replaced once it pairs or runs out.
    fn pair(&self, code: &str) -> Result<String, PairError> {
        let mut pairing = self.pairing.lock().unwrap();
        if !same(code.trim(), &pairing.code) {
            pairing.failures += 1;
            if pairing.failures >= MAX_ATTEMPTS {
                *pairing = Pairing { code: new_code(), failures: 0 };
                return Err(PairError::Renewed);
            }
            return Err(PairError::Wrong);
        }
        *pairing = Pairing { code: new_code(), failures: 0 };
        drop(pairing);
        let token = random_hex(4);
        let mut tokens = self.tokens.lock().unwrap();
        tokens.push(token.clone());
        let excess = tokens.len().saturating_sub(MAX_TOKENS);
        tokens.drain(..excess);
        Ok(token)
    }

    fn authorized(&self, request: &Request) -> Option<String> {
        let token = request.authorization.as_deref()?.strip_prefix("Bearer ")?.trim();
        if self.icloud_token.lock().unwrap().as_deref().is_some_and(|known| same(known, token)) {
            return Some(token.to_string());
        }
        self.tokens.lock().unwrap().iter().find(|known| same(known, token)).cloned()
    }
}

#[derive(Debug, PartialEq)]
enum PairError {
    Wrong,
    /// Too many wrong codes: the tray now shows another one.
    Renewed,
}

/// Compares without stopping at the first difference, so timing does not
/// give away how much of a guess was right.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

/// RandomState seeds itself from the OS, which is the entropy std exposes
/// without pulling in a crate (activities.rs makes its token the same way).
fn random_u64() -> u64 {
    RandomState::new().build_hasher().finish()
}

fn random_hex(words: usize) -> String {
    (0..words).map(|_| format!("{:016x}", random_u64())).collect()
}

fn new_code() -> String {
    format!("{:06}", random_u64() % 1_000_000)
}

fn tokens_path(app: &AppHandle) -> Option<PathBuf> {
    Some(app.path().app_config_dir().ok()?.join("watch-tokens.json"))
}

fn icloud_token_path(app: &AppHandle) -> Option<PathBuf> {
    Some(app.path().app_config_dir().ok()?.join("watch-icloud-token"))
}

fn save_tokens(app: &AppHandle) {
    let Some(path) = tokens_path(app) else { return };
    let tokens = app.state::<WatchHub>().tokens.lock().unwrap().clone();
    let written = path
        .parent()
        .map(fs::create_dir_all)
        .transpose()
        .and_then(|_| fs::write(&path, serde_json::to_vec(&tokens).unwrap_or_default()));
    if let Err(error) = written {
        eprintln!("[watch] failed to save {}: {error}", path.display());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
}

/// Forgets every paired watch; each has to pair again with a new code.
pub fn unpair_all(app: &AppHandle) {
    let hub = app.state::<WatchHub>();
    hub.tokens.lock().unwrap().clear();
    // Same-account watches lose their token too; the next iCloud record
    // carries a new one, so they come back on their own.
    *hub.icloud_token.lock().unwrap() = None;
    if let Some(path) = icloud_token_path(app) {
        let _ = fs::remove_file(path);
    }
    hub.renew_code();
    save_tokens(app);
}

/// The token for same-account watches, made the first time it is needed.
#[cfg(target_os = "macos")]
fn icloud_token(app: &AppHandle) -> String {
    let hub = app.state::<WatchHub>();
    let mut token = hub.icloud_token.lock().unwrap();
    if let Some(existing) = token.as_ref() {
        return existing.clone();
    }
    let fresh = random_hex(4);
    if let Some(path) = icloud_token_path(app) {
        if fs::write(&path, &fresh).is_ok() {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        }
    }
    *token = Some(fresh.clone());
    fresh
}

/// Flips the switch the listener watches. The setting itself is saved by the
/// caller.
pub fn set_enabled(app: &AppHandle, enabled: bool) {
    let hub = app.state::<WatchHub>();
    if enabled && !hub.enabled() {
        // A code that sat in the tray while the port was closed may have
        // been seen; start every session with a new one.
        hub.renew_code();
    }
    hub.enabled.store(enabled, Ordering::Relaxed);
}

/// This machine's address on the network the default route leads to, for
/// the tray to show. Connecting a UDP socket sends nothing; it only asks the
/// OS which interface it would use.
pub fn lan_address() -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(8, 8, 8, 8), 80)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

fn host_name() -> String {
    sysinfo::System::host_name().unwrap_or_else(|| "Bangs".into())
}

// MARK: - The listener

pub fn start(app: AppHandle) {
    if let Some(tokens) = tokens_path(&app)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
    {
        *app.state::<WatchHub>().tokens.lock().unwrap() = tokens;
    }
    if let Some(token) = icloud_token_path(&app)
        .and_then(|path| fs::read_to_string(path).ok())
        .map(|raw| raw.trim().to_string())
        .filter(|token| !token.is_empty())
    {
        *app.state::<WatchHub>().icloud_token.lock().unwrap() = Some(token);
    }
    let enabled = app.state::<SettingsState>().get().watch_enabled;
    app.state::<WatchHub>().enabled.store(enabled, Ordering::Relaxed);
    #[cfg(target_os = "macos")]
    icloud::start(app.clone());

    // One thread for the life of the app: it holds the port while the switch
    // is on and lets go of it as soon as the switch is off.
    thread::spawn(move || {
        let host = host_name();
        let mut listener: Option<TcpListener> = None;
        let mut warned = false;
        loop {
            if !app.state::<WatchHub>().enabled() {
                listener = None;
                thread::sleep(IDLE_STEP * 3);
                continue;
            }
            let Some(open) = listener.as_ref() else {
                match TcpListener::bind((Ipv4Addr::UNSPECIFIED, PORT)).and_then(|bound| {
                    bound.set_nonblocking(true)?;
                    Ok(bound)
                }) {
                    Ok(bound) => {
                        // The tray shows the same; this is for `pnpm tauri dev`.
                        #[cfg(debug_assertions)]
                        eprintln!("[watch] listening on port {PORT}, pairing code {}", app.state::<WatchHub>().code());
                        listener = Some(bound);
                        warned = false;
                    }
                    Err(error) => {
                        if !warned {
                            eprintln!("[watch] cannot listen on port {PORT}: {error}");
                            warned = true;
                        }
                        thread::sleep(Duration::from_secs(3));
                    }
                }
                continue;
            };
            match open.accept() {
                Ok((stream, _)) => {
                    let app = app.clone();
                    let host = host.clone();
                    thread::spawn(move || handle(&app, &host, stream));
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => thread::sleep(IDLE_STEP),
                Err(_) => thread::sleep(IDLE_STEP),
            }
        }
    });
}

#[derive(Deserialize)]
struct PairBody {
    code: String,
}

#[derive(Deserialize)]
struct TodoBody {
    text: String,
}

fn handle(app: &AppHandle, host: &str, mut stream: TcpStream) {
    // Accepted from a non-blocking listener; the request itself blocks.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let Some(request) = read_request(&mut stream) else {
        return reply(&mut stream, "400 Bad Request", r#"{"error":"bad request"}"#);
    };
    // Anything on the network can open a web page that posts here; nothing
    // a browser sends has any business with the watch API.
    if request.origin.is_some() {
        return reply(&mut stream, "403 Forbidden", r#"{"error":"not for browsers"}"#);
    }
    let hub = app.state::<WatchHub>();
    if !hub.enabled() {
        return reply(&mut stream, "503 Service Unavailable", r#"{"error":"watch access is off"}"#);
    }

    if (request.method.as_str(), request.route()) == ("POST", "/watch/pair") {
        let Ok(body) = serde_json::from_str::<PairBody>(&request.body) else {
            return reply(&mut stream, "400 Bad Request", r#"{"error":"send {\"code\":\"123456\"}"}"#);
        };
        let result = hub.pair(&body.code);
        // The code in the tray changed either way.
        crate::tray::refresh(app);
        return match result {
            Ok(token) => {
                save_tokens(app);
                let body = serde_json::json!({ "token": token, "host": host });
                reply(&mut stream, "200 OK", &body.to_string())
            }
            Err(PairError::Wrong) => reply(&mut stream, "403 Forbidden", r#"{"error":"wrong code"}"#),
            Err(PairError::Renewed) => {
                reply(&mut stream, "403 Forbidden", r#"{"error":"too many wrong codes; the tray shows a new one"}"#)
            }
        };
    }

    let Some(token) = hub.authorized(&request) else {
        return reply(&mut stream, "401 Unauthorized", r#"{"error":"pair first"}"#);
    };

    match (request.method.as_str(), request.route()) {
        ("GET", "/watch/state") => {
            let since = request.query("since").map(str::to_string);
            let deadline = Instant::now() + LONG_POLL;
            let (snapshot, rev) = loop {
                let snapshot = snapshot(app);
                let rev = revision(&snapshot);
                if since.as_deref() != Some(rev.as_str()) || Instant::now() >= deadline || !hub.enabled() {
                    break (snapshot, rev);
                }
                thread::sleep(POLL_STEP);
            };
            let envelope = Envelope { rev, host, server_time: now_ms(), snapshot: &snapshot };
            reply(&mut stream, "200 OK", &serde_json::to_string(&envelope).unwrap_or_default());
        }
        ("GET", "/watch/artwork") => {
            match app.state::<MediaHub>().current().and_then(|media| media.artwork).as_deref().and_then(decode_data_url) {
                Some((content_type, bytes)) => reply_bytes(&mut stream, "200 OK", &content_type, &bytes),
                None => reply(&mut stream, "404 Not Found", r#"{"error":"no artwork"}"#),
            }
        }
        ("POST", "/watch/media") => {
            let Ok(command) = serde_json::from_str::<MediaCommand>(&request.body) else {
                return reply(&mut stream, "400 Bad Request", r#"{"error":"send {\"action\":\"toggle\"}"}"#);
            };
            match media::command(app, command) {
                Ok(()) => reply(&mut stream, "200 OK", r#"{"ok":true}"#),
                Err(error) => reply(&mut stream, "409 Conflict", &serde_json::json!({ "error": error }).to_string()),
            }
        }
        ("POST", "/watch/todos") => {
            let Ok(body) = serde_json::from_str::<TodoBody>(&request.body) else {
                return reply(&mut stream, "400 Bad Request", r#"{"error":"send {\"text\":\"…\"}"}"#);
            };
            todos::todo_add(app.clone(), body.text);
            reply(&mut stream, "200 OK", r#"{"ok":true}"#);
        }
        ("DELETE", route) if route.starts_with("/watch/todos/") => {
            let id = route.trim_start_matches("/watch/todos/").to_string();
            todos::todo_remove(app.clone(), id);
            reply(&mut stream, "200 OK", r#"{"ok":true}"#);
        }
        ("DELETE", "/watch/pair") => {
            hub.tokens.lock().unwrap().retain(|known| *known != token);
            save_tokens(app);
            crate::tray::refresh(app);
            reply(&mut stream, "200 OK", r#"{"ok":true}"#);
        }
        _ => reply(&mut stream, "404 Not Found", r#"{"error":"not a watch route"}"#),
    }
}

// MARK: - iCloud

#[cfg(target_os = "macos")]
mod icloud {
    use std::ffi::{c_char, c_int, CString};
    use std::fs;
    use std::sync::atomic::Ordering;
    use std::thread;

    use serde::Serialize;
    use tauri::{AppHandle, Manager};

    use super::{host_name, icloud_token, lan_address, random_hex, WatchHub, ICLOUD_CONTAINER, ICLOUD_STEP, PORT};

    extern "C" {
        fn bangs_icloud_available(container: *const c_char) -> c_int;
        fn bangs_icloud_publish(container: *const c_char, record_name: *const c_char, payload: *const c_char);
        fn bangs_icloud_remove(container: *const c_char, record_name: *const c_char);
    }

    /// What the watch reads from the record (`MacRecord` in the watch app).
    #[derive(Serialize)]
    struct Payload<'a> {
        id: &'a str,
        host: &'a str,
        /// Tried in order: the LAN address, then the Bonjour name.
        addresses: Vec<String>,
        port: u16,
        token: &'a str,
    }

    /// Names this Mac's record, the same across launches.
    fn mac_id(app: &AppHandle) -> String {
        let path = app.path().app_config_dir().ok().map(|dir| dir.join("watch-mac-id"));
        if let Some(existing) = path
            .as_ref()
            .and_then(|path| fs::read_to_string(path).ok())
            .map(|raw| raw.trim().to_string())
            .filter(|id| !id.is_empty())
        {
            return existing;
        }
        let id = format!("mac-{}", random_hex(2));
        if let Some(path) = path {
            let _ = fs::write(path, &id);
        }
        id
    }

    pub fn start(app: AppHandle) {
        let container = CString::new(ICLOUD_CONTAINER).unwrap();
        // SAFETY: a valid C string that outlives the call.
        if unsafe { bangs_icloud_available(container.as_ptr()) } == 0 {
            eprintln!("[watch] no iCloud entitlement in this build; watches pair with the code");
            return;
        }
        app.state::<WatchHub>().icloud_ready.store(true, Ordering::Relaxed);
        thread::spawn(move || {
            let id = mac_id(&app);
            let record = CString::new(id.clone()).unwrap();
            let host = host_name();
            let bonjour = if host.ends_with(".local") { host.clone() } else { format!("{host}.local") };
            let mut published: Option<String> = None;
            loop {
                if app.state::<WatchHub>().enabled() {
                    let token = icloud_token(&app);
                    let mut addresses: Vec<String> = lan_address().map(|ip| ip.to_string()).into_iter().collect();
                    addresses.push(bonjour.clone());
                    let payload = Payload { id: &id, host: &host, addresses, port: PORT, token: &token };
                    let text = serde_json::to_string(&payload).unwrap_or_default();
                    if published.as_deref() != Some(text.as_str()) {
                        if let Ok(c_text) = CString::new(text.clone()) {
                            // SAFETY: valid C strings that outlive the call; the
                            // bridge copies them before returning.
                            unsafe { bangs_icloud_publish(container.as_ptr(), record.as_ptr(), c_text.as_ptr()) };
                        }
                        published = Some(text);
                    }
                } else if published.take().is_some() {
                    // SAFETY: as above.
                    unsafe { bangs_icloud_remove(container.as_ptr(), record.as_ptr()) };
                }
                thread::sleep(ICLOUD_STEP);
            }
        });
    }
}

fn now_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|since| since.as_millis() as f64).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_pairs_once_then_changes() {
        let hub = WatchHub::default();
        let code = hub.code();
        let token = hub.pair(&code).expect("token");
        assert_eq!(token.len(), 64);
        assert_ne!(hub.code(), code);
        assert_eq!(hub.pair(&code), Err(PairError::Wrong));
        assert_eq!(hub.paired(), 1);
    }

    #[test]
    fn too_many_wrong_codes_draw_a_new_one() {
        let hub = WatchHub::default();
        let code = hub.code();
        let wrong = if code == "000000" { "000001" } else { "000000" };
        for _ in 0..MAX_ATTEMPTS - 1 {
            assert_eq!(hub.pair(wrong), Err(PairError::Wrong));
        }
        assert_eq!(hub.pair(wrong), Err(PairError::Renewed));
        assert_eq!(hub.paired(), 0);
    }

    #[test]
    fn only_the_newest_watches_stay_paired() {
        let hub = WatchHub::default();
        let first = hub.pair(&hub.code()).unwrap();
        for _ in 0..MAX_TOKENS {
            hub.pair(&hub.code()).unwrap();
        }
        assert_eq!(hub.paired(), MAX_TOKENS);
        assert!(!hub.tokens.lock().unwrap().contains(&first));
    }

    #[test]
    fn codes_are_six_digits() {
        for _ in 0..50 {
            let code = new_code();
            assert_eq!(code.len(), 6);
            assert!(code.chars().all(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn data_urls_decode_to_type_and_bytes() {
        let (content_type, bytes) = decode_data_url("data:image/png;base64,iVBORw0K").unwrap();
        assert_eq!(content_type, "image/png");
        assert_eq!(&bytes[..4], b"\x89PNG");
        assert!(decode_data_url("https://example.com/a.png").is_none());
    }

    #[test]
    fn the_revision_follows_the_content() {
        let mut snapshot = Snapshot { media: None, lyrics: Vec::new(), sessions: Vec::new(), todos: Vec::new() };
        let before = revision(&snapshot);
        assert_eq!(before, revision(&snapshot));
        snapshot.todos.push(Todo { id: "1".into(), text: "buy milk".into(), created_at: 1 });
        assert_ne!(before, revision(&snapshot));
    }

    #[test]
    fn comparison_needs_the_whole_string() {
        assert!(same("123456", "123456"));
        assert!(!same("123456", "123457"));
        assert!(!same("12345", "123456"));
    }
}
