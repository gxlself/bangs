import { create } from "zustand";
import { persist } from "zustand/middleware";

import { native, type FileMeta } from "../lib/native";

export const SHELF_CAPACITY = 12;

export interface ShelfItem extends FileMeta {
  addedAt: number;
}

export interface AddResult {
  added: number;
  duplicates: number;
  rejectedForSpace: number;
}

interface ShelfStore {
  items: ShelfItem[];
  add(paths: string[]): Promise<AddResult>;
  remove(path: string): void;
  clear(): void;
  /** Drops files that were moved or deleted since they were shelved. */
  refresh(): Promise<void>;
}

export const useShelf = create<ShelfStore>()(
  persist(
    (set, get) => ({
      items: [],

      async add(paths) {
        const known = new Set(get().items.map((item) => item.path));
        const fresh = [...new Set(paths)].filter((path) => !known.has(path));
        const metas = fresh.length ? await native.inspectFiles(fresh) : [];
        const room = Math.max(0, SHELF_CAPACITY - get().items.length);
        const accepted = metas.slice(0, room).map((meta) => ({ ...meta, addedAt: Date.now() }));
        if (accepted.length) set({ items: [...get().items, ...accepted] });
        return {
          added: accepted.length,
          duplicates: paths.length - fresh.length,
          rejectedForSpace: metas.length - accepted.length,
        };
      },

      remove(path) {
        set({ items: get().items.filter((item) => item.path !== path) });
      },

      clear() {
        set({ items: [] });
      },

      async refresh() {
        const items = get().items;
        if (!items.length) return;
        const metas = new Map(
          (await native.inspectFiles(items.map((item) => item.path))).map((meta) => [meta.path, meta]),
        );
        set({
          items: get().items.flatMap((item) => {
            const meta = metas.get(item.path);
            return meta ? [{ ...meta, addedAt: item.addedAt }] : [];
          }),
        });
      },
    }),
    { name: "bangs.shelf", partialize: ({ items }) => ({ items }) },
  ),
);

/** Sync mirrors the shelf to the iPhone app; it only knows what it is told. */
function syncShelf() {
  native.shelfSync(useShelf.getState().items).catch((error) => console.warn("shelf sync failed", error));
}

useShelf.subscribe((state, previous) => {
  if (state.items !== previous.items) syncShelf();
});

/** Once at start-up, after `refresh` has dropped the files that went away. */
export const syncShelfNow = syncShelf;
