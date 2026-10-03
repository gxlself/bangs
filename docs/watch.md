# The watch API

The Apple Watch app (`watch/`) talks to Bangs over the local network. It is a
glance and a remote, nothing more: what is playing with its lyrics, the Claude
Code and Codex sessions, the to-do list and the board — with play/pause/skip,
adding a to-do and ticking one off. It never sees a path, the clipboard, the
shelf or a link, and it cannot open anything on the computer.

## Turning it on

The port is closed until the tray menu opens it: **Apple Watch → 允许手表连接 /
Allow the watch to connect**. The same submenu then shows

- the address to type on the watch — `192.168.1.20:17651`, port included;
- a six-digit pairing code.

On the watch, enter both and tap **配对 / Pair**. The watch keeps the token it
gets back in its keychain and does not need the code again.

The code is good for one pairing: once a watch pairs, the menu shows a new one.
Five wrong codes in a row replace it too, and after any wrong code nobody gets
another guess for two seconds — four after the next five, then eight, up to ten
minutes, until a watch pairs or the switch is turned off and on again. **忘掉已配对的手表 / Forget paired watches** in
the same submenu unpairs every watch; unpairing on the watch drops its own.
Paired watches are kept in `watch-devices.json`, next to `settings.json`.

Turning the switch off closes the port within a fraction of a second. Paired
watches stay paired and reconnect when it is turned back on.

## What travels

Plain HTTP on port **17651**, on every interface. A watch app cannot be talked
into trusting a self-signed certificate, so there is no TLS: anyone on the same
network who watches the traffic can read what the watch reads, and replay its
token. That is the trade for a remote that works without an account or a
server. Keep the switch off on networks you do not trust.

Every request except `/v1/hello` and `POST /v1/pair` carries
`Authorization: Bearer <token>`. A request with an `Origin` header — any web
page — is refused outright, as on the loopback API.

## Endpoints

| | | |
| --- | --- | --- |
| `GET` | `/v1/hello` | `{"app":"bangs","version":"0.2.6","host":"MacBook Pro"}` — no token needed; lets the watch check the address before it asks for a code. |
| `POST` | `/v1/pair` | `{"code":"123456","name":"Apple Watch"}` → `{"token":"…","host":"MacBook Pro"}`. `401` for a wrong code, `429` within two seconds of one, `403` when the port is closed. |
| `DELETE` | `/v1/pair` | Unpairs the watch making the request. |
| `GET` | `/v1/state` | Everything the watch shows; see below. |
| `GET` | `/v1/artwork` | The current album art as the image itself, `404` when there is none. |
| `GET` | `/v1/lyrics` | `{"lines":[{"at":12.3,"text":"…","translation":"…"}]}` for the track playing now; empty when the lines have no timestamps or belong to another track. |
| `POST` | `/v1/media` | `{"action":"toggle"}`, `"next"`, `"previous"`, or `{"action":"seek","position":42}`. |
| `POST` | `/v1/todos` | `{"text":"…"}` adds a line at the top; `409` when the list already has as many open lines as it takes. |
| `POST` | `/v1/todos/<id>/done` | Ticks a line off, as a tap on the notch does. It stays done there (and on the iPhone) until cleared, and leaves `todos` in `/v1/state`. Ticking a line that is already done changes nothing. |

`/v1/state`:

```json
{
  "host": "MacBook Pro",
  "now": 1790000000000,
  "media": {
    "title": "…", "artist": "…", "album": "…", "appName": "Music",
    "playing": true, "duration": 241.0,
    "elapsed": 63.2, "elapsedAt": 1789999998000,
    "artwork": "9f2c…", "lyrics": "41d0…"
  },
  "sessions": [
    { "id": "…", "agent": "claude", "name": "…", "project": "bangs",
      "status": "busy", "detail": null, "updatedAt": 1789999990000 }
  ],
  "todos": [ { "id": "…", "text": "…", "createdAt": 1789999000000 } ],
  "activities": [
    { "id": "ci", "title": "CI 跑完了", "subtitle": "3 个用例挂了",
      "icon": "code", "progress": null, "updatedAt": 1789999000 }
  ]
}
```

- `now` and `elapsedAt` are the computer's clock in Unix milliseconds. The
  position right now is `elapsed + (now - elapsedAt) / 1000` while `playing`;
  the watch carries it forward on its own clock after correcting for the
  difference.
- `artwork` and `lyrics` are fingerprints, not content. Fetch `/v1/artwork` or
  `/v1/lyrics` when one changes; both are `null` when there is nothing to fetch.
- `todos` holds the open lines only, newest first.
- A session's `id` is a fingerprint of the one Bangs keeps, which for Codex is
  a file path; it is stable for as long as the session lives.
- `status` is `busy`, `waiting` or `idle`; `agent` is `claude` or `codex`.
  Clients should treat values they do not know as `idle` and show the raw
  agent name, so a newer Bangs does not break an older watch.

## Trying it without a watch

```bash
curl -sS 192.168.1.20:17651/v1/hello

TOKEN=$(curl -sS -X POST 192.168.1.20:17651/v1/pair -d '{"code":"123456","name":"curl"}' \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["token"])')

curl -sS 192.168.1.20:17651/v1/state -H "Authorization: Bearer $TOKEN"
curl -sS -X POST 192.168.1.20:17651/v1/media -H "Authorization: Bearer $TOKEN" -d '{"action":"toggle"}'
curl -sS -X POST 192.168.1.20:17651/v1/todos -H "Authorization: Bearer $TOKEN" -d '{"text":"从 curl 加的"}'
```

`curl` counts as a paired watch until it is forgotten from the tray.
