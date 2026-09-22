import { useEffect, useRef, useState, type ComponentType, type CSSProperties, type PointerEvent, type SVGProps } from "react";
import { flushSync } from "react-dom";

import { refreshHover } from "../lib/hover";
import { events } from "../lib/native";
import { useNotch, type Section } from "../store/notch";

export interface Tab {
  id: Section;
  label: string;
  Icon: ComponentType<SVGProps<SVGSVGElement>>;
}

interface Props {
  tabs: Tab[];
  active: Section;
  onSelect(id: Section): void;
  onReorder(ids: Section[]): void;
}

/** How far a press travels before it stops being a click and starts a drag. */
const DRAG_THRESHOLD_PX = 4;
/** How long a dropped tab glides into its slot; matches styles.css. */
const SETTLE_MS = 180;
/**
 * A drag ends on a tab, which the browser counts as a click on it. That click
 * follows the release at once, or a moment after when the native side saw
 * the release first; anything later is a click of its own.
 */
const DROP_CLICK_MS = 250;

/** What the bar draws while a tab is picked up. */
interface Drag {
  id: Section;
  /** How far the picked-up tab is drawn from its own slot. */
  dx: number;
  from: number;
  /** The slot it lands in if dropped now. */
  to: number;
  /** How far the tabs it passes step aside: its width plus the gap. */
  step: number;
  /** Released and gliding into its slot. */
  settling: boolean;
}

interface Press {
  originX: number;
  originY: number;
  ids: Section[];
  /** The tabs' boxes when the press began; the drag moves nothing but transforms. */
  rects: DOMRect[];
  from: number;
  started: boolean;
  drag: Drag | null;
  detach(): void;
}

/**
 * The panel's tabs. A click picks one; a press that moves drags it along the
 * bar, the others step aside, and the order it is dropped in is kept.
 */
export function TabBar({ tabs, active, onSelect, onReorder }: Props) {
  const [drag, setDrag] = useState<Drag | null>(null);
  const nav = useRef<HTMLElement>(null);
  const press = useRef<Press | null>(null);
  const settleTimer = useRef<number | undefined>(undefined);
  const droppedAt = useRef(-Infinity);

  /**
   * Ends the press. Kept, the tab lands in the slot it is drawn over, and the
   * order and the offsets change in one frame so nothing jumps; otherwise
   * every tab goes back to where it was.
   */
  const finish = (keep: boolean) => {
    window.clearTimeout(settleTimer.current);
    const current = press.current;
    press.current = null;
    if (!current) return;
    current.detach();
    const shown = keep ? current.drag : null;
    if (shown && shown.to !== shown.from) {
      const ids = [...current.ids];
      ids.splice(shown.to, 0, ...ids.splice(shown.from, 1));
      flushSync(() => {
        setDrag(null);
        onReorder(ids);
      });
    } else {
      setDrag(null);
    }
    if (current.started) useNotch.getState().setReordering(false);
  };

  /** Lets go of the tab and lets it glide into the slot it is over. */
  const release = () => {
    const current = press.current;
    if (!current || current.drag?.settling) return;
    current.detach();
    if (!current.drag) {
      press.current = null;
      return;
    }
    droppedAt.current = performance.now();
    const { rects } = current;
    const { from, to } = current.drag;
    const own = rects[from];
    const slot = to > from ? rects[to].right - own.width : to < from ? rects[to].left : own.left;
    current.drag = { ...current.drag, dx: slot - own.left, settling: true };
    setDrag(current.drag);
    settleTimer.current = window.setTimeout(() => finish(true), SETTLE_MS);
  };

  const begin = (event: PointerEvent<HTMLButtonElement>, id: Section) => {
    if (event.button !== 0) return;
    // A tab still gliding into place lands before the next one is picked up.
    finish(true);
    const buttons = Array.from(nav.current?.querySelectorAll<HTMLElement>(".tab") ?? []);
    const ids = buttons.map((button) => button.dataset.id as Section);
    const from = ids.indexOf(id);
    if (from < 0 || ids.length < 2) return;

    const move = (moveEvent: globalThis.PointerEvent) => {
      const current = press.current;
      if (!current) return;
      const travel = moveEvent.clientX - current.originX;
      if (!current.started) {
        if (Math.hypot(travel, moveEvent.clientY - current.originY) < DRAG_THRESHOLD_PX) return;
        current.started = true;
        useNotch.getState().setReordering(true);
      }
      const { rects } = current;
      const own = rects[from];
      // The tab slides along the bar and stops at its ends.
      const dx = Math.min(Math.max(travel, rects[0].left - own.left), rects[rects.length - 1].right - own.right);
      // A neighbour gives way once the dragged tab's leading edge passes its middle.
      let to = from;
      rects.forEach((rect, index) => {
        const middle = rect.left + rect.width / 2;
        if (index > from && own.right + dx > middle) to = Math.max(to, index);
        if (index < from && own.left + dx < middle) to = Math.min(to, index);
      });
      const gap = rects.length > 1 ? rects[1].left - rects[0].right : 0;
      current.drag = { id, dx, from, to, step: own.width + gap, settling: false };
      setDrag(current.drag);
    };
    const detach = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", release);
      window.removeEventListener("pointercancel", release);
    };

    press.current = {
      originX: event.clientX,
      originY: event.clientY,
      ids,
      rects: buttons.map((button) => button.getBoundingClientRect()),
      from,
      started: false,
      drag: null,
      detach,
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", release);
    window.addEventListener("pointercancel", release);
  };

  // WKWebView now and then loses a release, which would leave the tab stuck
  // to the pointer until the next click; the native side watches the button
  // itself.
  const releaseRef = useRef(release);
  releaseRef.current = release;
  useEffect(() => {
    const unlisten = events.release(() => releaseRef.current());
    return () => void unlisten.then((stop) => stop());
  }, []);

  // A tab that appears or goes mid-drag moves the slots it was measured
  // against; call the drag off rather than land it somewhere unintended.
  const key = tabs.map((tab) => tab.id).join();
  useEffect(() => {
    if (press.current && press.current.ids.join() !== key) finish(false);
  }, [key]);

  // The cursor is read from the stylesheet only when the element under it
  // changes, and a picked-up tab stays under it.
  const holding = drag !== null && !drag.settling;
  useEffect(() => {
    refreshHover();
  }, [holding]);

  useEffect(
    () => () => {
      window.clearTimeout(settleTimer.current);
      press.current?.detach();
      if (press.current?.started) useNotch.getState().setReordering(false);
      press.current = null;
    },
    [],
  );

  const offset = (id: Section, index: number): CSSProperties | undefined => {
    if (!drag) return undefined;
    if (id === drag.id) {
      return { transform: `translateX(${drag.dx}px)${drag.settling ? "" : " scale(1.06)"}` };
    }
    if (index > drag.from && index <= drag.to) return { transform: `translateX(${-drag.step}px)` };
    if (index < drag.from && index >= drag.to) return { transform: `translateX(${drag.step}px)` };
    return undefined;
  };

  return (
    <nav
      ref={nav}
      className={`bar__side tabs${drag ? " is-reordering" : ""}${drag?.settling ? " is-settling" : ""}`}
    >
      {tabs.map(({ id, label, Icon }, index) => (
        <button
          key={id}
          data-id={id}
          className={`tab${active === id ? " is-active" : ""}${drag?.id === id ? " is-dragging" : ""}`}
          style={offset(id, index)}
          onPointerDown={(event) => begin(event, id)}
          onClick={() => {
            if (performance.now() - droppedAt.current > DROP_CLICK_MS) onSelect(id);
          }}
          title={label}
        >
          <Icon width={13} height={13} />
          {active === id && <span>{label}</span>}
        </button>
      ))}
    </nav>
  );
}
