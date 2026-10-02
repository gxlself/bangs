//! What the coding tools on this machine are doing: Claude Code sessions
//! (`~/.claude/sessions/*.json`), Codex CLI sessions (`~/.codex/sessions`
//! rollout logs) and the projects VS Code / Cursor have open.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System, UpdateKind};
use tauri::{AppHandle, Emitter, Manager};

const POLL: Duration = Duration::from_secs(2);
/// How far back a Codex rollout counts as a session worth showing.
const CODEX_RECENT: Duration = Duration::from_secs(6 * 60 * 60);
/// A Codex turn that stopped writing this long ago is treated as finished,
/// in case the process died mid-task.
const CODEX_STALE: Duration = Duration::from_secs(5 * 60);
/// Sessions of the Claude desktop app live in throwaway folders.
const SCRATCH_MARKER: &str = "/scratch-workspaces/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionStatus {
    /// Claude is working.
    Busy,
    /// Claude stopped and wants an answer.
    Waiting,
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Agent {
    Claude,
    Codex,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSession {
    pub id: String,
    pub agent: Agent,
    pub name: String,
    pub path: String,
    pub project: String,
    pub status: SessionStatus,
    /// What it is waiting for, when the session says so.
    pub detail: Option<String>,
    /// Unix milliseconds of the last status change.
    pub updated_at: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorWorkspace {
    /// "code" or "cursor".
    pub editor: String,
    pub editor_name: String,
    pub path: String,
    pub project: String,
    pub branch: Option<String>,
    /// The window the editor had in front when it last wrote its state.
    pub active: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevState {
    pub sessions: Vec<AgentSession>,
    pub workspaces: Vec<EditorWorkspace>,
}

#[derive(Default)]
pub struct DevHub(Mutex<DevState>);

impl DevHub {
    pub fn current(&self) -> DevState {
        self.0.lock().unwrap().clone()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionFile {
    pid: u32,
    session_id: String,
    cwd: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    waiting_for: Option<String>,
    #[serde(default)]
    status_updated_at: f64,
    #[serde(default)]
    updated_at: f64,
}

struct Editor {
    id: &'static str,
    name: &'static str,
    /// Directory name under the OS application-data folder.
    data_dir: &'static str,
    /// Substrings of the executable path that mean the editor is running.
    /// On macOS both editors run an executable called `Electron`, so the
    /// bundle path is what tells them apart.
    markers: &'static [&'static str],
}

const EDITORS: &[Editor] = &[
    Editor {
        id: "code",
        name: "VS Code",
        data_dir: "Code",
        markers: &["Visual Studio Code.app", "Microsoft VS Code", "Code.exe"],
    },
    Editor {
        id: "cursor",
        name: "Cursor",
        data_dir: "Cursor",
        markers: &["Cursor.app", "cursor/Cursor.exe", "Cursor.exe"],
    },
];

/// The panel is a glance, not a session manager: keep every session that is
/// doing something, and only the most recent idle ones.
const MAX_IDLE_SESSIONS: usize = 3;
/// Caps the workspace list so a long editor history cannot bury the sessions.
const MAX_WORKSPACES: usize = 5;

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Where editors keep `User/globalStorage`.
fn app_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    return home().map(|home| home.join("Library/Application Support"));
    #[cfg(windows)]
    return std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(any(target_os = "macos", windows)))]
    return home().map(|home| home.join(".config"));
}

fn project_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

fn read_claude_sessions(system: &System) -> Vec<AgentSession> {
    let Some(dir) = home().map(|home| home.join(".claude/sessions")) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| {
            let file: SessionFile = serde_json::from_str(&fs::read_to_string(entry.path()).ok()?).ok()?;
            // Session files outlive their process; drop the stale ones.
            system.process(sysinfo::Pid::from_u32(file.pid))?;
            if file.cwd.contains(SCRATCH_MARKER) {
                return None;
            }
            let status = match file.status.as_str() {
                "busy" => SessionStatus::Busy,
                "waiting" => SessionStatus::Waiting,
                _ => SessionStatus::Idle,
            };
            Some(AgentSession {
                agent: Agent::Claude,
                project: project_name(&file.cwd),
                name: file.name,
                path: file.cwd,
                status,
                detail: file.waiting_for.filter(|detail| !detail.is_empty()),
                updated_at: if file.status_updated_at > 0.0 { file.status_updated_at } else { file.updated_at },
                id: file.session_id,
            })
        })
        .collect()
}

/// Sorts attention first, then recency, and trims the idle tail.
fn prioritize(sessions: &mut Vec<AgentSession>) {
    sessions.sort_by(|a, b| {
        let rank = |status| match status {
            SessionStatus::Waiting => 0,
            SessionStatus::Busy => 1,
            SessionStatus::Idle => 2,
        };
        rank(a.status)
            .cmp(&rank(b.status))
            .then(b.updated_at.total_cmp(&a.updated_at))
    });

    let mut idle = 0;
    sessions.retain(|session| {
        if session.status != SessionStatus::Idle {
            return true;
        }
        idle += 1;
        idle <= MAX_IDLE_SESSIONS
    });
}

/// Alternates between the two agents so a busy one cannot push the other out
/// of the visible rows; each list is already sorted by urgency.
fn interleave(claude: Vec<AgentSession>, codex: Vec<AgentSession>) -> Vec<AgentSession> {
    let mut claude = claude.into_iter();
    let mut codex = codex.into_iter();
    let mut merged = Vec::new();
    loop {
        match (claude.next(), codex.next()) {
            (None, None) => return merged,
            (first, second) => merged.extend(first.into_iter().chain(second)),
        }
    }
}

/// Codex appends to one rollout log per session, so the file itself carries
/// both the project and whether a turn is still running.
#[derive(Default)]
struct CodexProbe {
    /// How far into the log the task markers have been scanned.
    read_to: u64,
    cwd: String,
    running: bool,
}

fn newest_dirs(dir: &Path, keep: usize) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    dirs.reverse();
    dirs.truncate(keep);
    dirs
}

/// `~/.codex/sessions/<year>/<month>/<day>`, newest days only.
fn codex_day_dirs() -> Vec<PathBuf> {
    let Some(root) = home().map(|home| home.join(".codex/sessions")) else {
        return Vec::new();
    };
    let mut dirs = vec![root];
    for keep in [1, 1, 2] {
        dirs = dirs.iter().flat_map(|dir| newest_dirs(dir, keep)).collect();
    }
    dirs
}

/// The first line of a rollout is its `session_meta`.
fn codex_cwd(path: &Path) -> Option<String> {
    let mut line = String::new();
    BufReader::new(File::open(path).ok()?).read_line(&mut line).ok()?;
    let meta: serde_json::Value = serde_json::from_str(&line).ok()?;
    meta["payload"]["cwd"].as_str().map(|cwd| cwd.to_string())
}

/// Reads the part of the log added since last time and reports whether the
/// last task marker in it was a start (`None` when the slice had neither).
fn codex_running(path: &Path, from: u64) -> Option<bool> {
    let mut file = File::open(path).ok()?;
    file.seek(SeekFrom::Start(from)).ok()?;
    let mut appended = String::new();
    file.take(8 * 1024 * 1024).read_to_string(&mut appended).ok()?;
    let started = appended.rfind(r#""type":"task_started""#);
    let completed = appended.rfind(r#""type":"task_complete""#);
    match (started, completed) {
        (Some(started), Some(completed)) => Some(started > completed),
        (Some(_), None) => Some(true),
        (None, Some(_)) => Some(false),
        (None, None) => None,
    }
}

fn unix_ms(time: SystemTime) -> f64 {
    time.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64() * 1000.0
}

fn read_codex_sessions(probes: &mut HashMap<PathBuf, CodexProbe>) -> Vec<AgentSession> {
    let now = SystemTime::now();
    let mut newest: BTreeMap<String, AgentSession> = BTreeMap::new();
    let mut seen = HashSet::new();

    for dir in codex_day_dirs() {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "jsonl") {
                continue;
            }
            let Ok(metadata) = entry.metadata() else { continue };
            let Ok(modified) = metadata.modified() else { continue };
            let idle_for = now.duration_since(modified).unwrap_or_default();
            if idle_for > CODEX_RECENT {
                continue;
            }

            let probe = probes.entry(path.clone()).or_default();
            if probe.cwd.is_empty() {
                let Some(cwd) = codex_cwd(&path) else { continue };
                probe.cwd = cwd;
            }
            if metadata.len() > probe.read_to {
                if let Some(running) = codex_running(&path, probe.read_to) {
                    probe.running = running;
                }
                probe.read_to = metadata.len();
            }
            seen.insert(path.clone());

            let session = AgentSession {
                id: path.to_string_lossy().into_owned(),
                agent: Agent::Codex,
                name: path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default(),
                project: project_name(&probe.cwd),
                path: probe.cwd.clone(),
                // A run that stopped writing minutes ago is over, even if the
                // log never got its task_complete.
                status: if probe.running && idle_for < CODEX_STALE {
                    SessionStatus::Busy
                } else {
                    SessionStatus::Idle
                },
                detail: None,
                updated_at: unix_ms(modified),
            };
            // One row per project: several rollouts pile up for the same one.
            newest
                .entry(session.path.clone())
                .and_modify(|current| {
                    if session.status == SessionStatus::Busy || session.updated_at > current.updated_at {
                        *current = session.clone();
                    }
                })
                .or_insert(session);
        }
    }

    probes.retain(|path, _| seen.contains(path));
    newest.into_values().collect()
}

/// `file:///Users/me/dev` -> `/Users/me/dev`; other schemes are skipped.
fn path_from_uri(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let mut decoded = String::with_capacity(rest.len());
    let mut bytes = Vec::new();
    let mut chars = rest.chars();
    while let Some(character) = chars.next() {
        if character == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                bytes.push(byte);
                continue;
            }
        }
        if !bytes.is_empty() {
            decoded.push_str(&String::from_utf8_lossy(&bytes));
            bytes.clear();
        }
        decoded.push(character);
    }
    if !bytes.is_empty() {
        decoded.push_str(&String::from_utf8_lossy(&bytes));
    }
    // Windows URIs look like file:///c%3A/dev.
    let path = decoded.strip_prefix('/').filter(|rest| rest.contains(':')).unwrap_or(&decoded);
    Some(path.to_string())
}

fn git_branch(path: &str) -> Option<String> {
    let head = fs::read_to_string(Path::new(path).join(".git/HEAD")).ok()?;
    let head = head.trim();
    match head.strip_prefix("ref: refs/heads/") {
        Some(branch) => Some(branch.to_string()),
        None => head.get(..7).map(|sha| sha.to_string()),
    }
}

fn read_workspaces(system: &System) -> Vec<EditorWorkspace> {
    let Some(app_data) = app_data_dir() else {
        return Vec::new();
    };

    let mut workspaces = Vec::new();
    for editor in EDITORS {
        let running = system.processes().values().any(|process| {
            let executable = process.exe().map(|path| path.to_string_lossy()).unwrap_or_default();
            editor.markers.iter().any(|marker| executable.contains(marker))
        });
        if !running {
            continue;
        }

        let storage = app_data.join(editor.data_dir).join("User/globalStorage/storage.json");
        let Ok(raw) = fs::read_to_string(&storage) else { continue };
        let Ok(state) = serde_json::from_str::<serde_json::Value>(&raw) else { continue };

        let windows = &state["windowsState"];
        let active = windows["lastActiveWindow"]["folder"].as_str().and_then(path_from_uri);
        // `backupWorkspaces` tracks the currently open windows; `windowsState`
        // is written on quit. Together they cover both cases.
        let folders = windows["openedWindows"]
            .as_array()
            .map(|windows| windows.iter().filter_map(|window| window["folder"].as_str()).collect())
            .unwrap_or_else(Vec::new)
            .into_iter()
            .chain(
                state["backupWorkspaces"]["folders"]
                    .as_array()
                    .map(|folders| folders.iter().filter_map(|folder| folder["folderUri"].as_str()).collect())
                    .unwrap_or_else(Vec::new),
            );

        let mut seen = BTreeMap::new();
        for folder in folders {
            let Some(path) = path_from_uri(folder) else { continue };
            if !Path::new(&path).is_dir() {
                continue;
            }
            seen.entry(path.clone()).or_insert_with(|| EditorWorkspace {
                editor: editor.id.to_string(),
                editor_name: editor.name.to_string(),
                project: project_name(&path),
                branch: git_branch(&path),
                active: active.as_deref() == Some(path.as_str()),
                path,
            });
        }
        workspaces.extend(seen.into_values());
    }

    workspaces.sort_by_key(|workspace| !workspace.active);
    workspaces.truncate(MAX_WORKSPACES);
    workspaces
}

pub fn start(app: AppHandle) {
    thread::spawn(move || {
        let mut system = System::new_with_specifics(
            RefreshKind::nothing()
                .with_processes(ProcessRefreshKind::nothing().with_exe(UpdateKind::Always)),
        );
        let mut codex_probes = HashMap::new();
        // Sync wants to hear the first pass even when there are no sessions at
        // all, or the ones it knows from the last run would never be cleared.
        let mut first_pass = true;
        loop {
            system.refresh_processes(ProcessesToUpdate::All, true);
            let mut claude = read_claude_sessions(&system);
            prioritize(&mut claude);
            let mut codex = read_codex_sessions(&mut codex_probes);
            prioritize(&mut codex);

            let next = DevState {
                sessions: interleave(claude, codex),
                workspaces: read_workspaces(&system),
            };

            let changed = {
                let hub = app.state::<DevHub>();
                let mut current = hub.0.lock().unwrap();
                let changed = *current != next;
                if changed {
                    *current = next.clone();
                }
                changed
            };
            if changed || first_pass {
                crate::sync::mirror_sessions(&app, &next.sessions);
            }
            first_pass = false;
            if changed {
                let _ = app.emit("bangs://dev", next);
            }
            thread::sleep(POLL);
        }
    });
}

/// Opens `path` in VS Code or Cursor, preferring the editor that already has
/// it open. Falls back to revealing the folder.
#[tauri::command]
pub fn open_project(app: AppHandle, path: String, editor: Option<String>) -> Result<(), String> {
    let hub = app.state::<DevHub>();
    let preferred = editor.or_else(|| {
        hub.current()
            .workspaces
            .iter()
            .find(|workspace| workspace.path == path)
            .map(|workspace| workspace.editor.clone())
    });

    let candidates: Vec<&Editor> = match preferred {
        Some(id) => EDITORS.iter().filter(|editor| editor.id == id).collect(),
        None => EDITORS.iter().collect(),
    };
    for editor in candidates {
        if crate::platform::open_in_editor(editor.id, &path).is_ok() {
            return Ok(());
        }
    }
    crate::shelf::reveal_file(app, path)
}
