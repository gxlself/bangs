import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type KeyboardEvent } from "react";

import { refreshHover } from "../lib/hover";
import { t } from "../lib/i18n";
import { DUST_STAGGER_MS, MAX_OPEN, shownOrder, useTodos } from "../store/todos";
import { CheckIcon, CloseIcon, PlusIcon } from "./Icons";

/** How far apart the characters of a line start coming loose. */
const STAGGER_MS = 220;
/** The longest line, in step with MAX_TEXT in src-tauri/src/todos.rs. */
const MAX_TEXT = 200;
/** "Clear done" asks once more for this long. */
const CONFIRM_MS = 3000;

/**
 * The one panel you type into. The notch never takes focus on its own, so the
 * field asks for the keyboard when it is focused and hands it straight back
 * (see `capture_keyboard` and src/store/todos.ts).
 *
 * Clicking a line ticks it off — it is struck through and sinks below what is
 * still open — and clicking it again brings it back. Deleting is its own act:
 * the × on a line, or "Clear done" for every done line at once, and a deleted
 * line comes apart and blows off the list.
 */
export function TodoPanel() {
  const items = useTodos((state) => state.items);
  const dusting = useTodos((state) => state.dusting);
  const held = useTodos((state) => state.held);
  const typing = useTodos((state) => state.typing);
  const { add, toggle, remove, clearDone, setTyping } = useTodos.getState();
  const [draft, setDraft] = useState("");
  const [confirming, setConfirming] = useState(false);
  const [more, setMore] = useState(false);
  const field = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);

  // The panel can be taken away mid-word — an agent alert switches tabs, the
  // notch collapses — and a field that never sees its own blur would leave
  // the keyboard with a window that is no longer showing it.
  useEffect(() => () => useTodos.getState().setTyping(false), []);

  // Rows move under a resting pointer when a line is ticked, deleted or
  // added; what is hovered (and so which × is live) has to follow.
  useLayoutEffect(() => {
    refreshHover();
    updateMore();
  }, [items, dusting, held]);

  // "Clear done" waits for a second click, but not forever.
  useEffect(() => {
    if (!confirming) return;
    const timer = window.setTimeout(() => setConfirming(false), CONFIRM_MS);
    return () => window.clearTimeout(timer);
  }, [confirming]);

  const shown = shownOrder(items, held);
  const alive = items.filter((item) => !dusting.includes(item.id));
  const left = alive.filter((item) => !item.done).length;
  const done = alive.length - left;
  const full = left >= MAX_OPEN;
  const dustOrder = shown.filter((item) => dusting.includes(item.id)).map((item) => item.id);

  /** Whether there are rows below the fold; the hidden scrollbar does not say. */
  function updateMore() {
    const rows = list.current;
    setMore(!!rows && rows.scrollTop + rows.clientHeight < rows.scrollHeight - 1);
  }

  const submit = () => {
    if (!draft.trim() || full) return;
    void add(draft);
    setDraft("");
    // The new line goes on top, which may be scrolled out of sight.
    list.current?.scrollTo({ top: 0, behavior: "smooth" });
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
      event.preventDefault();
      submit();
    } else if (event.key === "Escape") {
      setDraft("");
      field.current?.blur();
    }
  };

  const onClear = () => {
    if (!confirming) {
      setConfirming(true);
      return;
    }
    setConfirming(false);
    clearDone();
  };

  return (
    <div className="todo">
      <div className={`todo__new${typing ? " is-typing" : ""}`} onClick={() => field.current?.focus()}>
        <PlusIcon width={13} height={13} />
        <input
          ref={field}
          className="todo__input"
          value={draft}
          maxLength={MAX_TEXT}
          disabled={full}
          placeholder={
            full
              ? t("列表满了，先完成几件吧", "The list is full — finish a few first")
              : t("加一件事，回车记下", "Add something, press return")
          }
          spellCheck={false}
          autoComplete="off"
          onChange={(event) => setDraft(event.target.value)}
          onFocus={() => setTyping(true)}
          onBlur={() => setTyping(false)}
          onKeyDown={onKeyDown}
        />
      </div>

      {items.length ? (
        <div ref={list} className={`rows rows--snap${more ? " is-more" : ""}`} onScroll={updateMore}>
          {shown.map((item) => {
            const dust = dusting.includes(item.id);
            const rowDelay = dust ? Math.min(dustOrder.indexOf(item.id) * DUST_STAGGER_MS, 320) : 0;
            return (
              <div
                key={item.id}
                className={`row todo__row${item.done ? " is-done" : ""}${dust ? " is-dust" : ""}`}
                style={{ "--row-delay": `${rowDelay}ms` } as CSSProperties}
                title={item.text}
                onClick={() => toggle(item.id)}
              >
                <span className="todo__box">{item.done && <CheckIcon width={11} height={11} />}</span>
                <span className="row__main">
                  {dust ? (
                    <Dust text={item.text} delay={rowDelay} />
                  ) : (
                    <span className="row__title row__title--plain todo__text">{item.text}</span>
                  )}
                </span>
                <button
                  className="todo__delete"
                  title={t("删除", "Delete")}
                  aria-label={t("删除", "Delete")}
                  tabIndex={-1}
                  onClick={(event) => {
                    event.stopPropagation();
                    remove(item.id);
                  }}
                >
                  <CloseIcon width={10} height={10} />
                </button>
              </div>
            );
          })}
        </div>
      ) : (
        <div className="todo__blank">{t("这里还空着", "Nothing on the list")}</div>
      )}

      <div className="shelf__footer">
        <span>
          {left
            ? done
              ? t(`还有 ${left} 件`, `${left} to go`)
              : t(`还有 ${left} 件 · 点一下打勾`, `${left} to go · click to tick off`)
            : alive.length
              ? t("都做完了", "All done")
              : ""}
        </span>
        {done > 0 && (
          <button className={`link-button${confirming ? " is-confirming" : ""}`} tabIndex={-1} onClick={onClear}>
            {confirming ? t("再点一下清除", "Click again to clear") : t(`清除已完成（${done}）`, `Clear ${done} done`)}
          </button>
        )}
      </div>
    </div>
  );
}

/** The characters of a line as they are read: an emoji or an accented letter stays whole. */
function graphemes(text: string): string[] {
  if (typeof Intl !== "undefined" && "Segmenter" in Intl) {
    return Array.from(new Intl.Segmenter(undefined, { granularity: "grapheme" }).segment(text), (part) => part.segment);
  }
  return [...text];
}

/**
 * A line coming apart, character by character. The offsets are worked out from
 * each character's place in the line rather than drawn at random, so the same
 * line always blows away the same way and a re-render never restarts it.
 */
function Dust({ text, delay }: { text: string; delay: number }) {
  const characters = graphemes(text);
  const last = Math.max(characters.length - 1, 1);
  return (
    <span className="row__title row__title--plain" aria-label={text}>
      {characters.map((character, index) => (
        <span
          key={index}
          className="todo__dust"
          style={
            {
              "--delay": `${delay + Math.round((index / last) * STAGGER_MS)}ms`,
              "--drift": `${12 + ((index * 7) % 14)}px`,
              "--lift": `${-5 - ((index * 5) % 12)}px`,
              "--spin": `${((index * 11) % 26) - 13}deg`,
            } as CSSProperties
          }
        >
          {character === " " ? " " : character}
        </span>
      ))}
    </span>
  );
}
