import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface ScreenInfo {
  platform: string;
  hasNotch: boolean;
  notchWidth: number;
  notchHeight: number;
  menuBarHeight: number;
  displayName: string;
  /** Something covers this display end to end; never set on a cutout screen. */
  fullscreen: boolean;
}

export interface Settings {
  visible: boolean;
  expandOnHover: boolean;
  idleHandle: boolean;
  notifyClaudeIdle: boolean;
  lyricsEnabled: boolean;
  lyricsTranslationEnabled: boolean;
  clipboardHistory: boolean;
  display: string | null;
  /** "zh" or "en"; null follows the system language. */
  language: string | null;
  /** Tab ids in the order the user dragged them into; empty keeps the default. */
  tabOrder: string[];
}

export interface MediaState {
  title: string;
  artist: string;
  album: string;
  appName: string;
  sourceId: string;
  playing: boolean;
  /** Seconds. */
  duration: number | null;
  /** Seconds, sampled at `elapsedAt`. */
  elapsed: number | null;
  /** Unix milliseconds. */
  elapsedAt: number;
  /** Data URL. */
  artwork: string | null;
}

export interface FileMeta {
  path: string;
  name: string;
  extension: string;
  size: number;
  isDir: boolean;
  isImage: boolean;
}

export interface LyricWord {
  /** Seconds into the track. */
  at: number;
  duration: number;
  text: string;
}

export interface LyricLine {
  /** Seconds into the track. */
  at: number;
  text: string;
  translation: string | null;
  /** Per-character timing, when the source has it. */
  words?: LyricWord[];
}

export type LyricStatus = "idle" | "loading" | "found" | "uncertain" | "notFound";
export type LyricProvider = "qq-qrc" | "qq-lrc" | "netease-yrc" | "netease-lrc";

export interface LyricSource {
  original: LyricProvider;
  translation: LyricProvider | null;
  translationOffset: number | null;
}

export interface Lyrics {
  /** The track these lines belong to. */
  track: string;
  /** True when the accepted result has timed lyrics. */
  timed: boolean;
  /** True when at least part of the result has per-word timing. */
  wordTimed: boolean;
  status: LyricStatus;
  source: LyricSource | null;
  fromCache: boolean;
  lines: LyricLine[];
}

export type SessionStatus = "busy" | "waiting" | "idle";

export type Agent = "claude" | "codex";

export interface AgentSession {
  id: string;
  agent: Agent;
  name: string;
  path: string;
  project: string;
  status: SessionStatus;
  detail: string | null;
  /** Unix milliseconds of the last status change. */
  updatedAt: number;
}

export interface EditorWorkspace {
  editor: "code" | "cursor";
  editorName: string;
  path: string;
  project: string;
  branch: string | null;
  active: boolean;
}

export interface DevState {
  sessions: AgentSession[];
  workspaces: EditorWorkspace[];
}

export type ClipKind = "text" | "image" | "files";
/** Where the history comes from: the Paste app, or Bangs itself. */
export type ClipSource = "paste" | "builtin";

export interface ClipItem {
  id: number;
  kind: ClipKind;
  preview: string;
  app: string | null;
  /** Source app icon as a data URL. */
  icon: string | null;
  pinned: boolean;
  /** Unix milliseconds. */
  createdAt: number;
}

export interface ClipboardState {
  /** False on macOS when Paste is not installed. */
  available: boolean;
  source: ClipSource;
  items: ClipItem[];
}

/** A row another program asked the notch to show; see docs/plugins.md. */
export interface Activity {
  id: string;
  title: string;
  subtitle?: string | null;
  /** A glyph name the notch knows; anything else draws a dot. */
  icon?: string | null;
  /** 0…1, drawn as a bar under the title. */
  progress?: number | null;
  /** http(s) only; the native side opens it, the webview never sees a command. */
  url?: string | null;
  expiresAt?: number | null;
  updatedAt: number;
}

/** One line on the to-do list, kept in `<config>/todos.json`. */
export interface Todo {
  id: string;
  text: string;
  /** Unix milliseconds. */
  createdAt: number;
}

export interface Bootstrap {
  screen: ScreenInfo;
  settings: Settings;
  media: MediaState | null;
  lyrics: Lyrics;
  dev: DevState;
  clipboard: ClipboardState;
  activities: Activity[];
  todos: Todo[];
  dragIcon: string | null;
  /** "zh" or "en", resolved natively. */
  language: string;
}

export type MediaCommand =
  | { action: "toggle" }
  | { action: "next" }
  | { action: "previous" }
  | { action: "seek"; position: number };

export const native = {
  bootstrap: () => invoke<Bootstrap>("bootstrap"),
  ready: () => invoke<void>("notch_ready"),
  setHitRect: (width: number, height: number) => invoke<void>("set_hit_rect", { width, height }),
  /** The webview cannot set the cursor itself here; see src/lib/hover.ts. */
  setCursor: (shape: "default" | "pointer" | "grab" | "grabbing" | "text") => invoke<void>("set_cursor", { shape }),
  /** Only while a field in the panel is focused; see src/components/TodoPanel.tsx. */
  captureKeyboard: (capture: boolean) => invoke<void>("capture_keyboard", { capture }),
  setTabOrder: (order: string[]) => invoke<void>("set_tab_order", { order }),
  media: (command: MediaCommand) => invoke<void>("media_command", { command }),
  refreshLyrics: () => invoke<void>("lyrics_refresh"),
  openProject: (path: string, editor?: string) => invoke<void>("open_project", { path, editor }),
  clipboardUse: (id: number) => invoke<void>("clipboard_use", { id }),
  clipboardOpen: () => invoke<void>("clipboard_open"),
  clipboardClear: () => invoke<void>("clipboard_clear"),
  /** Loads the next page; false once everything is loaded. */
  clipboardMore: () => invoke<boolean>("clipboard_more"),
  clipboardInstall: () => invoke<void>("clipboard_install"),
  /** Opens the link on a plugin row; the native side looks the link up itself. */
  activityOpen: (id: string) => invoke<void>("activity_open", { id }),
  todoAdd: (text: string) => invoke<void>("todo_add", { text }),
  todoRemove: (id: string) => invoke<void>("todo_remove", { id }),
  inspectFiles: (paths: string[]) => invoke<FileMeta[]>("shelf_inspect", { paths }),
  openFile: (path: string) => invoke<void>("open_file", { path }),
  revealFile: (path: string) => invoke<void>("reveal_file", { path }),
};

export const events = {
  hover: (handler: (inside: boolean) => void) =>
    listen<boolean>("bangs://hover", (event) => handler(event.payload)),
  /** Window-local pointer position while the cursor is over the notch. */
  pointer: (handler: (point: [number, number]) => void) =>
    listen<[number, number]>("bangs://pointer", (event) => handler(event.payload)),
  outsideClick: (handler: () => void) => listen("bangs://outside-click", () => handler()),
  /** The mouse button went up, wherever it was; see TabBar.tsx. */
  release: (handler: () => void) => listen("bangs://release", () => handler()),
  language: (handler: (language: string) => void) =>
    listen<string>("bangs://language", (event) => handler(event.payload)),
  /** Windows only: drops the notch caught itself (see platform/win_drop.rs). */
  dragEnter: (handler: (paths: string[]) => void) =>
    listen<string[]>("bangs://drag-enter", (event) => handler(event.payload)),
  dragLeave: (handler: () => void) => listen("bangs://drag-leave", () => handler()),
  drop: (handler: (paths: string[]) => void) =>
    listen<string[]>("bangs://drop", (event) => handler(event.payload)),
  screen: (handler: (screen: ScreenInfo) => void) =>
    listen<ScreenInfo>("bangs://screen", (event) => handler(event.payload)),
  settings: (handler: (settings: Settings) => void) =>
    listen<Settings>("settings://changed", (event) => handler(event.payload)),
  media: (handler: (media: MediaState | null) => void) =>
    listen<MediaState | null>("media://update", (event) => handler(event.payload)),
  lyrics: (handler: (lyrics: Lyrics) => void) =>
    listen<Lyrics>("bangs://lyrics", (event) => handler(event.payload)),
  dev: (handler: (dev: DevState) => void) =>
    listen<DevState>("bangs://dev", (event) => handler(event.payload)),
  activities: (handler: (activities: Activity[]) => void) =>
    listen<Activity[]>("bangs://activities", (event) => handler(event.payload)),
  todos: (handler: (todos: Todo[]) => void) =>
    listen<Todo[]>("bangs://todos", (event) => handler(event.payload)),
  clipboard: (handler: (clipboard: ClipboardState) => void) =>
    listen<ClipboardState>("bangs://clipboard", (event) => handler(event.payload)),
};
