# Bangs for Apple Watch

A watch-only app (no iPhone app needed) that talks to Bangs on a Mac over the
local network. Four pages, swiped left and right:

- **正在播放 / Now playing**: artwork, title, the line being sung (with its
  translation), progress, and previous / play-pause / next
- **会话 / Sessions**: Claude Code and Codex sessions by project, busy, waiting
  or idle. The watch taps your wrist when one stops working, the same moment
  the notch opens on the Mac.
- **待办 / To-do**: the notch's list. Tap a line when it is done; type, scribble
  or dictate a new one at the top.
- **连接 / Connection**: which Mac, whether it is answering, and unpairing.

The watch only updates while the app is on screen. There is no push from the
Mac without a server in between, so nothing arrives with the app closed.

## Run it

Requires Xcode 16 or later (the project uses folder-synchronized groups) and
watchOS 10 or later.

1. On the Mac, turn on **Bangs tray icon → 手表 / Watch → 允许手表连接**. The
   submenu then shows the Mac's address and a six digit pairing code. macOS may
   ask whether Bangs may accept incoming connections; allow it.
2. Open `watch/BangsWatch.xcodeproj`, pick the **BangsWatch** scheme and a
   watch simulator, and run.
3. On the watch, enter the address (`127.0.0.1` works from the simulator) and
   the code, then **配对**.

The project signs with team `W8L8ZJ3N2P`. On a real watch, keep the watch,
or the iPhone it is paired with, on the same Wi-Fi as the Mac. Finding the
Mac through iCloud needs the setup below and the watch signed in to the same
Apple ID; a real watch is the easy way to try it, since a watch simulator
only has iCloud through a signed-in iPhone simulator.

## Same Apple ID, no code

A Mac build that carries the iCloud entitlement writes its address and a
token into the private CloudKit database of the Apple ID it is signed in to
(container `iCloud.com.gxlself.bangs`, zone `BangsWatch`, one `BangsMac`
record per Mac with an encrypted `payload`). The watch's pairing screen
reads it and connects on its own when it finds exactly one Mac, or lists them
when there are several. When a Mac paired this way stops answering, the watch
looks again, so a new DHCP address or a new token is picked up by itself.
The pairing code stays for a Mac on another Apple ID, Windows, or a build
without iCloud (`pnpm tauri dev` is one: it is not signed).

One-time setup, all for team `W8L8ZJ3N2P`:

1. **Container**: developer.apple.com → Certificates, IDs & Profiles →
   Identifiers → iCloud Containers → add `iCloud.com.gxlself.bangs`.
2. **App IDs**: enable iCloud with CloudKit on `com.gxlself.bangs` (macOS)
   and pick that container. Xcode's automatic signing does the same for
   `com.gxlself.bangs.watchkitapp` the first time the watch app runs on a
   device; if it does not, enable it there too.
3. **Profile**: Profiles → add a *Developer ID* profile for
   `com.gxlself.bangs` with your Developer ID Application certificate, and
   save it as `src-tauri/embedded.provisionprofile` (ignored by git).
4. **Schema**: CloudKit Console → `iCloud.com.gxlself.bangs` → Development →
   Schema → Record Types → add `BangsMac` with one field `payload` of type
   *Encrypted String*, then **Deploy Schema Changes** to Production. Both
   apps use the Production environment, which never creates types by itself.
5. **Build**: `scripts/build-release.sh --mac` picks the profile up when it is
   there; by hand it is
   `APPLE_SIGNING_IDENTITY=… pnpm tauri build --config src-tauri/tauri.icloud.conf.json`.
   Without the profile the iCloud entitlement must stay out (macOS refuses to
   launch an app that claims it unprovisioned), which is why the plain build
   does not use `entitlements.icloud.plist`.

Then turn on **允许手表连接** on the Mac; the tray adds a line saying watches on
this Apple ID connect on their own. Publishing errors show in Console.app
under `[watch] iCloud`.

## How it talks to the Mac

`src-tauri/src/watch.rs` serves plain HTTP on port `17651` on every interface
while the switch is on, and not at all while it is off.

| Request | What it does |
| --- | --- |
| `POST /watch/pair` `{"code":"123456"}` | Trades the tray's code for `{"token","host"}` |
| `GET /watch/state?since=<rev>` | Everything on the watch's pages. With `since`, waits up to 20 s for a change |
| `GET /watch/artwork` | The current artwork's bytes; refetch when `media.artworkId` changes |
| `POST /watch/media` `{"action":"toggle"}` | `toggle`, `next` or `previous` |
| `POST /watch/todos` `{"text":"…"}` | Adds a to-do |
| `DELETE /watch/todos/<id>` | Ticks one off |
| `DELETE /watch/pair` | Forgets this watch |

Every request but pairing carries `Authorization: Bearer <token>`. A pairing
code is good for one watch and five wrong guesses, then the tray shows a new
one; **取消所有手表配对** in the tray forgets every watch. Requests with an
`Origin` header are refused, so a web page cannot reach the API from a browser.
The watch never sees the clipboard, the shelf, file paths or the board.

To try the API without a watch:

```bash
curl -s -X POST 127.0.0.1:17651/watch/pair -d '{"code":"<code from the tray>"}'
curl -s 127.0.0.1:17651/watch/state -H "Authorization: Bearer <token>"
```

`pnpm tauri dev` also prints the pairing code on stderr when the port opens.
