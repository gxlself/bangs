import { create } from "zustand";

import { native, type Todo } from "../lib/native";

/** How long a line takes to blow away; in step with `dust` in styles.css. */
const DUST_MS = 940;

interface TodoStore {
  /** Open lines newest first, then done lines most recently done first (the native order). */
  items: Todo[];
  /** Lines coming apart right now, so the panel can draw the dust. */
  dusting: string[];
  /** True while a field in the panel holds the keyboard; see TodoPanel. */
  typing: boolean;
  update(items: Todo[]): void;
  add(text: string): Promise<void>;
  /** Ticks a line off, or brings a done one back. */
  toggle(id: string): void;
  /** Deletes a line: it comes apart first. */
  remove(id: string): void;
  /** Deletes every done line, all coming apart together. */
  clearDone(): void;
  setTyping(typing: boolean): void;
}

export const useTodos = create<TodoStore>((set, get) => {
  /** Turns `ids` to dust, then lets `gone` take them off the native list. */
  const blowAway = (ids: string[], gone: () => Promise<void>) => {
    const fresh = ids.filter((id) => !get().dusting.includes(id));
    if (!fresh.length) return;
    set({ dusting: [...get().dusting, ...fresh] });
    // The line goes for good, but only once there is nothing left of it:
    // the native list is what makes the row disappear.
    window.setTimeout(() => {
      gone().catch((error) => {
        console.warn("todo delete failed", error);
        // Nothing took the rows away, so put them back rather than leave
        // invisible lines on the list.
        set({ dusting: get().dusting.filter((id) => !fresh.includes(id)) });
      });
    }, DUST_MS);
  };

  return {
    items: [],
    dusting: [],
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
      native.todoToggle(id).catch((error) => console.warn("todo toggle failed", error));
    },

    remove(id) {
      blowAway([id], () => native.todoRemove(id));
    },

    clearDone() {
      const done = get().items.filter((item) => item.done).map((item) => item.id);
      blowAway(done, () => native.todoClearDone());
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
