import type { ComponentType, SVGProps } from "react";



import { t } from "../lib/i18n";
import type { Size } from "../lib/layout";
import { orderedSections, useNotch, type Section } from "../store/notch";
import { DevPanel } from "./DevPanel";
import { ActivityPanel } from "./ActivityPanel";
import { BoardIcon, ClipboardIcon, CodeIcon, MusicIcon, PinIcon, ShelfIcon, TodoIcon } from "./Icons";
import { MusicPanel } from "./MusicPanel";
import { ClipboardPanel } from "./ClipboardPanel";
import { ShelfPanel } from "./ShelfPanel";
import { TabBar } from "./TabBar";
import { TodoPanel } from "./TodoPanel";
import { useActivities } from "../store/activities";
import { useClipboard } from "../store/clipboard";

const TABS: Record<Section, { label: () => string; Icon: ComponentType<SVGProps<SVGSVGElement>> }> = {
  music: { label: () => t("音乐", "Music"), Icon: MusicIcon },
  shelf: { label: () => t("暂存", "Shelf"), Icon: ShelfIcon },
  dev: { label: () => t("代码", "Code"), Icon: CodeIcon },
  paste: { label: () => t("剪贴板", "Clipboard"), Icon: ClipboardIcon },
  todo: { label: () => t("待办", "To-do"), Icon: TodoIcon },
  board: { label: () => t("上岛", "Board"), Icon: BoardIcon },
};

interface Props {
  base: Size;
  /** Width of the hardware notch the header must leave empty. */
  notchGap: number;
}

export function ExpandedView({ base, notchGap }: Props) {
  const section = useNotch((s) => s.section);
  const pinned = useNotch((s) => s.pinned);
  const hasClipboard = useClipboard((state) => state.available);
  const hasActivities = useActivities((state) => state.items.length > 0);
  const isMac = useNotch((s) => s.screen.platform === "macos");
  const tabOrder = useNotch((s) => s.settings.tabOrder);
  const { selectSection, togglePin, reorderTabs } = useNotch.getState();
  // The clipboard tab stays visible on macOS without Paste, to offer it.
  // The board only appears once something has docked, so the bar stays short.
  const tabs = orderedSections(tabOrder)
    .filter((id) => (id === "paste" ? hasClipboard || isMac : id !== "board" || hasActivities || section === "board"))
    .map((id) => ({ id, label: TABS[id].label(), Icon: TABS[id].Icon }));

  return (
    <div className="expanded">
      <header className="bar" style={{ height: base.height }}>
        <TabBar tabs={tabs} active={section} onSelect={selectSection} onReorder={reorderTabs} />
        <div style={{ width: notchGap, flex: "none" }} />
        <div className="bar__side bar__side--end">
          <button
            className={`icon-button${pinned ? " is-active" : ""}`}
            onClick={togglePin}
            title={pinned ? t("取消固定", "Unpin") : t("固定展开", "Keep open")}
          >
            <PinIcon width={13} height={13} filled={pinned} />
          </button>
        </div>
      </header>

      {/* Keyed so switching tabs fades the new panel in. */}
      <main className="panel" key={section}>
        {section === "music" && <MusicPanel />}
        {section === "shelf" && <ShelfPanel />}
        {section === "dev" && <DevPanel />}
        {section === "paste" && <ClipboardPanel />}
        {section === "todo" && <TodoPanel />}
        {section === "board" && <ActivityPanel />}
      </main>
    </div>
  );
}
