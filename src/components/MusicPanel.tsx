import { useMemo, type MouseEvent } from "react";

import { clock } from "../lib/format";
import { t } from "../lib/i18n";
import { native, type LyricLine, type LyricProvider } from "../lib/native";
import { useNow } from "../lib/useNow";
import { lineAt, useLyrics } from "../store/lyrics";
import { elapsedAt, isMediaLive, useMedia } from "../store/media";
import { useNotch } from "../store/notch";
import { Empty } from "./Empty";
import { KaraokeLine } from "./KaraokeLine";
import { MusicIcon, NextIcon, PauseIcon, PlayIcon, PreviousIcon, RefreshIcon } from "./Icons";

export function MusicPanel() {
  const current = useMedia((s) => s.media);
  const lastActiveAt = useMedia((s) => s.lastActiveAt);
  const mediaClock = useMedia((s) => s.clock);
  const send = useMedia((s) => s.send);
  const lyrics = useLyrics();
  const lines = lyrics.lines;
  const showTranslations = useNotch((s) => s.settings.lyricsTranslationEnabled);
  // The sweep over the current line needs a faster clock than the progress bar.
  const now = useNow(current?.playing ? (lines.length ? 80 : 250) : 1000, !!current, true);

  // The same rule the compact strip uses, so the two never disagree about
  // whether a long-paused track still counts as playing.
  const media = isMediaLive(current, lastActiveAt, now) ? current : null;

  if (!media) {
    return <Empty icon={<MusicIcon />} title={t("没有正在播放的音乐", "Nothing is playing")}
        hint={t("在任意播放器里开始播放，就会显示在这里", "Start something in any player and it shows up here")} />;
  }

  const elapsed = elapsedAt(media, now, mediaClock);
  const duration = media.duration;
  const progress = duration && elapsed != null ? elapsed / duration : null;
  const hasLyrics = lines.length > 0;

  const seek = (event: MouseEvent<HTMLDivElement>) => {
    if (!duration) return;
    const rect = event.currentTarget.getBoundingClientRect();
    const ratio = Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width));
    send({ action: "seek", position: ratio * duration });
  };

  return (
    <div className="music">
      <div className="music__art">
        {media.artwork ? <img key={media.artwork} src={media.artwork} alt="" /> : <MusicIcon width={32} height={32} />}
      </div>

      <div className="music__main">
        <div>
          <div className="music__title" title={media.title}>{media.title}</div>
          {/* With lyrics the artist moves to the footer to make room. */}
          {!hasLyrics && <div className="music__artist">{media.artist || media.album || " "}</div>}
        </div>

        {hasLyrics ? (
          <LyricView lines={lines} elapsed={elapsed ?? 0} showTranslations={showTranslations} />
        ) : (
          <div className="lyrics lyrics--status">{statusLabel(lyrics.status)}</div>
        )}

        {progress != null && elapsed != null && duration ? (
          <div className="progress" onClick={seek}>
            <div className="progress__track">
              <div className="progress__fill" style={{ width: `${progress * 100}%` }} />
            </div>
            <div className="progress__times">
              <span>{clock(elapsed)}</span>
              <span>-{clock(duration - elapsed)}</span>
            </div>
          </div>
        ) : (
          <div className="progress progress--empty" />
        )}

        <div className="music__footer">
          <div className="music__meta">
            <span className="music__app">
              {hasLyrics && media.artist ? `${media.artist} · ${media.appName}` : media.appName}
            </span>
            <span className="lyrics__source" title={sourceLabel(lyrics)}>
              {sourceLabel(lyrics)}
            </span>
            <button
              className={`lyrics__refresh${lyrics.status === "loading" ? " is-loading" : ""}`}
              onClick={() => void native.refreshLyrics().catch((error) => console.warn("lyric refresh failed", error))}
              disabled={lyrics.status === "loading"}
              title={t("刷新歌词", "Refresh lyrics")}
            >
              <RefreshIcon width={12} height={12} />
            </button>
          </div>
          <div className="controls">
            <button className="control" onClick={() => send({ action: "previous" })} title={t("上一首", "Previous")}>
              <PreviousIcon width={18} height={18} />
            </button>
            <button className="control control--main" onClick={() => send({ action: "toggle" })} title={media.playing ? t("暂停", "Pause") : t("播放", "Play")}>
              {media.playing ? <PauseIcon width={18} height={18} /> : <PlayIcon width={18} height={18} />}
            </button>
            <button className="control" onClick={() => send({ action: "next" })} title={t("下一首", "Next")}>
              <NextIcon width={18} height={18} />
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function statusLabel(status: ReturnType<typeof useLyrics.getState>["status"]) {
  switch (status) {
    case "loading":
      return t("正在查找歌词…", "Looking for lyrics…");
    case "uncertain":
      return t("歌词匹配不确定", "Lyric match is uncertain");
    case "notFound":
      return t("未找到歌词", "No lyrics found");
    default:
      return "";
  }
}

function providerLabel(provider: LyricProvider) {
  return provider.startsWith("qq-") ? t("QQ 音乐", "QQ Music") : t("网易云", "NetEase");
}

function sourceLabel(lyrics: ReturnType<typeof useLyrics.getState>) {
  if (!lyrics.source) return "";
  let label = providerLabel(lyrics.source.original);
  if (lyrics.wordTimed) label += t(" · 逐字", " · word-timed");
  if (lyrics.source.translation) {
    const translation = providerLabel(lyrics.source.translation);
    label += translation === providerLabel(lyrics.source.original)
      ? t(" · 翻译", " · translation")
      : t(` + ${translation}翻译`, ` + ${translation} translation`);
  }
  if (lyrics.fromCache) label += t(" · 缓存", " · cached");
  return label;
}

/** Height of one lyric row; translations add a second row when enabled. */
const ROW = 20;

interface Row {
  /** Index of the line this row belongs to. */
  line: number;
  text: string;
  translation: boolean;
}

/** A translated lyric occupies a stable original/translation pair. */
function rowsOf(lines: LyricLine[], showTranslations: boolean): { rows: Row[]; rowOfLine: number[] } {
  const rows: Row[] = [];
  const rowOfLine: number[] = [];
  lines.forEach((line, index) => {
    rowOfLine[index] = rows.length;
    rows.push({ line: index, text: line.text, translation: false });
    if (showTranslations && line.translation?.trim()) {
      rows.push({ line: index, text: line.translation, translation: true });
    }
  });
  return { rows, rowOfLine };
}

/**
 * The lyric as a strip that scrolls, so the line being sung slides into the
 * middle instead of the two-line pair swapping its text at once.
 */
export function LyricView({ lines, elapsed, showTranslations }: { lines: LyricLine[]; elapsed: number; showTranslations: boolean }) {
  const { rows, rowOfLine } = useMemo(() => rowsOf(lines, showTranslations), [lines, showTranslations]);
  const index = lineAt(lines, elapsed);
  const line = lines[index];
  // Put the current original first; the second row is its translation or the next original.
  const rowOffset = index < 0 ? 0 : rowOfLine[index];

  return (
    <div className="lyrics">
      <div className="lyrics__scroll" style={{ transform: `translateY(${-rowOffset * ROW}px)` }}>
        {rows.map((row, position) => {
          const current = row.line === index;
          return (
            <div
              key={position}
              className={`lyrics__line${current ? " is-current" : ""}${row.translation ? " lyrics__line--sub" : ""}`}
            >
              {current && line && !row.translation ? (
                <KaraokeLine
                  text={line.text}
                  words={line.words}
                  from={line.at}
                  to={lines[index + 1]?.at ?? line.at + 6}
                  elapsed={elapsed}
                />
              ) : row.text}
            </div>
          );
        })}
      </div>
    </div>
  );
}
