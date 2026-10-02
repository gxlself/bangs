import { useEffect, type CSSProperties } from "react";

import { refreshHover } from "../lib/hover";
import { baseNotch, cornerRadii, hitRect, notchSize } from "../lib/layout";
import { native } from "../lib/native";
import { useNow } from "../lib/useNow";
import { waitingSessions, useDev } from "../store/dev";
import { lineAt, useLyrics } from "../store/lyrics";
import { elapsedAt, isMediaLive, useMedia } from "../store/media";
import { useActivities } from "../store/activities";
import { useNotch } from "../store/notch";
import { useShelf } from "../store/shelf";
import { TODO_REMINDER_MS, useTodos } from "../store/todos";
import { CompactView, type Activity } from "./CompactView";
import { DropView } from "./DropView";
import { ExpandedView } from "./ExpandedView";

/** How long a new to-do stays beside the collapsed notch. */

export function Notch() {
  const mode = useNotch((s) => s.mode);
  const screen = useNotch((s) => s.screen);
  const settings = useNotch((s) => s.settings);
  const hovering = useNotch((s) => s.hovering);
  const media = useMedia((s) => s.media);
  const lastActiveAt = useMedia((s) => s.lastActiveAt);
  const clock = useMedia((s) => s.clock);
  const shelfCount = useShelf((s) => s.items.length);
  const waiting = useDev((s) => waitingSessions(s.sessions).length);
  const lyricLines = useLyrics((s) => s.lines);
  const docked = useActivities((s) => s.items[0] ?? null);
  // The newest line still to do; a done line is nothing to be reminded of.
  const newestTodo = useTodos((s) => s.items.find((item) => !item.done && !s.dusting.includes(item.id)) ?? null);
  const lyricsTimed = useLyrics((s) => s.timed);

  // The lyric sweep needs a fast clock; everything else here is slow.
  const now = useNow(media?.playing && lyricLines.length ? 80 : 5_000, true, true);

  const live = isMediaLive(media, lastActiveAt, now) ? media : null;
  const elapsed = media?.playing ? elapsedAt(media, now, clock) : null;
  const lyricIndex = live?.playing && lyricsTimed && elapsed != null ? lineAt(lyricLines, elapsed) : -1;
  const lyricLine = lyricLines[lyricIndex];
  const showTranslation = settings.lyricsTranslationEnabled && Boolean(lyricLine?.translation?.trim());
  const lyric =
    lyricLine && elapsed != null
      ? {
          text: lyricLine.text,
          translation: showTranslation ? lyricLine.translation : null,
          showTranslation,
          words: lyricLine.words,
          from: lyricLine.at,
          to: lyricLines[lyricIndex + 1]?.at ?? lyricLine.at + 6,
          elapsed,
          revision: clock.revision,
        }
      : null;

  // A to-do is a reminder, not live activity: it rides the strip for a while
  // after it is written down and then lets the notch rest again. The list
  // itself is always a hover away. Music that is actually playing keeps the
  // strip — a lyric mid-line is not worth interrupting — but a player left
  // paused half an hour ago does not.
  const fresh = newestTodo && now - newestTodo.createdAt < TODO_REMINDER_MS;
  const todo = fresh && !live?.playing ? newestTodo : null;

  const activity: Activity = { attention: waiting, docked, media: live, lyric, todo, shelfCount };
  const hasActivity =
    activity.attention > 0 ||
    !!activity.docked ||
    !!activity.media ||
    !!activity.todo ||
    activity.shelfCount > 0;

  const base = baseNotch(screen);
  const size = notchSize(
    mode,
    screen,
    settings,
    hasActivity,
    mode === "compact" && !!lyric,
    mode === "compact" && !!lyric?.showTranslation,
  );
  const hit = hitRect(size);
  const { ear, bottom } = cornerRadii(mode, size);

  useEffect(() => {
    native.setHitRect(hit.width, hit.height).catch(() => {});
  }, [hit.width, hit.height]);

  // What is under a resting pointer changes when the panel opens or closes,
  // and the shape it has just grown into is the one that counts.
  useEffect(() => {
    refreshHover();
    const settled = window.setTimeout(refreshHover, 420);
    return () => window.clearTimeout(settled);
  }, [mode]);

  const style = {
    width: size.width,
    height: size.height,
    "--ear": `${ear}px`,
    "--radius": `${bottom}px`,
  } as CSSProperties;

  return (
    <div className="stage">
      <div
        className={`notch notch--${mode}${hovering ? " is-hovering" : ""}`}
        style={style}
        onClick={mode === "compact" ? () => useNotch.getState().expand(undefined, true) : undefined}
      >
        <div className="notch__clip">
          {/* Content is laid out at the target size so it never reflows while the shape animates. */}
          <div key={mode === "success" ? "drop" : mode} className="notch__content" style={{ width: size.width, height: size.height }}>
            {mode === "compact" && <CompactView activity={activity} screen={screen} />}
            {mode === "expanded" && <ExpandedView base={base} notchGap={screen.hasNotch ? base.width : 0} />}
            {(mode === "drop" || mode === "success") && <DropView base={base} success={mode === "success"} />}
          </div>
        </div>
      </div>
    </div>
  );
}
