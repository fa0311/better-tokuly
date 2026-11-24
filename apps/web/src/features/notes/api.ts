import {
  type InfiniteData,
  type QueryClient,
  useInfiniteQuery,
  useQuery,
} from "@tanstack/react-query";
import { create, notes } from "@tokuly/api";
import { create as createStore } from "zustand";
import { api } from "#/lib/api";

export const noteClient = () => api().service(notes.NoteService);
export const draftClient = () => api().service(notes.NoteDraftService);
export const muteClient = () => api().service(notes.NoteMuteService);

/** Which list of notes a timeline shows. */
export type TimelineSource =
  | { kind: "home" }
  | { kind: "following" }
  | { kind: "global" }
  | { kind: "mentions" }
  | { kind: "user"; userId: string; filter: notes.UserTimelineFilter }
  /** `tag` is normalized (lowercase, no `#`). */
  | { kind: "hashtag"; tag: string };

/** One page of any timeline, in one shape. */
export interface TimelinePage {
  entries: notes.TimelineEntry[];
  nextCursor: string;
  /** Only on the first page of a profile's notes tab, latest pinned first. */
  pinnedNotes?: notes.Note[];
  /** Only for hashtag timelines. */
  hashtag?: notes.Hashtag;
}

export type TimelineData = InfiniteData<TimelinePage, string>;

export const PAGE_SIZE = 30;

function sourceKey(source: TimelineSource): readonly unknown[] {
  if (source.kind === "user") return ["user", source.userId, source.filter];
  if (source.kind === "hashtag") return ["hashtag", source.tag];
  return [source.kind];
}

/** Query keys of the notes domain. Everything that holds notes lives under `["notes"]`, which is where live updates look. */
export const notesKeys = {
  all: ["notes"] as const,
  rules: ["notes", "rules"] as const,
  timelines: ["notes", "timeline"] as const,
  timeline: (source: TimelineSource) => ["notes", "timeline", ...sourceKey(source)] as const,
  userTimelines: (userId: string) => ["notes", "timeline", "user", userId] as const,
  thread: (noteId: string) => ["notes", "thread", noteId] as const,
  threads: ["notes", "thread"] as const,
  quotes: (noteId: string) => ["notes", "quotes", noteId] as const,
  note: (noteId: string) => ["notes", "note", noteId] as const,
  likers: (noteId: string) => ["notes", "likers", noteId] as const,
  renoters: (noteId: string) => ["notes", "renoters", noteId] as const,
  trending: ["notes", "trending"] as const,
  trendingNotes: (language: string) => ["notes", "trending-notes", language] as const,
  drafts: ["notes", "drafts"] as const,
  mutes: ["notes", "mutes"] as const,
  edits: (noteId: string) => ["notes", "edits", noteId] as const,
  analytics: (noteId: string) => ["notes", "analytics", noteId] as const,
  hashtagSearch: (prefix: string) => ["notes", "hashtag-search", prefix] as const,
};

/** Reads one page of a timeline. */
export async function fetchTimeline(
  source: TimelineSource,
  cursor: string,
  limit = PAGE_SIZE,
): Promise<TimelinePage> {
  const page = { cursor, limit };
  const client = noteClient();
  switch (source.kind) {
    case "home": {
      const response = await client.getHomeTimeline({ page });
      return { entries: response.entries, nextCursor: response.page?.nextCursor ?? "" };
    }
    case "following": {
      const response = await client.getFollowingTimeline({ page });
      return { entries: response.entries, nextCursor: response.page?.nextCursor ?? "" };
    }
    case "global": {
      const response = await client.getGlobalTimeline({ page });
      return { entries: response.entries, nextCursor: response.page?.nextCursor ?? "" };
    }
    case "mentions": {
      const response = await client.getMentionsTimeline({ page });
      return { entries: response.entries, nextCursor: response.page?.nextCursor ?? "" };
    }
    case "user": {
      const response = await client.getUserTimeline({
        userId: source.userId,
        filter: source.filter,
        page,
      });
      return {
        entries: response.entries,
        nextCursor: response.page?.nextCursor ?? "",
        pinnedNotes: response.pinnedNotes,
      };
    }
    case "hashtag": {
      const response = await client.getHashtagTimeline({ tag: source.tag, page });
      return {
        entries: response.entries,
        nextCursor: response.page?.nextCursor ?? "",
        hashtag: response.hashtag,
      };
    }
  }
}

/**
 * A timeline, newest first, paged. It never goes stale by itself: a refetch
 * would read every loaded page again. New notes are put on top instead
 * (`withFreshEntries`), which is what the timeline component does when it
 * returns to the screen, is refreshed, or is told that something was posted.
 */
export function useTimeline(source: TimelineSource, enabled = true) {
  return useInfiniteQuery({
    queryKey: notesKeys.timeline(source),
    queryFn: ({ pageParam }) => fetchTimeline(source, pageParam),
    initialPageParam: "",
    getNextPageParam: (last) => last.nextCursor || undefined,
    enabled,
    staleTime: Number.POSITIVE_INFINITY,
  });
}

/** A note in its conversation: the notes it answers, itself, and its replies oldest first (paged). */
export function useThread(noteId: string) {
  return useInfiniteQuery({
    queryKey: notesKeys.thread(noteId),
    queryFn: ({ pageParam }) =>
      noteClient().getNoteThread({ noteId, page: { cursor: pageParam, limit: PAGE_SIZE } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
}

export type ThreadData = InfiniteData<notes.GetNoteThreadResponse, string>;

/** Quote notes of a note, newest first. */
export function useQuotes(noteId: string, enabled = true) {
  return useInfiniteQuery({
    queryKey: notesKeys.quotes(noteId),
    queryFn: ({ pageParam }) =>
      noteClient().listNoteQuotes({ noteId, page: { cursor: pageParam, limit: PAGE_SIZE } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
    enabled,
  });
}

// ───────────────────────────── Single notes, batched ─────────────────────────────

/** `BatchGetNotes` takes at most this many IDs. */
const BATCH_LIMIT = 100;

interface Waiter {
  noteId: string;
  resolve(note: notes.Note | null): void;
  reject(error: unknown): void;
}

let waiting: Waiter[] = [];
let scheduled = false;

async function flush(): Promise<void> {
  scheduled = false;
  const waiters = waiting;
  waiting = [];
  const ids = [...new Set(waiters.map((waiter) => waiter.noteId))];
  for (let start = 0; start < ids.length; start += BATCH_LIMIT) {
    const chunk = new Set(ids.slice(start, start + BATCH_LIMIT));
    const mine = waiters.filter((waiter) => chunk.has(waiter.noteId));
    try {
      const response = await noteClient().batchGetNotes({ noteIds: [...chunk] });
      const found = new Map(response.notes.map((note) => [note.id, note]));
      for (const waiter of mine) waiter.resolve(found.get(waiter.noteId) ?? null);
    } catch (error) {
      for (const waiter of mine) waiter.reject(error);
    }
  }
}

/** One note by ID; `null` when it is gone or hidden. Requests of the same moment share one `BatchGetNotes` call. */
export function loadNote(noteId: string): Promise<notes.Note | null> {
  return new Promise((resolve, reject) => {
    waiting.push({ noteId, resolve, reject });
    if (scheduled) return;
    scheduled = true;
    setTimeout(() => void flush(), 10);
  });
}

/** A note shown outside a timeline (a saved note, a feed card). Kept current by the same live updates as timelines. */
export function useNote(noteId: string, enabled = true) {
  return useQuery({
    queryKey: notesKeys.note(noteId),
    queryFn: () => loadNote(noteId),
    enabled: enabled && Boolean(noteId),
    staleTime: 60_000,
  });
}

// ───────────────────────────── Cache patching ─────────────────────────────

const NOTE_TYPE = "tokuly.notes.v1.Note";

/**
 * Rebuilds `value` with every note in it passed through `update` (quoted
 * notes included). Whatever did not change keeps its identity, so only the
 * cards that changed render again.
 */
export function mapNotes<T>(value: T, update: (note: notes.Note) => notes.Note): T {
  if (Array.isArray(value)) {
    let changed = false;
    const next = value.map((item) => {
      const mapped = mapNotes(item, update);
      if (mapped !== item) changed = true;
      return mapped;
    });
    return (changed ? next : value) as T;
  }
  if (value === null || typeof value !== "object" || value instanceof Uint8Array) return value;
  let next: Record<string, unknown> | null = null;
  for (const [key, item] of Object.entries(value)) {
    const mapped = mapNotes(item, update);
    if (mapped !== item) {
      next ??= { ...(value as Record<string, unknown>) };
      next[key] = mapped;
    }
  }
  const result = (next ?? value) as T;
  return (result as { $typeName?: string }).$typeName === NOTE_TYPE
    ? (update(result as notes.Note) as T)
    : result;
}

/** Applies `update` to a note wherever it is cached: timelines, threads, quote lists, single notes, quote embeds. */
export function patchNote(
  queryClient: QueryClient,
  noteId: string,
  update: (note: notes.Note) => notes.Note,
): void {
  const apply = (note: notes.Note) => (note.id === noteId ? update(note) : note);
  for (const query of queryClient.getQueryCache().findAll({ queryKey: notesKeys.all })) {
    const data = query.state.data;
    if (data === undefined) continue;
    const next = mapNotes(data, apply);
    // Keep the entry's age: a counter that moved does not make a page of notes fresh.
    if (next !== data)
      queryClient.setQueryData(query.queryKey, next, { updatedAt: query.state.dataUpdatedAt });
  }
}

/** A note as it is cached right now, wherever it is cached; `undefined` when no list on screen holds it. */
export function findCachedNote(queryClient: QueryClient, noteId: string): notes.Note | undefined {
  let found: notes.Note | undefined;
  for (const query of queryClient.getQueryCache().findAll({ queryKey: notesKeys.all })) {
    mapNotes(query.state.data, (note) => {
      if (note.id === noteId) found ??= note;
      return note;
    });
    if (found) return found;
  }
  return undefined;
}

export function withCounters(note: notes.Note, counters: notes.NoteCounters): notes.Note {
  return { ...note, counters };
}

/** The note's counters with one of them moved by `delta`: the guess shown before the server answers. */
export function bumpCounter(
  note: notes.Note,
  counter: "likeCount" | "renoteCount" | "replyCount" | "quoteCount",
  delta: number,
): notes.NoteCounters {
  const current = note.counters ?? create(notes.NoteCountersSchema, { noteId: note.id });
  const next = current[counter] + BigInt(delta);
  return { ...current, [counter]: next < 0n ? 0n : next };
}

export function withViewer(
  note: notes.Note,
  viewer: Partial<Pick<notes.NoteViewerState, "liked" | "renoted">>,
): notes.Note {
  return {
    ...note,
    viewer: { ...(note.viewer ?? create(notes.NoteViewerStateSchema)), ...viewer },
  };
}

// ───────────────────────────── Deleted notes ─────────────────────────────

/**
 * Notes known to be gone (deleted here, or announced as deleted over
 * realtime). Lists hide them and quote embeds show them as unavailable
 * without anyone rewriting cached pages.
 */
export const useDeletedNotes = createStore<{ ids: ReadonlySet<string> }>(() => ({
  ids: new Set(),
}));

export function markNoteDeleted(noteId: string): void {
  const { ids } = useDeletedNotes.getState();
  if (!ids.has(noteId)) useDeletedNotes.setState({ ids: new Set(ids).add(noteId) });
}

export function useIsDeleted(noteId: string | undefined): boolean {
  return useDeletedNotes((state) => (noteId ? state.ids.has(noteId) : false));
}

/** Nothing known to be deleted anymore: the next account starts clean. */
export function resetDeletedNotes(): void {
  useDeletedNotes.setState({ ids: new Set() });
}

// ───────────────────────────── People who left ─────────────────────────────

/**
 * A friendship ended or a block went up: that person's notes (and the rows
 * their renotes put there) leave every timeline already loaded. Timelines
 * never refetch by themselves, so nothing else would take them out. Their
 * own timeline, threads and single notes are read again when next shown.
 */
export function forgetUserNotes(queryClient: QueryClient, userId: string): void {
  if (!userId) return;
  const theirs = (entry: notes.TimelineEntry) =>
    entry.note?.author?.id === userId ||
    entry.renote?.user?.id === userId ||
    entry.note?.quote?.note?.author?.id === userId;
  queryClient.removeQueries({ queryKey: notesKeys.userTimelines(userId) });
  for (const query of queryClient.getQueryCache().findAll({ queryKey: notesKeys.timelines })) {
    const data = query.state.data as TimelineData | undefined;
    if (
      !data?.pages.some(
        (page) =>
          page.entries.some(theirs) || page.pinnedNotes?.some((note) => note.author?.id === userId),
      )
    )
      continue;
    queryClient.setQueryData<TimelineData>(
      query.queryKey,
      {
        ...data,
        pages: data.pages.map((page) => ({
          ...page,
          entries: page.entries.filter((entry) => !theirs(entry)),
          pinnedNotes: page.pinnedNotes?.filter((note) => note.author?.id !== userId),
        })),
      },
      { updatedAt: query.state.dataUpdatedAt },
    );
  }
  void queryClient.invalidateQueries({ queryKey: notesKeys.threads });
  void queryClient.invalidateQueries({ queryKey: ["notes", "note"] });
  void queryClient.invalidateQueries({ queryKey: ["notes", "quotes"] });
  // The mixed home feed and the friends' log are read again as well.
  void queryClient.invalidateQueries({ queryKey: ["feed"] });
}

// ───────────────────────────── Timeline entries ─────────────────────────────

/** Identity of a timeline row: the note, plus the renote that put it there. */
export function entryKey(entry: notes.TimelineEntry): string {
  const renotedAt = entry.renote?.renotedAt;
  return `${entry.note?.id ?? ""}:${entry.renote?.user?.id ?? ""}:${renotedAt ? `${renotedAt.seconds}.${renotedAt.nanos}` : ""}`;
}

/**
 * How many rows of `fresh` (the newest page, just read) come before the
 * first row already on screen. `undefined` when that row is not in `fresh`
 * at all: more arrived than one page holds.
 */
export function countNewEntries(
  current: TimelineData | undefined,
  fresh: TimelinePage,
): number | undefined {
  const top = current?.pages[0]?.entries[0];
  if (!top) return fresh.entries.length;
  const index = fresh.entries.findIndex((entry) => entryKey(entry) === entryKey(top));
  return index === -1 ? undefined : index;
}

/**
 * The timeline with the rows of `fresh` that are new put on top. A note
 * shows once per timeline, so a row that moved up (it was renoted again)
 * leaves its old place. When `fresh` does not reach the rows on screen, the
 * list starts over from it rather than hiding a gap.
 */
export function withFreshEntries(
  current: TimelineData | undefined,
  fresh: TimelinePage,
): TimelineData {
  const count = countNewEntries(current, fresh);
  if (!current || current.pages.length === 0 || count === undefined)
    return { pages: [fresh], pageParams: [""] };
  if (count === 0) {
    // Nothing new on top; the first page still learns what only it carries (the pinned note).
    return {
      ...current,
      pages: current.pages.map((page, index) =>
        index === 0
          ? { ...page, pinnedNotes: fresh.pinnedNotes, hashtag: fresh.hashtag ?? page.hashtag }
          : page,
      ),
    };
  }
  const added = fresh.entries.slice(0, count);
  const addedIds = new Set(added.map((entry) => entry.note?.id));
  return {
    ...current,
    pages: current.pages.map((page, index) => {
      const kept = page.entries.filter((entry) => !addedIds.has(entry.note?.id));
      return index === 0
        ? {
            ...page,
            pinnedNotes: fresh.pinnedNotes,
            hashtag: fresh.hashtag ?? page.hashtag,
            entries: [...added, ...kept],
          }
        : { ...page, entries: kept };
    }),
  };
}

/** Puts a note the viewer just posted on top of a cached timeline (if that timeline is cached at all). */
export function prependNote(
  queryClient: QueryClient,
  source: TimelineSource,
  note: notes.Note,
): void {
  queryClient.setQueryData<TimelineData>(notesKeys.timeline(source), (current) => {
    if (!current || current.pages.length === 0) return current;
    const entry = create(notes.TimelineEntrySchema, { note });
    return {
      ...current,
      pages: current.pages.map((page, index) =>
        index === 0
          ? {
              ...page,
              entries: [entry, ...page.entries.filter((item) => item.note?.id !== note.id)],
            }
          : page,
      ),
    };
  });
}

// ───────────────────────────── Hashtags ─────────────────────────────

/** Hashtags many different people are using right now, best first. */
export function useTrendingHashtags(limit = 10) {
  return useQuery({
    queryKey: [...notesKeys.trending, limit] as const,
    queryFn: () => noteClient().listTrendingHashtags({ limit }),
    // The server recomputes the ranking every few minutes.
    staleTime: 3 * 60_000,
  });
}

/** Hashtags starting with `prefix`, most used first: completion while writing. */
export function useHashtagSearch(prefix: string, enabled: boolean) {
  return useQuery({
    queryKey: notesKeys.hashtagSearch(prefix),
    queryFn: async () => (await noteClient().searchHashtags({ prefix, limit: 8 })).hashtags,
    enabled,
    staleTime: 60_000,
  });
}
