import type { InfiniteData, QueryClient } from "@tanstack/query-core";
import { create, notes } from "@tokuly/api";
import { notesKeys, type TimelineSource } from "./keys";
import type { TimelineData } from "./timeline";

export type ThreadData = InfiniteData<notes.GetNoteThreadResponse, string>;

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

export type NoteCounter = "likeCount" | "renoteCount" | "replyCount" | "quoteCount";

/** The note's counters with one of them moved by `delta`: the guess shown before the server answers. */
export function bumpCounter(
  note: notes.Note,
  counter: NoteCounter,
  delta: number,
): notes.NoteCounters {
  const current = note.counters ?? create(notes.NoteCountersSchema, { noteId: note.id });
  const next = current[counter] + BigInt(delta);
  return { ...current, [counter]: next < 0n ? 0n : next };
}

export function withViewer(
  note: notes.Note,
  viewer: Partial<Pick<notes.NoteViewerState, "liked" | "renoted" | "threadMuted">>,
): notes.Note {
  return {
    ...note,
    viewer: { ...(note.viewer ?? create(notes.NoteViewerStateSchema)), ...viewer },
  };
}

/**
 * A like or a renote shown at once: the viewer's mark and the count move
 * together (`on` = the state after the tap). Applying it to a note already
 * in that state changes nothing, so a tap and its undo never count twice.
 */
export function withToggle(note: notes.Note, kind: "like" | "renote", on: boolean): notes.Note {
  const field = kind === "like" ? "liked" : "renoted";
  if ((note.viewer?.[field] ?? false) === on) return note;
  const counter = kind === "like" ? "likeCount" : "renoteCount";
  return withViewer(
    { ...note, counters: bumpCounter(note, counter, on ? 1 : -1) },
    {
      [field]: on,
    },
  );
}

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

type ThreadCounter = "replyCount" | "quoteCount";

/** The reply and quote counts of the notes a new note answers or quotes, as cached when it is sent. */
export interface CountsBefore {
  replies?: bigint;
  quotes?: bigint;
}

const counterNow = (
  queryClient: QueryClient,
  noteId: string | undefined,
  counter: ThreadCounter,
): bigint | undefined =>
  noteId ? findCachedNote(queryClient, noteId)?.counters?.[counter] : undefined;

/** Read before a note is posted or deleted: what `settle…` compares with once the server answered. */
export function countsBefore(
  queryClient: QueryClient,
  targets: { replyToId?: string; quoteOfId?: string },
): CountsBefore {
  return {
    replies: counterNow(queryClient, targets.replyToId, "replyCount"),
    quotes: counterNow(queryClient, targets.quoteOfId, "quoteCount"),
  };
}

/**
 * Moves a counter of the note a reply or a quote hangs off, once the server
 * has confirmed the change — unless the count already moved meanwhile: the
 * realtime push with the new total often arrives before the answer does,
 * and the change must not be counted twice.
 */
function settleCounter(
  queryClient: QueryClient,
  noteId: string | undefined,
  counter: ThreadCounter,
  before: bigint | undefined,
  delta: number,
): void {
  if (!noteId) return;
  patchNote(queryClient, noteId, (note) =>
    (note.counters?.[counter] ?? 0n) === (before ?? 0n)
      ? { ...note, counters: bumpCounter(note, counter, delta) }
      : note,
  );
}

/** A note of the viewer's was deleted: the note it answered or quoted lost a reply or a quote. */
export function settleDeletedNote(
  queryClient: QueryClient,
  note: notes.Note,
  before: CountsBefore,
): void {
  settleCounter(queryClient, note.replyTo?.noteId, "replyCount", before.replies, -1);
  settleCounter(queryClient, note.quote?.noteId, "quoteCount", before.quotes, -1);
}

/**
 * A note the viewer just posted takes its place in the lists already on
 * screen. `before` holds the reply and quote counts of the note it answers
 * or quotes as they were when the request was sent.
 */
export function addPostedNote(
  queryClient: QueryClient,
  note: notes.Note,
  before: CountsBefore,
): void {
  const authorId = note.author?.id;
  const isReply = Boolean(note.replyTo?.noteId);
  // A channel note reaches home only through a followed channel, and never the global timeline.
  const inChannel = Boolean(note.channel?.id);
  if (!inChannel) prependNote(queryClient, { kind: "home" }, note);
  if (!isReply && !inChannel && note.viewer?.canQuote)
    prependNote(queryClient, { kind: "global" }, note);
  for (const tag of note.hashtags) prependNote(queryClient, { kind: "hashtag", tag }, note);
  if (authorId) {
    if (!isReply)
      prependNote(
        queryClient,
        { kind: "user", userId: authorId, filter: notes.UserTimelineFilter.NOTES },
        note,
      );
    prependNote(
      queryClient,
      { kind: "user", userId: authorId, filter: notes.UserTimelineFilter.WITH_REPLIES },
      note,
    );
    if (note.attachments.length > 0)
      void queryClient.invalidateQueries({
        queryKey: notesKeys.timeline({
          kind: "user",
          userId: authorId,
          filter: notes.UserTimelineFilter.MEDIA,
        }),
      });
  }

  const parentId = note.replyTo?.noteId;
  if (parentId) {
    settleCounter(queryClient, parentId, "replyCount", before.replies, 1);
    // Replies are listed oldest first: the new one belongs at the end, which is only on screen once every page is.
    queryClient.setQueryData<ThreadData>(notesKeys.thread(parentId), (thread) => {
      const last = thread?.pages.at(-1);
      if (!thread || !last || last.page?.nextCursor) return thread;
      return {
        ...thread,
        pages: thread.pages.map((page) =>
          page === last
            ? { ...page, replies: [...page.replies.filter((reply) => reply.id !== note.id), note] }
            : page,
        ),
      };
    });
  }
  const quotedId = note.quote?.noteId;
  if (quotedId) {
    settleCounter(queryClient, quotedId, "quoteCount", before.quotes, 1);
    void queryClient.invalidateQueries({ queryKey: notesKeys.quotes(quotedId) });
  }
}
