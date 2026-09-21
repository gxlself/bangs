//! Lyrics for whatever is playing. QQ Music and NetEase are queried together,
//! then the most reliable timed result is selected and cached on disk.

use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::media::MediaState;
use crate::settings::SettingsState;

mod qrc;

const SEARCH_URL: &str = "https://c.y.qq.com/soso/fcgi-bin/client_search_cp";
const LYRIC_URL: &str = "https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg";
/// Returns QRC, which carries a timing per character.
const QRC_URL: &str = "https://c.y.qq.com/qqmusic/fcgi-bin/lyric_download.fcg";
const REFERER: &str = "https://y.qq.com/portal/player.html";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(4);
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(8);
const CACHE_VERSION: u8 = 4;
const DAY: u64 = 24 * 60 * 60;
const GOOD_CACHE_TTL: u64 = 30 * DAY;
const PARTIAL_CACHE_TTL: u64 = 7 * DAY;
const MISS_CACHE_TTL: u64 = 6 * 60 * 60;
const ERROR_CACHE_TTL: u64 = 5 * 60;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricWord {
    /// Seconds into the track.
    pub at: f64,
    pub duration: f64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricLine {
    /// Seconds into the track.
    pub at: f64,
    pub text: String,
    pub translation: Option<String>,
    /// Per-character timing, when the source has it (QRC).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<LyricWord>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LyricStatus {
    #[default]
    Idle,
    Loading,
    Found,
    Uncertain,
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Provider {
    QqQrc,
    QqLrc,
    NeteaseYrc,
    NeteaseLrc,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricSource {
    pub original: Provider,
    pub translation: Option<Provider>,
    pub translation_offset: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lyrics {
    /// The track these lines belong to, so the webview can drop stale ones.
    pub track: String,
    /// Whether the accepted lines have real timestamps.
    #[serde(default)]
    pub timed: bool,
    #[serde(default)]
    pub word_timed: bool,
    #[serde(default)]
    pub status: LyricStatus,
    pub source: Option<LyricSource>,
    #[serde(default)]
    pub from_cache: bool,
    pub lines: Vec<LyricLine>,
}

#[derive(Default)]
pub struct LyricsHub {
    current: Mutex<Lyrics>,
    requests: Mutex<Option<Sender<Request>>>,
    generation: AtomicU64,
}

impl LyricsHub {
    pub fn current(&self) -> Lyrics {
        self.current.lock().unwrap().clone()
    }
}

#[derive(Clone)]
struct Request {
    track: String,
    title: String,
    artist: String,
    album: String,
    duration: Option<f64>,
    player: PlayerKind,
    generation: u64,
    force: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlayerKind {
    QqMusic,
    NetEase,
    Spotify,
    Other,
}

#[derive(Debug, Clone)]
struct FetchedLyrics {
    provider: Provider,
    meta: SongMeta,
    score: MatchScore,
    lines: Vec<LyricLine>,
    translations: Vec<LyricLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CacheFile {
    version: u8,
    created_at: u64,
    expires_at: u64,
    match_score: Option<i32>,
    lyrics: Lyrics,
}

#[derive(Debug, Clone)]
struct SongMeta {
    title: String,
    artists: Vec<String>,
    album: String,
    duration: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Confidence {
    Rejected,
    Uncertain,
    Reliable,
}

#[derive(Debug, Clone, Copy)]
struct MatchScore {
    total: i32,
    confidence: Confidence,
}

#[derive(Default)]
struct SourceBatch {
    candidates: Vec<FetchedLyrics>,
    responded: bool,
}

struct LookupOutcome {
    lyrics: Lyrics,
    match_score: Option<i32>,
    ttl: u64,
}

/// Stable song identity; dynamic source, duration and artwork do not belong in it.
pub fn track_key(media: &MediaState) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}",
        normalize(&media.title),
        normalize(&media.artist),
        normalize(&media.album),
    )
}

/// Called whenever the media state changes; fetches once per track.
pub fn sync(app: &AppHandle, media: Option<&MediaState>) {
    let hub = app.state::<LyricsHub>();
    let Some(media) = media.filter(|media| !media.title.is_empty()) else {
        hub.generation.fetch_add(1, Ordering::Relaxed);
        publish(app, Lyrics::default());
        return;
    };

    let track = track_key(media);
    if hub.current().track == track {
        return;
    }
    let generation = hub.generation.fetch_add(1, Ordering::Relaxed) + 1;
    // Clear immediately: the old lines belong to the previous song.
    publish(
        app,
        Lyrics {
            track: track.clone(),
            status: if app.state::<SettingsState>().get().lyrics_enabled {
                LyricStatus::Loading
            } else {
                LyricStatus::Idle
            },
            ..Lyrics::default()
        },
    );

    if !app.state::<SettingsState>().get().lyrics_enabled {
        return;
    }
    send_request(app, media, track, generation, false);
}

/// Applies a change to the lyrics setting: clear the lines, or look up the
/// track that is playing right now.
pub fn set_enabled(app: &AppHandle, enabled: bool) {
    let media = app.state::<crate::media::MediaHub>().current();
    publish(app, Lyrics::default());
    if enabled {
        sync(app, media.as_ref());
    }
}

#[tauri::command]
pub fn lyrics_refresh(app: AppHandle) -> Result<(), String> {
    if !app.state::<SettingsState>().get().lyrics_enabled {
        return Err("lyrics are disabled".into());
    }
    let media = app
        .state::<crate::media::MediaHub>()
        .current()
        .filter(|media| !media.title.is_empty())
        .ok_or_else(|| "nothing is playing".to_string())?;
    let track = track_key(&media);
    let current = app.state::<LyricsHub>().current();
    if current.track != track {
        sync(&app, Some(&media));
        return Ok(());
    }
    if current.status == LyricStatus::Loading {
        return Ok(());
    }
    let generation = app
        .state::<LyricsHub>()
        .generation
        .fetch_add(1, Ordering::Relaxed)
        + 1;
    publish(
        &app,
        Lyrics {
            status: LyricStatus::Loading,
            ..current
        },
    );
    send_request(&app, &media, track, generation, true);
    Ok(())
}

fn send_request(app: &AppHandle, media: &MediaState, track: String, generation: u64, force: bool) {
    let hub = app.state::<LyricsHub>();
    let request = Request {
        track,
        title: media.title.clone(),
        artist: media.artist.clone(),
        album: media.album.clone(),
        duration: media.duration,
        player: player_kind(media),
        generation,
        force,
    };
    let sender = hub.requests.lock().unwrap().clone();
    if let Some(sender) = sender {
        let _ = sender.send(request);
    }
}

fn publish(app: &AppHandle, lyrics: Lyrics) {
    {
        let hub = app.state::<LyricsHub>();
        let mut current = hub.current.lock().unwrap();
        if *current == lyrics {
            return;
        }
        *current = lyrics.clone();
    }
    let _ = app.emit("bangs://lyrics", lyrics);
}

pub fn start(app: AppHandle) {
    let (sender, requests) = mpsc::channel();
    *app.state::<LyricsHub>().requests.lock().unwrap() = Some(sender);
    thread::spawn(move || run(app, requests));
}

fn run(app: AppHandle, requests: Receiver<Request>) {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map(|dir| dir.join("lyrics"))
        .ok();
    if let Some(dir) = &cache_dir {
        let _ = fs::create_dir_all(dir);
    }

    while let Ok(first) = requests.recv() {
        let request = requests.try_iter().last().unwrap_or(first);
        let app = app.clone();
        let cache_dir = cache_dir.clone();
        thread::spawn(move || process_request(&app, request, &cache_dir));
    }
}

fn process_request(app: &AppHandle, request: Request, cache_dir: &Option<PathBuf>) {
    if !is_current(app, &request) {
        return;
    }
    let path = cache_dir
        .as_ref()
        .map(|dir| dir.join(cache_name(&request.track)));
    let cached = path
        .as_ref()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|raw| read_cache(&raw));
    let now = unix_seconds();
    if !request.force {
        if let Some(cache) = cached.as_ref().filter(|cache| cache.expires_at > now) {
            let mut lyrics = cache.lyrics.clone();
            lyrics.track = request.track.clone();
            lyrics.from_cache = true;
            if is_current(app, &request) {
                publish(app, lyrics);
            }
            return;
        }
    }

    let fallback = cached
        .as_ref()
        .map(|cache| cache.lyrics.clone())
        .or_else(|| {
            let current = app.state::<LyricsHub>().current();
            (current.track == request.track && !current.lines.is_empty()).then_some(current)
        });
    let outcome = fetch_all(&request);
    if !is_current(app, &request) {
        return;
    }

    if outcome.lyrics.status != LyricStatus::Found {
        if let Some(mut stale) = fallback.filter(|lyrics| !lyrics.lines.is_empty()) {
            stale.track = request.track;
            stale.status = LyricStatus::Found;
            stale.from_cache = true;
            publish(app, stale);
            return;
        }
    }

    let mut lyrics = outcome.lyrics;
    lyrics.track = request.track;
    if let Some(path) = path {
        write_cache(&path, &lyrics, outcome.match_score, outcome.ttl);
    }
    publish(app, lyrics);
}

fn is_current(app: &AppHandle, request: &Request) -> bool {
    let hub = app.state::<LyricsHub>();
    hub.generation.load(Ordering::Relaxed) == request.generation
        && hub.current().track == request.track
}

fn cache_name(track: &str) -> String {
    let mut hasher = DefaultHasher::new();
    track.hash(&mut hasher);
    format!("{:016x}.json", hasher.finish())
}

fn has_timing(lines: &[LyricLine]) -> bool {
    !lines.is_empty()
        && lines
            .iter()
            .all(|line| line.at.is_finite() && line.at >= 0.0)
}

fn read_cache(raw: &str) -> Option<CacheFile> {
    let cache: CacheFile = serde_json::from_str(raw).ok()?;
    if cache.version != CACHE_VERSION {
        return None;
    }
    Some(cache)
}

fn write_cache(path: &Path, lyrics: &Lyrics, match_score: Option<i32>, ttl: u64) {
    let created_at = unix_seconds();
    let cache = CacheFile {
        version: CACHE_VERSION,
        created_at,
        expires_at: created_at.saturating_add(ttl),
        match_score,
        lyrics: lyrics.clone(),
    };
    if let Ok(raw) = serde_json::to_string(&cache) {
        let _ = fs::write(path, raw);
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn client() -> Option<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent("Mozilla/5.0")
        .build()
        .ok()
}

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn player_kind(media: &MediaState) -> PlayerKind {
    let haystack = format!("{} {}", media.source_id, media.app_name).to_lowercase();
    if haystack.contains("spotify") {
        PlayerKind::Spotify
    } else if haystack.contains("qqmusic")
        || haystack.contains("qq 音乐")
        || haystack.contains("qq音乐")
    {
        PlayerKind::QqMusic
    } else if haystack.contains("cloudmusic")
        || haystack.contains("netease")
        || haystack.contains("网易云")
        || haystack.contains("163music")
    {
        PlayerKind::NetEase
    } else {
        PlayerKind::Other
    }
}

impl Provider {
    fn is_qq(self) -> bool {
        matches!(self, Provider::QqQrc | Provider::QqLrc)
    }
}

fn fetch_all(request: &Request) -> LookupOutcome {
    let Some(client) = client() else {
        return missing_outcome(false);
    };
    let (sender, receiver) = mpsc::channel();
    let qq_sender = sender.clone();
    let qq_client = client.clone();
    let qq_request = request.clone();
    thread::spawn(move || {
        let _ = qq_sender.send(fetch_qq_candidates(&qq_client, &qq_request));
    });
    let netease_client = client;
    let netease_request = request.clone();
    thread::spawn(move || {
        let _ = sender.send(fetch_netease_candidates(&netease_client, &netease_request));
    });

    let deadline = Instant::now() + LOOKUP_TIMEOUT;
    let mut batches = Vec::new();
    while batches.len() < 2 {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match receiver.recv_timeout(remaining) {
            Ok(batch) => batches.push(batch),
            Err(_) => break,
        }
    }
    let responded = batches.iter().any(|batch| batch.responded);
    let candidates = batches
        .into_iter()
        .flat_map(|batch| batch.candidates)
        .collect::<Vec<_>>();
    choose_result(candidates, request.player, responded)
}

/// Drops the suffixes players add to a title: "Song - Live", "Song (feat. X)".
/// A title that is all decoration — "(Don't Fear) The Reaper" — keeps its own
/// text, because an empty one matches every song in the results.
fn plain_title(title: &str) -> String {
    let plain = title.split(" - ").next().unwrap_or(title);
    let plain = plain.split(['(', '（', '[']).next().unwrap_or(plain).trim();
    if plain.is_empty() {
        title.trim().to_string()
    } else {
        plain.to_string()
    }
}

/// The lead artist; a search does worse with the whole billing.
fn first_artist(artist: &str) -> String {
    let lead = split_artists(artist).into_iter().next().unwrap_or_default();
    if lead.is_empty() {
        artist.trim().to_string()
    } else {
        lead
    }
}

fn fetch_qq_candidates(client: &reqwest::blocking::Client, request: &Request) -> SourceBatch {
    let mut batch = SourceBatch::default();
    let plain_title = plain_title(&request.title);
    let plain_artist = first_artist(&request.artist);
    let mut queries = vec![format!("{} {}", request.title, request.artist)];
    let plain = format!("{plain_title} {plain_artist}");
    if plain != queries[0] {
        queries.push(plain);
    }
    for query in queries {
        let before = batch.candidates.len();
        let Ok(response) = client
            .get(format!(
                "{SEARCH_URL}?w={}&format=json&n=20&p=1",
                encode(&query)
            ))
            .header("Referer", REFERER)
            .send()
        else {
            continue;
        };
        batch.responded = true;
        let Ok(body) = response.text() else { continue };
        let Ok(search) = serde_json::from_str::<serde_json::Value>(strip_jsonp(&body)) else {
            continue;
        };
        let Some(songs) = search["data"]["song"]["list"].as_array() else {
            continue;
        };
        let mut ranked = songs
            .iter()
            .filter_map(|song| {
                let meta = qq_meta(song);
                let score = match_score(request, &meta);
                (score.confidence != Confidence::Rejected).then_some((song, meta, score))
            })
            .collect::<Vec<_>>();
        ranked.sort_by_key(|(_, _, score)| std::cmp::Reverse(score.total));
        for (song, meta, score) in ranked.into_iter().take(3) {
            if let Some((lines, translations)) = fetch_qq_qrc_song(client, song) {
                batch.candidates.push(FetchedLyrics {
                    provider: Provider::QqQrc,
                    meta: meta.clone(),
                    score,
                    lines,
                    translations,
                });
            }
            if let Some((lines, translations)) = fetch_qq_lrc_song(client, song) {
                batch.candidates.push(FetchedLyrics {
                    provider: Provider::QqLrc,
                    meta,
                    score,
                    lines,
                    translations,
                });
            }
        }
        if batch.candidates.len() > before {
            break;
        }
    }
    batch
}

fn fetch_qq_qrc_song(
    client: &reqwest::blocking::Client,
    song: &serde_json::Value,
) -> Option<(Vec<LyricLine>, Vec<LyricLine>)> {
    let id = song["songid"].as_i64()?;
    fetch_qrc(client, id)
}

fn fetch_qq_lrc_song(
    client: &reqwest::blocking::Client,
    song: &serde_json::Value,
) -> Option<(Vec<LyricLine>, Vec<LyricLine>)> {
    let song_mid = song["songmid"].as_str()?.to_string();
    if song_mid.is_empty() {
        return None;
    }
    for attempt in 0..2 {
        if attempt > 0 {
            thread::sleep(Duration::from_millis(400));
        }
        let Some(payload) = lyric_payload(client, &song_mid) else {
            continue;
        };
        let lines = parse_lrc(payload["lyric"].as_str().unwrap_or_default());
        if !lines.is_empty() {
            return Some((
                lines,
                parse_lrc(payload["trans"].as_str().unwrap_or_default()),
            ));
        }
    }
    None
}

fn fetch_netease_candidates(client: &reqwest::blocking::Client, request: &Request) -> SourceBatch {
    let mut batch = SourceBatch::default();
    let query = format!(
        "{} {}",
        plain_title(&request.title),
        first_artist(&request.artist)
    );
    let Ok(response) = client
        .get(format!(
            "https://music.163.com/api/search/get/web?s={}&type=1&offset=0&total=true&limit=10",
            encode(&query)
        ))
        .header("Referer", "https://music.163.com/")
        .send()
    else {
        return batch;
    };
    batch.responded = true;
    let Ok(body) = response.text() else {
        return batch;
    };
    let Ok(search) = serde_json::from_str::<serde_json::Value>(&body) else {
        return batch;
    };
    let Some(songs) = search["result"]["songs"].as_array() else {
        return batch;
    };
    let mut ranked = songs
        .iter()
        .filter_map(|song| {
            let meta = netease_meta(song);
            let score = match_score(request, &meta);
            (score.confidence != Confidence::Rejected).then_some((song, meta, score))
        })
        .collect::<Vec<_>>();
    ranked.sort_by_key(|(_, _, score)| std::cmp::Reverse(score.total));
    for (song, meta, score) in ranked.into_iter().take(3) {
        let Some(id) = song["id"].as_i64() else {
            continue;
        };
        let Ok(response) = client
            .get(format!(
                "https://music.163.com/api/song/lyric?id={id}&lv=1&kv=1&tv=-1"
            ))
            .header("Referer", "https://music.163.com/")
            .send()
        else {
            continue;
        };
        let Ok(body) = response.text() else { continue };
        let Ok(payload) = serde_json::from_str::<serde_json::Value>(&body) else {
            continue;
        };
        let yrc = payload["yrc"]["lyric"]
            .as_str()
            .map(parse_yrc)
            .unwrap_or_default();
        let (provider, lines) = if yrc.is_empty() {
            (
                Provider::NeteaseLrc,
                parse_lrc(payload["lrc"]["lyric"].as_str().unwrap_or_default()),
            )
        } else {
            (Provider::NeteaseYrc, yrc)
        };
        if lines.is_empty() {
            continue;
        }
        batch.candidates.push(FetchedLyrics {
            provider,
            meta,
            score,
            lines,
            translations: parse_lrc(payload["tlyric"]["lyric"].as_str().unwrap_or_default()),
        });
    }
    batch
}

fn lyric_payload(client: &reqwest::blocking::Client, song_mid: &str) -> Option<serde_json::Value> {
    let raw = client
        .get(format!(
            "{LYRIC_URL}?songmid={}&format=json&nobase64=1&g_tk=5381",
            encode(song_mid)
        ))
        .header("Referer", REFERER)
        .send()
        .ok()?
        .text()
        .ok()?;
    serde_json::from_str(strip_jsonp(&raw)).ok()
}

fn normalize(value: &str) -> String {
    crate::platform::to_simplified(value)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

fn split_artists(artist: &str) -> Vec<String> {
    let replaced = artist
        .to_lowercase()
        .replace(" feat. ", "/")
        .replace(" feat ", "/")
        .replace(" featuring ", "/")
        .replace(" x ", "/");
    replaced
        .split([',', '，', '&', '/', ';', '；', '、', '×'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

const VERSION_TAGS: [&str; 7] = [
    "live",
    "remix",
    "acoustic",
    "instrumental",
    "remaster",
    "伴奏",
    "现场",
];

fn version_tags(title: &str) -> HashSet<&'static str> {
    let lower = title.to_lowercase();
    VERSION_TAGS
        .iter()
        .copied()
        .filter(|tag| lower.contains(tag))
        .collect()
}

fn qq_meta(song: &serde_json::Value) -> SongMeta {
    SongMeta {
        title: song["songname"].as_str().unwrap_or_default().to_string(),
        artists: song["singer"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        album: song["albumname"].as_str().unwrap_or_default().to_string(),
        duration: song["interval"].as_f64(),
    }
}

fn netease_meta(song: &serde_json::Value) -> SongMeta {
    SongMeta {
        title: song["name"].as_str().unwrap_or_default().to_string(),
        artists: song["artists"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        album: song["album"]["name"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        duration: song["duration"]
            .as_f64()
            .map(|milliseconds| milliseconds / 1000.0),
    }
}

fn match_score(request: &Request, candidate: &SongMeta) -> MatchScore {
    let wanted_title = normalize(&request.title);
    let actual_title = normalize(&candidate.title);
    let wanted_base = normalize(&plain_title(&request.title));
    let actual_base = normalize(&plain_title(&candidate.title));
    let title = if !wanted_title.is_empty() && wanted_title == actual_title {
        50
    } else if !wanted_base.is_empty() && wanted_base == actual_base {
        42
    } else if !wanted_base.is_empty()
        && !actual_base.is_empty()
        && (wanted_base.contains(&actual_base) || actual_base.contains(&wanted_base))
    {
        25
    } else {
        0
    };

    let wanted_artists = split_artists(&request.artist)
        .into_iter()
        .map(|artist| normalize(&artist))
        .filter(|artist| !artist.is_empty())
        .collect::<Vec<_>>();
    let actual_artists = candidate
        .artists
        .iter()
        .map(|artist| normalize(artist))
        .filter(|artist| !artist.is_empty())
        .collect::<Vec<_>>();
    let artist = if wanted_artists
        .iter()
        .any(|wanted| actual_artists.contains(wanted))
    {
        25
    } else if wanted_artists.iter().any(|wanted| {
        actual_artists
            .iter()
            .any(|actual| wanted.contains(actual) || actual.contains(wanted))
    }) {
        15
    } else {
        0
    };

    let wanted_album = normalize(&request.album);
    let actual_album = normalize(&candidate.album);
    let album = if !wanted_album.is_empty() && wanted_album == actual_album {
        10
    } else if !wanted_album.is_empty()
        && !actual_album.is_empty()
        && (wanted_album.contains(&actual_album) || actual_album.contains(&wanted_album))
    {
        5
    } else {
        0
    };
    let duration = match (request.duration, candidate.duration) {
        (Some(wanted), Some(actual)) if (wanted - actual).abs() <= 3.0 => 10,
        (Some(wanted), Some(actual)) if (wanted - actual).abs() <= 8.0 => 5,
        _ => 0,
    };
    let wanted_versions = version_tags(&request.title);
    let actual_versions = version_tags(&candidate.title);
    let version = if wanted_versions == actual_versions {
        5
    } else if wanted_versions != actual_versions
        && (!wanted_versions.is_empty() || !actual_versions.is_empty())
    {
        -20
    } else {
        0
    };
    let total = (title + artist + album + duration + version).clamp(0, 100);
    let artist_required = !wanted_artists.is_empty();
    let reliable = total >= 75
        && title >= 42
        && if artist_required {
            artist > 0
        } else {
            album > 0 && duration > 0
        };
    let confidence = if reliable {
        Confidence::Reliable
    } else if total >= 50 && title >= 25 {
        Confidence::Uncertain
    } else {
        Confidence::Rejected
    };
    MatchScore { total, confidence }
}

#[derive(Clone)]
struct AlignedTranslation {
    lines: Vec<LyricLine>,
    provider: Provider,
    offset: f64,
    matched: usize,
}

fn choose_result(
    candidates: Vec<FetchedLyrics>,
    player: PlayerKind,
    responded: bool,
) -> LookupOutcome {
    let uncertain = candidates
        .iter()
        .filter(|candidate| candidate.score.confidence == Confidence::Uncertain)
        .max_by_key(|candidate| candidate.score.total);
    let reliable = candidates
        .iter()
        .filter(|candidate| {
            candidate.score.confidence == Confidence::Reliable && has_timing(&candidate.lines)
        })
        .collect::<Vec<_>>();
    if reliable.is_empty() {
        if let Some(candidate) = uncertain {
            return LookupOutcome {
                lyrics: Lyrics {
                    status: LyricStatus::Uncertain,
                    source: Some(LyricSource {
                        original: candidate.provider,
                        translation: None,
                        translation_offset: None,
                    }),
                    ..Lyrics::default()
                },
                match_score: Some(candidate.score.total),
                ttl: MISS_CACHE_TTL,
            };
        }
        return missing_outcome(responded);
    }

    let mut selected: Option<(
        FetchedLyrics,
        Option<AlignedTranslation>,
        (i32, usize, usize, i32, usize),
    )> = None;
    for original in &reliable {
        let best_translation = reliable
            .iter()
            .filter(|translation| {
                !translation.translations.is_empty()
                    && same_recording(&original.meta, &translation.meta)
            })
            .filter_map(|translation| {
                align_translation(&original.lines, &translation.translations).map(
                    |(lines, offset, matched)| AlignedTranslation {
                        lines,
                        provider: translation.provider,
                        offset,
                        matched,
                    },
                )
            })
            .max_by_key(|translation| translation.matched);
        let word_rows = original
            .lines
            .iter()
            .filter(|line| !line.words.is_empty())
            .count();
        let translated_rows = best_translation
            .as_ref()
            .map(|translation| translation.matched)
            .unwrap_or_default();
        let word_coverage = word_rows * 1000 / original.lines.len().max(1);
        let translation_coverage = translated_rows * 1000 / original.lines.len().max(1);
        let affinity = source_affinity(player, original.provider);
        let key = (
            original.score.total,
            word_coverage,
            translation_coverage,
            affinity,
            original.lines.len(),
        );
        if selected
            .as_ref()
            .is_none_or(|(_, _, current_key)| key > *current_key)
        {
            selected = Some(((*original).clone(), best_translation, key));
        }
    }

    let (selected, translation, _) = selected.expect("reliable candidates are not empty");
    let mut lines = selected.lines;
    let (translation_provider, translation_offset, translated) = if let Some(aligned) = translation
    {
        lines = aligned.lines;
        (
            Some(aligned.provider),
            Some(aligned.offset),
            aligned.matched,
        )
    } else {
        (None, None, 0)
    };
    let word_timed = lines.iter().any(|line| !line.words.is_empty());
    let high_translation_coverage = !lines.is_empty() && translated * 10 >= lines.len() * 7;
    LookupOutcome {
        lyrics: Lyrics {
            timed: true,
            word_timed,
            status: LyricStatus::Found,
            source: Some(LyricSource {
                original: selected.provider,
                translation: translation_provider,
                translation_offset,
            }),
            from_cache: false,
            lines,
            ..Lyrics::default()
        },
        match_score: Some(selected.score.total),
        ttl: if word_timed && high_translation_coverage {
            GOOD_CACHE_TTL
        } else {
            PARTIAL_CACHE_TTL
        },
    }
}

fn missing_outcome(responded: bool) -> LookupOutcome {
    LookupOutcome {
        lyrics: Lyrics {
            status: LyricStatus::NotFound,
            ..Lyrics::default()
        },
        match_score: None,
        ttl: if responded {
            MISS_CACHE_TTL
        } else {
            ERROR_CACHE_TTL
        },
    }
}

fn source_affinity(player: PlayerKind, provider: Provider) -> i32 {
    match player {
        PlayerKind::QqMusic if provider.is_qq() => 1,
        PlayerKind::NetEase if !provider.is_qq() => 1,
        _ => 0,
    }
}

fn same_recording(left: &SongMeta, right: &SongMeta) -> bool {
    if normalize(&plain_title(&left.title)) != normalize(&plain_title(&right.title)) {
        return false;
    }
    let left_artists = left
        .artists
        .iter()
        .map(|artist| normalize(artist))
        .collect::<HashSet<_>>();
    let right_artists = right
        .artists
        .iter()
        .map(|artist| normalize(artist))
        .collect::<HashSet<_>>();
    let artist_matches = !left_artists.is_disjoint(&right_artists);
    let album_matches =
        !normalize(&left.album).is_empty() && normalize(&left.album) == normalize(&right.album);
    let duration_matches = match (left.duration, right.duration) {
        (Some(left), Some(right)) => (left - right).abs() <= 8.0,
        _ => false,
    };
    artist_matches || (album_matches && duration_matches)
}

/// Finds one global offset, then pairs translation rows monotonically. This
/// tolerates providers shifting an otherwise identical translation timeline.
fn align_translation(
    originals: &[LyricLine],
    translations: &[LyricLine],
) -> Option<(Vec<LyricLine>, f64, usize)> {
    let available = originals.len().min(translations.len());
    if available == 0 {
        return None;
    }
    let mut buckets: HashMap<i32, usize> = HashMap::new();
    for original in originals {
        for translation in translations {
            let delta = original.at - translation.at;
            if delta.abs() <= 5.0 {
                *buckets.entry((delta * 20.0).round() as i32).or_default() += 1;
            }
        }
    }
    let mut offsets = buckets.into_iter().collect::<Vec<_>>();
    offsets.sort_by_key(|(bucket, votes)| (std::cmp::Reverse(*votes), bucket.abs()));

    let mut best: Option<(Vec<LyricLine>, f64, usize)> = None;
    for (bucket, _) in offsets {
        let offset = bucket as f64 / 20.0;
        let (lines, matched) = pair_translation(originals, translations, offset);
        if best.as_ref().is_none_or(|(_, best_offset, best_matched)| {
            matched > *best_matched
                || (matched == *best_matched && offset.abs() < best_offset.abs())
        }) {
            best = Some((lines, offset, matched));
        }
    }
    let (lines, offset, matched) = best?;
    let enough_rows = matched >= available.min(5);
    let enough_coverage = matched * 10 >= available * 7;
    (enough_rows && enough_coverage).then_some((lines, offset, matched))
}

fn pair_translation(
    originals: &[LyricLine],
    translations: &[LyricLine],
    offset: f64,
) -> (Vec<LyricLine>, usize) {
    let mut lines = originals.to_vec();
    let mut translation_index = 0;
    let mut matched = 0;
    for line in &mut lines {
        while translation_index < translations.len()
            && translations[translation_index].at + offset < line.at - 0.2
        {
            translation_index += 1;
        }
        let Some(translation) = translations.get(translation_index) else {
            break;
        };
        if (translation.at + offset - line.at).abs() <= 0.2 {
            let text = translation.text.trim();
            if !text.is_empty() && text != line.text.trim() {
                line.translation = Some(text.to_string());
                matched += 1;
            }
            translation_index += 1;
        }
    }
    (lines, matched)
}

/// Downloads the QRC document and turns it into timed lines. The response is
/// XML whose CDATA sections hold the encrypted lyric and its translation.
fn fetch_qrc(
    client: &reqwest::blocking::Client,
    song_id: i64,
) -> Option<(Vec<LyricLine>, Vec<LyricLine>)> {
    let document = client
        .get(format!(
            "{QRC_URL}?version=15&miniversion=82&lrctype=4&musicid={song_id}"
        ))
        .header("Referer", "https://y.qq.com")
        .send()
        .ok()?
        .text()
        .ok()?;

    let blocks: Vec<&str> = document
        .split("<![CDATA[")
        .skip(1)
        .filter_map(|block| block.split("]]>").next())
        .map(str::trim)
        .collect();

    let lines = parse_qrc(&qrc::decrypt_lyrics(blocks.first()?)?);
    if lines.is_empty() {
        return None;
    }
    let translations = blocks
        .get(1)
        .filter(|block| !block.is_empty())
        .and_then(|block| qrc::decrypt_lyrics(block))
        .map(|text| parse_translation(&text))
        .unwrap_or_default();
    Some((lines, translations))
}

/// The lyric lives in the `LyricContent` attribute of the QRC document.
fn qrc_content(document: &str) -> Option<&str> {
    let start = document.find("LyricContent=\"")? + "LyricContent=\"".len();
    let rest = &document[start..];
    let end = rest.rfind("\"")?;
    Some(&rest[..end])
}

/// `[start,duration]字(start,duration)字(start,duration)…`, milliseconds.
fn parse_qrc(document: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    for row in qrc_content(document).unwrap_or_default().lines() {
        let Some(rest) = row.strip_prefix('[') else {
            continue;
        };
        let Some((head, body)) = rest.split_once(']') else {
            continue;
        };
        let Some((start, _)) = head.split_once(',') else {
            continue;
        };
        let Ok(start): Result<f64, _> = start.trim().parse() else {
            continue;
        };

        let words = parse_qrc_words(body);
        let text: String = words.iter().map(|word| word.text.as_str()).collect();
        if text.trim().is_empty() {
            continue;
        }
        lines.push(LyricLine {
            at: start / 1000.0,
            text,
            translation: None,
            words,
        });
    }
    lines.sort_by(|a, b| a.at.total_cmp(&b.at));
    lines
}

/// NetEase's YRC uses the same timing data as QRC, without the XML wrapper or
/// encryption; both before-character and after-character layouts are accepted.
fn parse_yrc(raw: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    for row in raw.lines() {
        let Some(rest) = row.strip_prefix('[') else {
            continue;
        };
        let Some((head, body)) = rest.split_once(']') else {
            continue;
        };
        let Some((start, _)) = head.split_once(',') else {
            continue;
        };
        let Ok(start): Result<f64, _> = start.trim().parse() else {
            continue;
        };
        let words = parse_yrc_words(body);
        let text: String = words.iter().map(|word| word.text.as_str()).collect();
        if text.trim().is_empty() {
            continue;
        }
        lines.push(LyricLine {
            at: words.first().map(|word| word.at).unwrap_or(start / 1000.0),
            text,
            translation: None,
            words,
        });
    }
    lines.sort_by(|a, b| a.at.total_cmp(&b.at));
    lines
}

/// YRC commonly puts each timing group before its character, while QRC puts
/// it after the character. Accept both forms because NetEase has served both
/// layouts over time.
fn parse_yrc_words(body: &str) -> Vec<LyricWord> {
    if !body.trim_start().starts_with('(') {
        return parse_qrc_words(body);
    }
    let mut words = Vec::new();
    let mut chars = body.chars().peekable();
    while chars.peek().is_some() {
        if chars.next() != Some('(') {
            continue;
        }
        let timing: String = chars.by_ref().take_while(|next| *next != ')').collect();
        let mut values = timing.split(',');
        let Some(at) = values
            .next()
            .and_then(|value| value.trim().parse::<f64>().ok())
        else {
            continue;
        };
        let Some(duration) = values
            .next()
            .and_then(|value| value.trim().parse::<f64>().ok())
        else {
            continue;
        };
        let mut text = String::new();
        while let Some(next) = chars.peek().copied() {
            if next == '(' {
                break;
            }
            text.push(next);
            chars.next();
        }
        if !text.is_empty() {
            words.push(LyricWord {
                at: at / 1000.0,
                duration: duration / 1000.0,
                text,
            });
        }
    }
    words
}

fn parse_qrc_words(body: &str) -> Vec<LyricWord> {
    let mut words = Vec::new();
    let mut text = String::new();
    let mut chars = body.chars().peekable();

    while let Some(character) = chars.next() {
        if character != '(' {
            text.push(character);
            continue;
        }
        let timing: String = chars.by_ref().take_while(|next| *next != ')').collect();
        // A literal bracket in the lyric is not a timing group.
        let mut values = timing.split(',');
        let Some(at) = values
            .next()
            .and_then(|value| value.trim().parse::<f64>().ok())
        else {
            text.push('(');
            text.push_str(&timing);
            text.push(')');
            continue;
        };
        let Some(duration) = values
            .next()
            .and_then(|value| value.trim().parse::<f64>().ok())
        else {
            text.push('(');
            text.push_str(&timing);
            text.push(')');
            continue;
        };
        words.push(LyricWord {
            at: at / 1000.0,
            duration: duration / 1000.0,
            text: std::mem::take(&mut text),
        });
    }
    if !text.is_empty() {
        if let Some(last) = words.last_mut() {
            last.text.push_str(&text);
        }
    }
    words
}

/// Parses a translation payload while preserving literal parentheses. QRC
/// timing groups are removed only when they contain numeric timing values.
fn parse_translation(document: &str) -> Vec<LyricLine> {
    let content = qrc_content(document).unwrap_or(document);
    let mut lines = Vec::new();
    for row in content.lines() {
        let Some(rest) = row.strip_prefix('[') else {
            continue;
        };
        let Some((head, body)) = rest.split_once(']') else {
            continue;
        };
        let Some((start, _)) = head.split_once(',') else {
            continue;
        };
        let Ok(start): Result<f64, _> = start.trim().parse() else {
            continue;
        };
        let words = parse_qrc_words(body);
        let text = if words.is_empty() {
            body.trim().to_string()
        } else {
            words.into_iter().map(|word| word.text).collect()
        };
        if !text.trim().is_empty() {
            lines.push(LyricLine {
                at: start / 1000.0,
                text,
                translation: None,
                words: Vec::new(),
            });
        }
    }
    if lines.is_empty() {
        parse_lrc(content)
    } else {
        lines.sort_by(|a, b| a.at.total_cmp(&b.at));
        lines
    }
}

/// Some responses come back wrapped in a callback, e.g. `MusicJsonCallback({…})`.
fn strip_jsonp(raw: &str) -> &str {
    match (raw.find('{'), raw.rfind('}')) {
        (Some(start), Some(end)) if end > start => &raw[start..=end],
        _ => raw,
    }
}

/// `[mm:ss.xx]text`, with repeated stamps for shared lines.
fn parse_lrc(raw: &str) -> Vec<LyricLine> {
    let mut lines: Vec<LyricLine> = Vec::new();
    for row in raw.lines() {
        let mut rest = row;
        let mut stamps = Vec::new();
        while let Some(close) = rest.strip_prefix('[').and_then(|rest| rest.find(']')) {
            let stamp = &rest[1..=close];
            if let Some(at) = parse_stamp(stamp) {
                stamps.push(at);
            }
            rest = &rest[close + 2..];
        }
        let text = rest.trim();
        if text.is_empty() {
            continue;
        }
        for at in stamps {
            lines.push(LyricLine {
                at,
                text: text.to_string(),
                translation: None,
                words: Vec::new(),
            });
        }
    }
    lines.sort_by(|a, b| a.at.total_cmp(&b.at));
    lines
}

fn parse_stamp(stamp: &str) -> Option<f64> {
    let (minutes, rest) = stamp.split_once(':')?;
    let minutes: f64 = minutes.trim().parse().ok()?;
    let seconds: f64 = rest.replace(':', ".").parse().ok()?;
    Some(minutes * 60.0 + seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media(source_id: &str, app_name: &str) -> MediaState {
        MediaState {
            title: "Song".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            app_name: app_name.into(),
            source_id: source_id.into(),
            playing: true,
            duration: Some(180.0),
            elapsed: Some(10.0),
            elapsed_at: 0.0,
            artwork: None,
        }
    }

    fn request(title: &str, artist: &str, album: &str, duration: Option<f64>) -> Request {
        Request {
            track: "track".into(),
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            duration,
            player: PlayerKind::Other,
            generation: 1,
            force: false,
        }
    }

    fn meta(title: &str, artist: &str, album: &str, duration: Option<f64>) -> SongMeta {
        SongMeta {
            title: title.into(),
            artists: (!artist.is_empty())
                .then(|| artist.into())
                .into_iter()
                .collect(),
            album: album.into(),
            duration,
        }
    }

    fn lines(prefix: &str, count: usize, offset: f64) -> Vec<LyricLine> {
        (0..count)
            .map(|index| LyricLine {
                at: index as f64 * 2.0 + offset,
                text: format!("{prefix}{index}"),
                translation: None,
                words: Vec::new(),
            })
            .collect()
    }

    fn candidate(
        provider: Provider,
        score: i32,
        confidence: Confidence,
        word_timed: bool,
        translations: Vec<LyricLine>,
    ) -> FetchedLyrics {
        let mut lyric_lines = lines("line", 10, 0.0);
        if word_timed {
            for line in &mut lyric_lines {
                line.words.push(LyricWord {
                    at: line.at,
                    duration: 1.0,
                    text: line.text.clone(),
                });
            }
        }
        FetchedLyrics {
            provider,
            meta: meta("Song", "Artist", "Album", Some(180.0)),
            score: MatchScore {
                total: score,
                confidence,
            },
            lines: lyric_lines,
            translations,
        }
    }

    #[test]
    fn detects_known_player_identifiers() {
        assert_eq!(
            player_kind(&media("com.tencent.QQMusicMac", "QQ 音乐")),
            PlayerKind::QqMusic
        );
        assert_eq!(
            player_kind(&media("com.netease.163music", "网易云音乐")),
            PlayerKind::NetEase
        );
        assert_eq!(
            player_kind(&media("com.spotify.client", "Spotify")),
            PlayerKind::Spotify
        );
    }

    #[test]
    fn exact_metadata_is_reliable_and_title_only_is_uncertain() {
        let exact = match_score(
            &request("Song", "Artist", "Album", Some(180.0)),
            &meta("Song", "Artist", "Album", Some(181.0)),
        );
        assert_eq!(exact.confidence, Confidence::Reliable);
        assert!(exact.total >= 75);

        let title_only = match_score(
            &request("Song", "", "", None),
            &meta("Song", "Other", "Other", None),
        );
        assert_eq!(title_only.confidence, Confidence::Uncertain);
    }

    #[test]
    fn missing_artist_requires_album_and_duration() {
        let score = match_score(
            &request("Song", "", "Album", Some(180.0)),
            &meta("Song", "Artist", "Album", Some(181.0)),
        );
        assert_eq!(score.confidence, Confidence::Reliable);
    }

    #[test]
    fn conflicting_versions_are_not_reliable() {
        let score = match_score(
            &request("Song - Live", "Artist", "Album", Some(180.0)),
            &meta("Song - Remix", "Artist", "Album", Some(180.0)),
        );
        assert_eq!(score.confidence, Confidence::Uncertain);
    }

    #[test]
    fn parses_yrc_words() {
        let lines = parse_yrc("[1000,1000]he(1000,400)llo(1400,600)");
        assert_eq!(lines[0].text, "hello");
        assert_eq!(lines[0].words.len(), 2);
        assert!((lines[0].words[1].at - 1.4).abs() < f64::EPSILON);

        let prefixed = parse_yrc("[1000,1000](1000,400,0)he(1400,600,0)llo");
        assert_eq!(prefixed[0].text, "hello");
        assert_eq!(prefixed[0].words.len(), 2);
        assert!((prefixed[0].words[0].at - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn parses_qrc_content_and_character_timings() {
        let lines = parse_qrc(r#"<LyricContent="[0,1000]he(0,500)llo(500,500)"/>"#);
        assert_eq!(lines[0].text, "hello");
        assert_eq!(lines[0].words.len(), 2);
        assert!((lines[0].words[1].duration - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn aligns_positive_and_negative_translation_offsets() {
        let original = lines("line", 8, 0.0);
        let translated = lines("译", 8, 1.2);
        let (_, offset, matched) = align_translation(&original, &translated).unwrap();
        assert!((offset + 1.2).abs() <= 0.05);
        assert_eq!(matched, 8);

        let early = lines("译", 8, -0.8);
        let (_, offset, matched) = align_translation(&original, &early).unwrap();
        assert!((offset - 0.8).abs() <= 0.05);
        assert_eq!(matched, 8);
    }

    #[test]
    fn rejects_translation_with_low_timeline_coverage() {
        let original = lines("line", 10, 0.0);
        let mut translated = lines("译", 3, 0.0);
        translated.extend(lines("wrong", 7, 100.0));
        assert!(align_translation(&original, &translated).is_none());
    }

    #[test]
    fn qrc_translation_preserves_literal_parentheses() {
        let lines = parse_qrc(r#"<LyricContent="[1000,1000]Hello(1200,800)"/>"#);
        let translations =
            parse_translation(r#"<LyricContent="[1000,1000]你好（朋友）(1000,1000)"/>"#);
        let (lines, _, _) = align_translation(&lines, &translations).unwrap();
        assert_eq!(lines[0].translation.as_deref(), Some("你好（朋友）"));
    }

    #[test]
    fn selection_prefers_match_then_word_timing_then_translation() {
        let translation = lines("译", 10, 1.0);
        let lower_match = candidate(
            Provider::NeteaseYrc,
            80,
            Confidence::Reliable,
            true,
            translation.clone(),
        );
        let higher_match = candidate(Provider::QqLrc, 90, Confidence::Reliable, false, Vec::new());
        let outcome = choose_result(vec![lower_match, higher_match], PlayerKind::Other, true);
        assert_eq!(outcome.lyrics.source.unwrap().original, Provider::QqLrc);

        let plain = candidate(Provider::QqLrc, 90, Confidence::Reliable, false, Vec::new());
        let timed = candidate(
            Provider::NeteaseYrc,
            90,
            Confidence::Reliable,
            true,
            translation,
        );
        let outcome = choose_result(vec![plain, timed], PlayerKind::Other, true);
        assert_eq!(
            outcome.lyrics.source.unwrap().original,
            Provider::NeteaseYrc
        );
        assert!(outcome.lyrics.word_timed);
    }

    #[test]
    fn cross_source_translation_is_reported_and_extends_cache_ttl() {
        let original = candidate(Provider::QqQrc, 90, Confidence::Reliable, true, Vec::new());
        let translation = candidate(
            Provider::NeteaseLrc,
            90,
            Confidence::Reliable,
            false,
            lines("译", 10, 1.0),
        );
        let outcome = choose_result(vec![original, translation], PlayerKind::Other, true);
        let source = outcome.lyrics.source.unwrap();
        assert_eq!(source.original, Provider::QqQrc);
        assert_eq!(source.translation, Some(Provider::NeteaseLrc));
        assert_eq!(outcome.ttl, GOOD_CACHE_TTL);
    }

    #[test]
    fn player_source_breaks_equal_quality_ties() {
        let qq = candidate(Provider::QqQrc, 90, Confidence::Reliable, true, Vec::new());
        let netease = candidate(
            Provider::NeteaseYrc,
            90,
            Confidence::Reliable,
            true,
            Vec::new(),
        );
        let outcome = choose_result(vec![qq, netease], PlayerKind::NetEase, true);
        assert_eq!(
            outcome.lyrics.source.unwrap().original,
            Provider::NeteaseYrc
        );
    }

    #[test]
    fn uncertain_result_never_exposes_lines() {
        let outcome = choose_result(
            vec![candidate(
                Provider::QqLrc,
                60,
                Confidence::Uncertain,
                false,
                Vec::new(),
            )],
            PlayerKind::Other,
            true,
        );
        assert_eq!(outcome.lyrics.status, LyricStatus::Uncertain);
        assert!(outcome.lyrics.lines.is_empty());
        assert_eq!(outcome.ttl, MISS_CACHE_TTL);
    }

    #[test]
    fn cache_v4_round_trips_and_v3_is_rejected() {
        let cache = CacheFile {
            version: CACHE_VERSION,
            created_at: 10,
            expires_at: 20,
            match_score: Some(90),
            lyrics: Lyrics {
                status: LyricStatus::Found,
                ..Lyrics::default()
            },
        };
        let raw = serde_json::to_string(&cache).unwrap();
        assert_eq!(read_cache(&raw).unwrap().expires_at, 20);
        let old = raw.replace("\"version\":4", "\"version\":3");
        assert!(read_cache(&old).is_none());
        assert_eq!(
            serde_json::to_string(&Provider::NeteaseYrc).unwrap(),
            "\"netease-yrc\""
        );
        assert_eq!(missing_outcome(false).ttl, ERROR_CACHE_TTL);
    }
}
