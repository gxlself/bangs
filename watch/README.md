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

On a real watch, pick your team under *Signing & Capabilities* first, enter
the Mac's LAN address from the tray, and keep the watch, or the iPhone it is
paired with, on the same Wi-Fi as the Mac.

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
