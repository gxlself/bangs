import { create } from "zustand";

import { native, type Todo } from "../lib/native";

/** How long a line takes to blow away; in step with `dust` in styles.css. */
export const DUST_MS = 940;
/** With "Clear done", each next row starts this much later, up to `DUST_STAGGER_MAX_MS`. */
export const DUST_STAGGER_MS = 40;
const DUST_STAGGER_MAX_MS = 320;
/** A new line rides the collapsed notch this long (see Notch.tsx and relevantSection). */
export const TODO_REMINDER_MS = 5 * 60_000;
/** Open lines the list holds; in step with MAX_TODOS in src-tauri/src/todos.rs. */
export const MAX_OPEN = 60;
/** How long a ticked line stays where it was before it sinks to the done ones. */
const HOLD_MS = 1200;

interface TodoStore {
  /** Open lines newest first, then done lines most recently done first (the native order). */
  items: Todo[];
  /** Lines coming apart right now, so the panel can draw the dust. */
  dusting: string[];
  /**
   * The order on screen while a tick is settling: a ticked line stays under the pointer for
   * a moment instead of jumping away, so a second click lands on the line it was meant for.
   */
  held: string[] | null;
  /** True while a field in the panel holds the keyboard; see TodoPanel. */
  typing: boolean;
  update(items: Todo[]): void;
  add(text: string): Promise<void>;
  /** Ticks a line off, or brings a done one back. */
  toggle(id: string): void;
  /** Deletes a line: it comes apart first. */
  remove(id: string): void;
  /** Deletes every done line, coming apart one after another. */
  clearDone(): void;
  setTyping(typing: boolean): void;
}

let holdTimer: number | undefined;

export const useTodos = create<TodoStore>((set, get) => {
  /** Turns `ids` to dust, then lets `gone` take them off the native list.
   *  `gone` may answer with the ids it did take off: the others come back. */
  const blowAway = (ids: string[], gone: () => Promise<string[] | void>) => {
    const fresh = ids.filter((id) => !get().dusting.includes(id));
    if (!fresh.length) return;
    set({ dusting: [...get().dusting, ...fresh] });
    const stagger = Math.min((fresh.length - 1) * DUST_STAGGER_MS, DUST_STAGGER_MAX_MS);
    const restore = (back: string[]) => {
      if (back.length) set({ dusting: get().dusting.filter((id) => !back.includes(id)) });
    };
    // The line goes for good, but only once there is nothing left of it:
    // the native list is what makes the row disappear.
    window.setTimeout(() => {
      gone()
        .then((taken) => {
          // Unticked on the phone while it blew away: it stays, so it shows.
          if (taken) restore(fresh.filter((id) => !taken.includes(id)));
        })
        .catch((error) => {
          console.warn("todo delete failed", error);
          // Nothing took the rows away, so put them back rather than leave
          // invisible lines on the list.
          restore(fresh);
        });
    }, DUST_MS + stagger);
  };

  return {
    items: [],
    dusting: [],
    held: null,
    typing: false,

    update(items) {
      // Whatever the native list no longer has is done blowing away.
      const alive = new Set(items.map((item) => item.id));
      set({ items, dusting: get().dusting.filter((id) => alive.has(id)) });
    },

    async add(text) {
      if (!text.trim()) return;
      await native.todoAdd(text).catch((error) => console.warn("todo add failed", error));
    },

    toggle(id) {
      if (get().dusting.includes(id)) return;
      set({ held: get().held ?? get().items.map((item) => item.id) });
      window.clearTimeout(holdTimer);
      holdTimer = window.setTimeout(() => set({ held: null }), HOLD_MS);
      native.todoToggle(id).catch((error) => console.warn("todo toggle failed", error));
    },

    remove(id) {
      blowAway([id], () => native.todoRemove(id));
    },

    clearDone() {
      const done = get().items.filter((item) => item.done).map((item) => item.id);
      blowAway(done, () => native.todoClearDone(done));
    },

    setTyping(typing) {
      // Asking twice would have the native side remember the notch itself as
      // the window to hand focus back to.
      if (get().typing === typing) return;
      set({ typing });
      native.captureKeyboard(typing).catch((error) => console.warn("keyboard capture failed", error));
    },
  };
});

/** The list in the order it should be on screen: `held` while a tick settles. */
export function shownOrder(items: Todo[], held: string[] | null): Todo[] {
  if (!held) return items;
  const place = new Map(held.map((id, index) => [id, index]));
  // A line that arrived meanwhile is new, so it goes on top.
  return [...items].sort((a, b) => (place.get(a.id) ?? -1) - (place.get(b.id) ?? -1));
}
