import { type RefObject, useEffect } from "react";
import { noteClient } from "./api";

/** A note counts as seen once this much of it stayed on screen… */
const VISIBLE_RATIO = 0.5;
/** …for this long. */
const DWELL_MS = 1000;
/** Views are reported together, this long after the first one of a batch. */
const FLUSH_MS = 2500;
/** `RecordNoteViews` takes at most this many notes. */
const BATCH_LIMIT = 100;

/** Notes this page already reported: the server counts a viewer once per note for hours anyway. */
const reported = new Set<string>();
let queue: string[] = [];
let flushTimer: ReturnType<typeof setTimeout> | undefined;

function flush(): void {
  flushTimer = undefined;
  const noteIds = queue.slice(0, BATCH_LIMIT);
  queue = queue.slice(BATCH_LIMIT);
  if (noteIds.length > 0)
    void noteClient()
      .recordNoteViews({ noteIds })
      .catch(() => {});
  if (queue.length > 0) flushTimer = setTimeout(flush, FLUSH_MS);
}

function report(noteId: string): void {
  if (reported.has(noteId)) return;
  reported.add(noteId);
  queue.push(noteId);
  if (flushTimer === undefined) flushTimer = setTimeout(flush, FLUSH_MS);
}

/** Forgets what was reported: views belong to the account that was signed in. */
export function resetViews(): void {
  reported.clear();
  queue = [];
}

interface Watched {
  noteId: string;
  timer: ReturnType<typeof setTimeout> | undefined;
}

const watched = new Map<Element, Watched>();
let observer: IntersectionObserver | null = null;

function watch(element: Element, noteId: string): () => void {
  observer ??= new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        const state = watched.get(entry.target);
        if (!state) continue;
        const visible = entry.isIntersecting && entry.intersectionRatio >= VISIBLE_RATIO;
        if (visible && !state.timer) {
          state.timer = setTimeout(() => report(state.noteId), DWELL_MS);
        } else if (!visible && state.timer) {
          clearTimeout(state.timer);
          state.timer = undefined;
        }
      }
    },
    { threshold: [0, VISIBLE_RATIO, 1] },
  );
  watched.set(element, { noteId, timer: undefined });
  observer.observe(element);
  return () => {
    clearTimeout(watched.get(element)?.timer);
    watched.delete(element);
    observer?.unobserve(element);
  };
}

/**
 * Reports a view of the note once its card was really on screen: half of it,
 * for a second. Scrolling past does not count; notes of the viewer are not
 * reported (the server would not count them).
 */
export function useRecordView(
  card: RefObject<Element | null>,
  noteId: string,
  enabled: boolean,
): void {
  useEffect(() => {
    const element = card.current;
    if (!element || !enabled || reported.has(noteId)) return;
    return watch(element, noteId);
  }, [card, noteId, enabled]);
}
