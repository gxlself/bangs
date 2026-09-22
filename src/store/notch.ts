import { create } from "zustand";

import type { Mode } from "../lib/layout";
import { native, type Bootstrap, type ScreenInfo, type Settings } from "../lib/native";
import { hasFreshActivity, useActivities } from "./activities";
import { waitingSessions, useDev } from "./dev";
import { isMediaLive, useMedia } from "./media";
import { useTodos } from "./todos";

export type Section = "music" | "shelf" | "dev" | "paste" | "board" | "todo";

/** The panel's tabs in their default order. */
export const SECTIONS: readonly Section[] = ["music", "shelf", "dev", "paste", "todo", "board"];

/**
 * Every section in the order the user dragged the tabs into. Sections the
 * saved order does not name, such as one added by a later version, follow in
 * their default order.
 */
export function orderedSections(order: readonly string[]): Section[] {
  const known = SECTIONS.filter((id) => order.includes(id)).sort((a, b) => order.indexOf(a) - order.indexOf(b));
  return [...known, ...SECTIONS.filter((id) => !known.includes(id))];
}

const COLLAPSE_DELAY_MS = 450;
const DROP_LEAVE_DELAY_MS = 150;
const SUCCESS_MS = 1300;

interface NotchStore {
  ready: boolean;
  screen: ScreenInfo;
  settings: Settings;
  dragIcon: string | null;
  mode: Mode;
  section: Section;
  /** Pinned panels stay open until an outside click or the pin is released. */
  pinned: boolean;
  hovering: boolean;
  /** A shelf file is being dragged out; keep the panel open until it lands. */
  draggingOut: boolean;
  /** A tab is being dragged along the bar; keep the panel open until it drops. */
  reordering: boolean;
  successText: string;

  init(boot: Bootstrap): void;
  hoverChanged(inside: boolean): void;
  outsideClicked(): void;
  expand(section?: Section, pin?: boolean): void;
  collapse(): void;
  togglePin(): void;
  selectSection(section: Section): void;
  dragEntered(): void;
  dragLeft(): void;
  dropped(message: string): void;
  setDraggingOut(dragging: boolean): void;
  setReordering(reordering: boolean): void;
  /** Takes the tabs on show in their new order and remembers it. */
  reorderTabs(visible: Section[]): void;
}

let collapseTimer: number | undefined;
let dropLeaveTimer: number | undefined;
let successTimer: number | undefined;
let modeBeforeDrop: Mode = "compact";

/** Gives the keyboard back, from wherever the panel is being closed. */
function stopTyping() {
  if (!useTodos.getState().typing) return;
  if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
  useTodos.getState().setTyping(false);
}

function clearTimers() {
  window.clearTimeout(collapseTimer);
  window.clearTimeout(dropLeaveTimer);
  window.clearTimeout(successTimer);
}

export const useNotch = create<NotchStore>((set, get) => {
  const scheduleCollapse = () => {
    window.clearTimeout(collapseTimer);
    collapseTimer = window.setTimeout(() => {
      const { mode, hovering, pinned, draggingOut, reordering } = get();
      // A half-typed to-do is not something to close out from under the user;
      // ask again in a moment rather than dropping the timer altogether.
      if (useTodos.getState().typing) {
        scheduleCollapse();
        return;
      }
      if (mode === "expanded" && !hovering && !pinned && !draggingOut && !reordering) get().collapse();
    }, COLLAPSE_DELAY_MS);
  };

  /** Opens on whatever is happening now, falling back to the last section. */
  const relevantSection = (): Section => {
    // A session waiting on an answer is the most urgent thing on screen.
    if (waitingSessions(useDev.getState().sessions).length) return "dev";
    // A row that just arrived is news; one parked there is not, and should
    // not keep the panel off whatever else is going on.
    if (hasFreshActivity(useActivities.getState().items, Date.now())) return "board";
    const { media, lastActiveAt } = useMedia.getState();
    if (media?.playing) return "music";
    const now = typeof performance !== "undefined" ? performance.now() : Date.now();
    if (isMediaLive(media, lastActiveAt, now)) return "music";
    return get().section;
  };

  return {
    ready: false,
    screen: { platform: "", hasNotch: false, notchWidth: 0, notchHeight: 0, menuBarHeight: 0, displayName: "", fullscreen: false },
    settings: {
      visible: true,
      expandOnHover: true,
      idleHandle: false,
      notifyClaudeIdle: true,
      lyricsEnabled: true,
      lyricsTranslationEnabled: true,
      clipboardHistory: true,
      display: null,
      language: null,
      tabOrder: [],
    },
    dragIcon: null,
    mode: "compact",
    section: "music",
    pinned: false,
    hovering: false,
    draggingOut: false,
    reordering: false,
    successText: "",

    init(boot) {
      set({ ready: true, screen: boot.screen, settings: boot.settings, dragIcon: boot.dragIcon });
    },

    hoverChanged(inside) {
      set({ hovering: inside });
      const { mode, settings, pinned } = get();
      if (mode === "drop" || mode === "success") return;
      if (inside) {
        window.clearTimeout(collapseTimer);
        if (mode === "compact" && settings.expandOnHover) get().expand(relevantSection());
      } else if (mode === "expanded" && !pinned) {
        scheduleCollapse();
      }
    },

    outsideClicked() {
      // A drag-out that never reported back must not keep the panel open.
      set({ draggingOut: false });
      // Clicking into another app is done typing, whatever the field thinks.
      stopTyping();
      // Pinning is for keeping the panel open while you work elsewhere, so a
      // click in another window must not close it; the pin does that.
      if (get().mode === "expanded" && !get().pinned) get().collapse();
    },

    expand(section, pin = false) {
      window.clearTimeout(collapseTimer);
      const current = get();
      set({
        mode: "expanded",
        section: section ?? (current.mode === "compact" ? relevantSection() : current.section),
        pinned: pin || current.pinned,
      });
    },

    collapse() {
      clearTimers();
      stopTyping();
      set({ mode: "compact", pinned: false });
    },

    togglePin() {
      const pinned = !get().pinned;
      set({ pinned });
      if (!pinned && !get().hovering) scheduleCollapse();
    },

    selectSection(section) {
      set({ section });
    },

    dragEntered() {
      const { mode, draggingOut } = get();
      if (draggingOut) return;
      clearTimers();
      if (mode === "drop") return;
      if (mode !== "success") modeBeforeDrop = mode;
      set({ mode: "drop" });
    },

    dragLeft() {
      window.clearTimeout(dropLeaveTimer);
      dropLeaveTimer = window.setTimeout(() => {
        if (get().mode !== "drop") return;
        const restore = modeBeforeDrop === "expanded" && (get().pinned || get().hovering);
        set({ mode: restore ? "expanded" : "compact" });
      }, DROP_LEAVE_DELAY_MS);
    },

    dropped(message) {
      clearTimers();
      set({ mode: "success", successText: message, section: "shelf" });
      successTimer = window.setTimeout(() => {
        if (get().mode !== "success") return;
        if (get().hovering) {
          set({ mode: "expanded" });
        } else {
          get().collapse();
        }
      }, SUCCESS_MS);
    },

    setDraggingOut(dragging) {
      set({ draggingOut: dragging });
      if (!dragging && get().mode === "expanded" && !get().hovering && !get().pinned) scheduleCollapse();
    },

    setReordering(reordering) {
      set({ reordering });
      if (!reordering && get().mode === "expanded" && !get().hovering && !get().pinned) scheduleCollapse();
    },

    reorderTabs(visible) {
      const { settings } = get();
      // Tabs hidden right now, like an empty board, keep their places; the
      // ones on show fill the slots they held, in their new order.
      const rest = [...visible];
      const tabOrder = orderedSections(settings.tabOrder).map((id) => (visible.includes(id) ? rest.shift()! : id));
      // Shown at once; the native side writes it down and echoes it back.
      set({ settings: { ...settings, tabOrder } });
      native.setTabOrder(tabOrder).catch(() => {});
    },
  };
});
