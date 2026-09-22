# Bangs

A dynamic notch for macOS and Windows, built with Tauri 2 + React by gxlself
(bundle id `com.gxlself.bangs`). It sits at the top center of the screen, shows live activity next to
the notch, and expands on hover.

**[Download](https://github.com/gxlself/bangs/releases/latest)** ·
**[bangs site](https://gxlself.github.io/bangs/)** — the macOS build is signed with a Developer ID
and notarized, so it opens on a double click; the Windows installer is unsigned, so SmartScreen
wants "Run anyway" once.

- **Now playing**: artwork, progress and play/pause/skip/seek for the current system media session,
  with timed lyrics — the line being sung shows next to the collapsed notch, two lines with translation in the panel
- **File shelf**: drop files on the notch to park them, drag them back out to other apps
- **Dev panel**: Claude Code and Codex CLI sessions with busy/waiting/idle status in one list, and
  the projects VS Code and Cursor have open, grouped per editor with their git branch. Click a row
  to open that project in the editor. Optional alerts open the panel when a session changes from
  busy to waiting or idle, and close it again after a few seconds if nobody looks.
- **Clipboard**: on macOS the panel mirrors gxlself's own Paste app (`gxlself.paste-tool`) and
  offers to install it when it is missing; on Windows, where there is no Paste, Bangs records the
  history itself. Click an entry to put it back on the clipboard.
- **To-do**: a short list you type into — the one place the notch takes the keyboard, and only while
  the field is focused. Click a line when it is done and it comes apart and blows off the list;
  nothing is kept. A new line waits beside the collapsed notch for a few minutes unless something
  is playing.
- **Tabs**: drag a tab along the bar to put the panels in the order you use them; the order is
  kept across restarts. A tab that only shows up sometimes, like the board, keeps its place.
- **Board**: anything else on the machine can dock a row — a title, a subtitle, a progress bar and
  at most a link — by writing a JSON file, or by posting to a loopback endpoint. The newest row
  shows beside the collapsed notch. See [docs/plugins.md](docs/plugins.md).

Screens with a hardware notch get wings around it; other screens get a virtual notch, which shrinks
to a thin bar when nothing is happening (the tray menu can turn that off). Full-screen video, games
and presentations get the screen to themselves: the notch collapses to that bar on macOS and hides
altogether on Windows — unless the screen has a real cutout, where it was never in the way.

## Develop

Requires Node.js (`^20.19.0 || >=22.12.0`), pnpm and Rust. macOS also needs the Xcode command line tools.

```bash
pnpm install
pnpm tauri dev            # run
pnpm tauri build          # .app/.dmg on macOS, .msi/.exe on Windows
```

Build each platform on that platform (Windows installers cannot be produced on macOS).

The tray menu — the menu bar icon on macOS, the notification area icon on Windows — controls
show/hide, expand on hover, idle bar, lyrics, lyric translations, Claude alerts, display, language
and launch at login.

Bangs speaks Chinese and English, following the system language unless the Language submenu pins it
to one (`src-tauri/src/i18n.rs`, `src/lib/i18n.ts`). WKWebView reports the app's own language rather
than the system's, so the native side resolves it and hands it to the webview. Its last
entry is the version: it asks GitHub for the newest release at start-up and every six hours, says
"有新版本 vX.Y.Z" when this build is behind, and opens the release page when clicked
(`src-tauri/src/update.rs`). Nothing is ever downloaded or replaced without the user.

## Release

```bash
scripts/version.sh 0.2.0        # one version across package.json, tauri.conf.json and Cargo.toml
scripts/build-release.sh        # .dmg + .app.zip here, .exe + .msi over ssh on the Windows box
scripts/publish-site.sh         # site/ to the gh-pages branch
```

`scripts/publish-release.md` has the full checklist: the asset names both the site and the in-app
version check depend on, and the one command that stores the notarization credentials
(`BANGS_NOTARY_PROFILE`). Signing happens automatically from the keychain's Developer ID; the
hardened runtime needs `src-tauri/entitlements.plist`, whose Apple Events entitlement is what keeps
the Spotify panel working in a notarized build.

Lyrics query QQ Music and NetEase concurrently, rank candidates by title, artist, album, duration and
version, then prefer word-timed results with well-aligned translations. Reliable translations can be
merged across providers after a global timeline offset is verified. Only track metadata is sent; results
use expiring on-disk caches, can be refreshed from the music panel, and the tray menu can turn lookup off.

The clipboard panel asks Paste for its panel with `open pasteg://panel`, which Paste answers in
`AppDelegate.application(_:open:)`; older Paste builds only get activated instead.

## How it works

| | macOS | Windows |
|---|---|---|
| Window | NSPanel (`tauri-nspanel`), non-activating, status level above the menu bar | Topmost, `focusable: false` (`WS_EX_NOACTIVATE`), no taskbar entry; hides during full-screen apps |
| Now playing | MediaRemote, loaded into `/usr/bin/perl` (see below) | `GlobalSystemMediaTransportControlsSessionManager` |
| Notch size | `NSScreen.safeAreaInsets` / `auxiliaryTop*Area` | Virtual only |
| Dev panel | Claude Code / Codex session state and VS Code / Cursor state | Same, using Windows application-data paths |
| Clipboard | Paste's Core Data store, read-only | Bangs' own history |
| Full screen | A layer-0 window covering the display (`CGWindowListCopyWindowInfo`) | `SHQueryUserNotificationState` |
| Keyboard | The panel is made key while the to-do field is focused, and resigns it after | `WS_EX_NOACTIVATE` comes off for as long, then focus goes back |
| Lyrics | QQ Music and NetEase, cached on disk | Same |

The host window is a fixed 640 x 280 transparent window in logical pixels. The visible notch animates
inside it, and a native thread polls the cursor every 33 ms (`src-tauri/src/geometry.rs`) to:

- make the window click-through everywhere except the current notch rect,
- emit hover and outside-click events,
- stream the pointer position, because WKWebView ignores mouse moves in a panel that is not key.
  CSS therefore uses `[data-hover]` (set by `src/lib/hover.ts`) instead of `:hover`.

**MediaRemote on macOS 15.4+** requires an Apple-entitled process. `src-tauri/native/macos/media_bridge.m`
is built by `src-tauri/build.rs` into `src-tauri/resources/libbangs_media.dylib`, loaded into
`/usr/bin/perl`, and talks to the app over stdio (JSON lines out, commands in). This relies on system
behavior Apple could change in a future release.

The dev integration polls every two seconds, reading `~/.claude/sessions/*.json`, the newest
`~/.codex/sessions` rollout logs (a session counts as running while its log has an unfinished
`task_started`), the editors' `User/globalStorage/storage.json`, and project `.git/HEAD` read-only. It checks running
processes to ignore stale sessions and closed editors. Opening a project uses the editor on macOS
or its `code` / `cursor` CLI on Windows, with a folder-reveal fallback.

On macOS the clipboard panel opens Paste's local Core Data store (`PasteTool.sqlite`) read-only and
polls the latest 24 entries every three seconds, checking Paste's sandbox container first. Bangs
never writes to that database; picking an entry writes to the system clipboard — a picture goes on
as PNG and TIFF, so it pastes anywhere — while file entries hand over to Paste's own panel
(`pasteg://panel`), which is what holds them.

On Windows there is no Paste, so Bangs keeps the history: it polls `GetClipboardSequenceNumber`,
stores what changed (text and dropped file paths, up to 200 entries) in the app data directory, and
writes an entry back when it is picked. Content that apps mark private with
`ExcludeClipboardContentFromMonitorProcessing` or `CanIncludeInClipboardHistory` — password managers
do — is skipped, and the tray menu can stop the recording altogether.

```text
src/                       React UI
  App.tsx                  native events, drag/drop and agent alerts
  components/              Notch shell, compact/expanded views, music and shelf panels
    DevPanel.tsx           agent sessions and per-editor project rows
    ClipboardPanel.tsx     clipboard history, copy, clear and install actions
  store/                   zustand stores: notch state machine, media, shelf
    dev.ts                 agent sessions, workspaces and busy-to-idle/waiting detection
    clipboard.ts           clipboard history, copy feedback and platform actions
    TodoPanel.tsx          the to-do list and the field that takes the keyboard
  lib/layout.ts            notch sizes per mode (keep WINDOW in sync with geometry.rs)
  lib/hover.ts             pointer-driven [data-hover] workaround
    lyrics.ts              current lines and the line-at-time lookup
src-tauri/src/
  lib.rs                   app setup, bootstrap and commands
  geometry.rs              window placement, hit rect, cursor tracker, display watcher
  platform/{mac,win}.rs     window setup, cursor, notch metrics, editor launch, full-screen handling
  media/{mod,mac,win}.rs    shared media state and platform now-playing providers
  dev.rs                   read-only Claude/Codex/editor polling and project opening
  lyrics/mod.rs            lyric lookup, LRC/QRC parsing and the on-disk cache
  lyrics/qrc.rs            vendored QRC decrypter (per-character timings)
  media/spotify.rs         Spotify over AppleScript, for what the session leaves out
  update.rs                the newest GitHub release, for the tray
  clipboard/mod.rs         shared clipboard panel state and commands
  clipboard/mac.rs         read-only Paste store, copy and panel hand-off
  clipboard/win.rs         Bangs' own clipboard history
  todos.rs                 the to-do list, saved in <config>/todos.json
  shelf.rs                 file metadata, open/reveal, drag preview
  tray.rs, settings.rs      tray menu and persisted settings
site/                      the landing page published to gh-pages
scripts/generate-icons.swift  app/tray/drag icons (then `pnpm tauri icon src-tauri/icons/app-icon.png`)
scripts/version.sh, build-release.sh, publish-site.sh, windows-build.ps1  release plumbing
```
