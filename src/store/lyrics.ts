import { create } from "zustand";

import type { LyricLine, Lyrics } from "../lib/native";

interface LyricsStore extends Lyrics {
  update(next: Lyrics): void;
}

export const useLyrics = create<LyricsStore>((set) => ({
  track: "",
  timed: false,
  wordTimed: false,
  status: "idle",
  source: null,
  fromCache: false,
  lines: [],
  update(next) {
    set(next);
  },
}));

/** Index of the line being sung at `elapsed` seconds, or -1 before the first. */
export function lineAt(lines: LyricLine[], elapsed: number): number {
  let index = -1;
  for (let i = 0; i < lines.length; i += 1) {
    if (lines[i].at > elapsed + 0.05) break;
    index = i;
  }
  return index;
}
