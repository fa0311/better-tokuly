import { formatClock, parseClock } from "../format";

/** A chapter as it is typed in the editor: the start is free text until it parses. */
export interface ChapterDraft {
  /** Stable key for the row while it is edited. */
  key: string;
  time: string;
  title: string;
}

/** What is wrong with a row; the editor shows `studio.chapters.errors.<code>` under it. */
export type ChapterError = "time" | "title" | "first" | "order" | "gap" | "end";

/** Chapters start at least this far apart. */
export const MIN_CHAPTER_GAP = 5;
export const MAX_CHAPTERS = 100;

let nextKey = 0;
export function chapterKey(): string {
  nextKey += 1;
  return `chapter-${nextKey}`;
}

export function toDrafts(chapters: { startSeconds: number; title: string }[]): ChapterDraft[] {
  return chapters.map((chapter) => ({
    key: chapterKey(),
    time: formatClock(chapter.startSeconds),
    title: chapter.title,
  }));
}

/**
 * Checks the rows the way the server will: every start is a time inside the
 * video, the first is 0:00, each begins at least five seconds after the one
 * before, and every chapter has a name. Returns one error (or none) per row,
 * in row order.
 */
export function validateChapters(
  drafts: ChapterDraft[],
  durationSeconds: number,
): (ChapterError | null)[] {
  let previous: number | null = null;
  return drafts.map((draft, index) => {
    const start = parseClock(draft.time);
    if (start === null) return "time";
    const before = previous;
    previous = start;
    if (index === 0 && start !== 0) return "first";
    if (before !== null && start <= before) return "order";
    if (before !== null && start - before < MIN_CHAPTER_GAP) return "gap";
    if (durationSeconds > 0 && start >= durationSeconds) return "end";
    if (!draft.title.trim() || draft.title.trim().length > 100) return "title";
    return null;
  });
}

/** The rows as the API takes them. Only call it when `validateChapters` found nothing. */
export function toChapters(drafts: ChapterDraft[]): { startSeconds: number; title: string }[] {
  return drafts.map((draft) => ({
    startSeconds: parseClock(draft.time) ?? 0,
    title: draft.title.trim(),
  }));
}

/**
 * A new row for the moment `at` of the video, put where it belongs in time.
 * The very first row always starts at 0:00.
 */
export function withChapterAt(drafts: ChapterDraft[], at: number): ChapterDraft[] {
  const start = drafts.length === 0 ? 0 : Math.max(0, Math.floor(at));
  const row: ChapterDraft = { key: chapterKey(), time: formatClock(start), title: "" };
  const index = drafts.findIndex(
    (draft) => (parseClock(draft.time) ?? Number.POSITIVE_INFINITY) > start,
  );
  return index < 0 ? [...drafts, row] : [...drafts.slice(0, index), row, ...drafts.slice(index)];
}

/** Lowercases, strips `#` and blanks, and drops what is already there: a tag as the server stores it. */
export function normalizeTag(input: string): string {
  return input.trim().replace(/^#+/, "").replace(/\s+/g, "").toLowerCase().slice(0, 30);
}
