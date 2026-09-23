use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use tauri::{AppHandle, Manager};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionMediaProperties as MediaProperties,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use windows::Storage::Streams::DataReader;

use super::{publish, MediaCommand, MediaHub, MediaState};

/// 100ns ticks between the FILETIME epoch (1601) and the Unix epoch.
const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;
const TICKS_PER_SECOND: f64 = 10_000_000.0;
const POLL: Duration = Duration::from_secs(1);
const MAX_ARTWORK_BYTES: u32 = 8 * 1024 * 1024;
/// Polls (one per second) to keep asking a player for its artwork.
const ARTWORK_ATTEMPTS: u8 = 20;

#[derive(Default)]
struct ArtworkCache {
    track: String,
    attempts: u8,
    data_url: Option<String>,
}

/// Position kept by Bangs itself for players that publish no timeline at all
/// (NetEase Cloud Music on Windows reports start, end and position as zero and
/// never updates them). It runs only while the player says it is playing, and
/// it is only trusted for a track Bangs saw begin: one that replaced another
/// track in the same player. A track already under way when Bangs started, or
/// the one a player restores at launch, could be anywhere, and a wrong lyric
/// is worse than none. A seek inside the player cannot be seen at all.
#[derive(Default)]
struct Stopwatch {
    track: String,
    trusted: bool,
    banked: f64,
    running_since: Option<Instant>,
}

impl Stopwatch {
    fn elapsed(&mut self, track: &str, playing: bool) -> Option<f64> {
        if self.track != track {
            // The track key starts with the player's id; another app taking
            // over the media session and handing it back is not a new song.
            let player = |key: &str| key.split('\u{1f}').next().map(str::to_string);
            let trusted = !self.track.is_empty() && player(&self.track) == player(track);
            *self = Stopwatch { track: track.to_string(), trusted, ..Stopwatch::default() };
        }
        match (playing, self.running_since) {
            (true, None) => self.running_since = Some(Instant::now()),
            (false, Some(since)) => {
                self.banked += since.elapsed().as_secs_f64();
                self.running_since = None;
            }
            _ => {}
        }
        self.trusted.then(|| {
            self.banked + self.running_since.map_or(0.0, |since| since.elapsed().as_secs_f64())
        })
    }
}

pub fn start(app: AppHandle) {
    let (sender, commands) = mpsc::channel();
    app.state::<MediaHub>().set_sender(sender);

    thread::spawn(move || {
        let manager = loop {
            match SessionManager::RequestAsync().and_then(|operation| operation.get()) {
                Ok(manager) => break manager,
                Err(error) => {
                    eprintln!("[media] media session manager unavailable: {error}");
                    thread::sleep(Duration::from_secs(10));
                }
            }
        };

        let mut artwork = ArtworkCache::default();
        let mut stopwatch = Stopwatch::default();
        loop {
            // The current session changes as players start and stop, so it
            // is looked up on every poll rather than subscribed to once.
            let session = manager.GetCurrentSession().ok();
            if session.is_none() {
                // The player quit; whatever it shows next may be a track it
                // restored halfway through.
                stopwatch = Stopwatch::default();
            }
            let state = session
                .as_ref()
                .and_then(|session| read_session(session, &mut artwork, &mut stopwatch).ok().flatten());
            publish(&app, state);

            match commands.recv_timeout(POLL) {
                Ok(command) => {
                    if let Some(session) = &session {
                        if let Err(error) = send_command(session, command) {
                            eprintln!("[media] command failed: {error}");
                        }
                        // Give the player a moment so the next read reflects it.
                        thread::sleep(Duration::from_millis(150));
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    });
}

fn read_session(
    session: &Session,
    artwork: &mut ArtworkCache,
    stopwatch: &mut Stopwatch,
) -> windows::core::Result<Option<MediaState>> {
    let properties = session.TryGetMediaPropertiesAsync()?.get()?;
    let title = properties.Title()?.to_string();
    if title.is_empty() {
        return Ok(None);
    }
    let artist = properties.Artist()?.to_string();
    let album = properties.AlbumTitle()?.to_string();
    let source_id = session.SourceAppUserModelId()?.to_string();
    let playing = session.GetPlaybackInfo()?.PlaybackStatus()? == PlaybackStatus::Playing;

    let timeline = session.GetTimelineProperties()?;
    let start = timeline.StartTime()?.Duration;
    let end = timeline.EndTime()?.Duration;
    let position = timeline.Position()?.Duration;
    let updated = timeline.LastUpdatedTime()?.UniversalTime;
    let duration = (end > start).then(|| (end - start) as f64 / TICKS_PER_SECOND);
    let has_timeline = duration.is_some() || position != 0 || updated > UNIX_EPOCH_TICKS;

    // Players often publish the thumbnail well after the title — some only
    // once playback actually starts — so keep asking for a while.
    let track = format!("{source_id}\u{1f}{title}\u{1f}{artist}\u{1f}{album}");
    let (elapsed, elapsed_at) = if has_timeline {
        (
            duration.map(|_| (position - start) as f64 / TICKS_PER_SECOND),
            if updated > UNIX_EPOCH_TICKS {
                (updated - UNIX_EPOCH_TICKS) as f64 / 10_000.0
            } else {
                now_ms()
            },
        )
    } else {
        (stopwatch.elapsed(&track, playing), now_ms())
    };
    if artwork.track != track {
        *artwork = ArtworkCache { track, ..ArtworkCache::default() };
    }
    if artwork.data_url.is_none() && artwork.attempts < ARTWORK_ATTEMPTS {
        artwork.attempts += 1;
        match read_thumbnail(&properties) {
            Ok(Some(data_url)) => artwork.data_url = Some(data_url),
            Ok(None) if artwork.attempts == ARTWORK_ATTEMPTS => {
                eprintln!("[media] {source_id} sends no artwork for {title}");
            }
            Ok(None) => {}
            Err(error) => eprintln!("[media] artwork read failed: {error}"),
        }
    }

    Ok(Some(MediaState {
        app_name: friendly_app_name(&source_id),
        title,
        artist,
        album,
        source_id,
        playing,
        duration,
        elapsed,
        elapsed_at,
        artwork: artwork.data_url.clone(),
    }))
}

fn read_thumbnail(properties: &MediaProperties) -> windows::core::Result<Option<String>> {
    let Ok(reference) = properties.Thumbnail() else {
        return Ok(None);
    };
    let stream = reference.OpenReadAsync()?.get()?;
    let size = u32::try_from(stream.Size()?).unwrap_or(u32::MAX);
    if size == 0 || size > MAX_ARTWORK_BYTES {
        return Ok(None);
    }
    let reader = DataReader::CreateDataReader(&stream)?;
    let loaded = reader.LoadAsync(size)?.get()?;
    let mut bytes = vec![0u8; loaded as usize];
    reader.ReadBytes(&mut bytes)?;
    if bytes.is_empty() {
        return Ok(None);
    }

    // The stream's own content type is often empty or something generic like
    // application/octet-stream, and a data URL with the wrong type is simply
    // not drawn, so the bytes decide.
    let mime = sniff(&bytes).unwrap_or_else(|| {
        let reported = stream.ContentType().map(|value| value.to_string()).unwrap_or_default();
        if reported.starts_with("image/") { reported } else { "image/jpeg".into() }
    });
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(Some(format!("data:{mime};base64,{encoded}")))
}

/// The image type as the bytes themselves declare it.
fn sniff(bytes: &[u8]) -> Option<String> {
    let kind = match bytes {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xFF, 0xD8, 0xFF, ..] => "image/jpeg",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [b'B', b'M', ..] => "image/bmp",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        _ => return None,
    };
    Some(kind.to_string())
}

fn send_command(session: &Session, command: MediaCommand) -> windows::core::Result<()> {
    match command {
        MediaCommand::Toggle => {
            session.TryTogglePlayPauseAsync()?.get()?;
        }
        MediaCommand::Next => {
            session.TrySkipNextAsync()?.get()?;
        }
        MediaCommand::Previous => {
            session.TrySkipPreviousAsync()?.get()?;
        }
        MediaCommand::Seek { position } => {
            let start = session.GetTimelineProperties()?.StartTime()?.Duration;
            let target = start + (position.max(0.0) * TICKS_PER_SECOND) as i64;
            session.TryChangePlaybackPositionAsync(target)?.get()?;
        }
    }
    Ok(())
}

fn friendly_app_name(source_id: &str) -> String {
    const KNOWN: &[(&str, &str)] = &[
        ("spotify", "Spotify"),
        ("qqmusic", "QQ 音乐"),
        ("cloudmusic", "网易云音乐"),
        ("kugou", "酷狗音乐"),
        ("kuwo", "酷我音乐"),
        ("zunemusic", "媒体播放器"),
        ("msedge", "Microsoft Edge"),
        ("chrome", "Google Chrome"),
        ("firefox", "Firefox"),
    ];
    let lower = source_id.to_lowercase();
    if let Some((_, name)) = KNOWN.iter().find(|(needle, _)| lower.contains(needle)) {
        return (*name).to_string();
    }
    let base = source_id.rsplit(['\\', '!']).next().unwrap_or(source_id);
    base.strip_suffix(".exe")
        .or_else(|| base.strip_suffix(".EXE"))
        .unwrap_or(base)
        .to_string()
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs_f64() * 1000.0)
        .unwrap_or_default()
}
