import type { DragDropEvent } from "@tauri-apps/api/webview";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useEffect } from "react";

import { Notch } from "./components/Notch";
import { notify, playPing } from "./lib/alerts";
import { hoverAt } from "./lib/hover";
import { setLanguage, t } from "./lib/i18n";
import { events, native, type DevState } from "./lib/native";
import { useDev } from "./store/dev";
import { useLyrics } from "./store/lyrics";
import { useMedia } from "./store/media";
import { useActivities } from "./store/activities";
import { useTodos } from "./store/todos";
import { useClipboard } from "./store/clipboard";
import { useNotch, type Section } from "./store/notch";
import { syncShelfNow, useShelf, type AddResult } from "./store/shelf";

export default function App() {
  const ready = useNotch((s) => s.ready);

  useEffect(() => {
    let disposed = false;
    const subscriptions = [
      events.hover((inside) => {
        if (!inside) hoverAt(null);
        useNotch.getState().hoverChanged(inside);
      }),
      events.pointer(hoverAt),
      events.outsideClick(() => useNotch.getState().outsideClicked()),
      events.screen((screen) => useNotch.setState({ screen })),
      events.settings((settings) => useNotch.setState({ settings })),
      events.language((language) => {
        setLanguage(language);
        // Nothing else changed, so nudge the tree into re-rendering the words.
        useNotch.setState((state) => ({ settings: { ...state.settings } }));
      }),
      events.media((media) => useMedia.getState().update(media)),
      events.lyrics((lyrics) => useLyrics.getState().update(lyrics)),
      events.dev(handleDev),
      events.clipboard((clipboard) => useClipboard.getState().update(clipboard)),
      events.activities((items) => useActivities.getState().update(items)),
      events.todos((todos) => useTodos.getState().update(todos)),
      getCurrentWebview().onDragDropEvent((event) => void handleDragDrop(event.payload)),
      // Windows catches its own drops; see src-tauri/src/platform/win_drop.rs.
      events.dragEnter((paths) => {
        if (paths.length) useNotch.getState().dragEntered();
      }),
      events.dragLeave(() => useNotch.getState().dragLeft()),
      events.drop((paths) => void handleDrop(paths)),
    ];

    // Subscribe first, then snapshot, so no update falls in between.
    Promise.all(subscriptions)
      .then(async () => {
        const boot = await native.bootstrap();
        if (disposed) return;
        setLanguage(boot.language);
        useMedia.getState().update(boot.media);
        useLyrics.getState().update(boot.lyrics);
        useDev.getState().update(boot.dev);
        useClipboard.getState().update(boot.clipboard);
        useActivities.getState().update(boot.activities);
        useTodos.getState().update(boot.todos);
        await useShelf.getState().refresh();
        syncShelfNow();
        useNotch.getState().init(boot);
        // Not requestAnimationFrame: hidden webviews never run frame callbacks.
        window.setTimeout(() => void native.ready(), 50);
      })
      .catch((error) => console.error("bootstrap failed", error));

    return () => {
      disposed = true;
      subscriptions.forEach((subscription) => subscription.then((unlisten) => unlisten()));
    };
  }, []);

  return ready ? <Notch /> : null;
}

async function handleDragDrop(event: DragDropEvent) {
  const notch = useNotch.getState();
  switch (event.type) {
    case "enter":
      if (event.paths.length) notch.dragEntered();
      break;
    case "leave":
      notch.dragLeft();
      break;
    case "drop":
      await handleDrop(event.paths);
      break;
  }
}

async function handleDrop(paths: string[]) {
  const notch = useNotch.getState();
  if (notch.draggingOut) return;
  notch.dropped(dropMessage(await useShelf.getState().add(paths)));
}

/** How long an alert keeps the panel open when nobody looks at it. */
const ALERT_OPEN_MS = 8_000;
let alertTimer: number | undefined;

/** Opens the notch for an alert, then gets out of the way on its own. */
function alertWith(section: Section) {
  useNotch.getState().expand(section, true);
  window.clearTimeout(alertTimer);
  alertTimer = window.setTimeout(() => {
    const notch = useNotch.getState();
    if (notch.mode !== "expanded") return;
    // The pin was the alert's, not the user's: give it back either way, so a
    // panel that opened by itself never stays open by itself.
    useNotch.setState({ pinned: false });
    if (!notch.hovering) notch.collapse();
  }, ALERT_OPEN_MS);
}

/** Claude Code sessions run unattended, so surface the ones that stopped. */
function handleDev(next: DevState) {
  const finished = useDev.getState().update(next);
  const notch = useNotch.getState();
  if (!notch.settings.notifyClaudeIdle) return;

  const session = finished.find((entry) => entry.status === "waiting") ?? finished[0];
  if (!session) return;
  playPing();
  void notify(
    session.status === "waiting"
      ? t(`${session.project} 在等你回复`, `${session.project} is waiting for you`)
      : t(`${session.project} 跑完了`, `${session.project} has finished`),
    session.detail ?? session.name,
  );
  alertWith("dev");
}

function dropMessage({ added, duplicates, rejectedForSpace }: AddResult) {
  if (added > 0) {
    return rejectedForSpace > 0
      ? t(`已暂存 ${added} 个，暂存架满了`, `Kept ${added}, the shelf is full`)
      : t(`已暂存 ${added} 个文件`, `Kept ${added} file${added > 1 ? "s" : ""}`);
  }
  if (rejectedForSpace > 0) return t("暂存架满了", "The shelf is full");
  return duplicates > 0 ? t("已经在暂存架里", "Already on the shelf") : t("无法暂存这个项目", "That cannot be kept here");
}
