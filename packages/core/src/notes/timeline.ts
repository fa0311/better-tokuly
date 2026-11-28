import type { InfiniteData, QueryClient } from "@tanstack/query-core";
import type { notes } from "@tokuly/api";
import { type ClientSource, journal, pushAfter } from "../query";
import { notesKeys, type TimelineSource } from "./keys";

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

/** The services timelines are read from, as the app's clients. */
export interface TimelineClients {
  notes: ClientSource<typeof notes.NoteService>;
  lists: ClientSource<typeof notes.NoteListService>;
  antennas: ClientSource<typeof notes.AntennaService>;
}

/** Reads one page of a timeline, from `cursor` (`""` = the newest). */
export async function loadTimelinePage(
  clients: TimelineClients,
  source: TimelineSource,
  cursor: string,
  signal?: AbortSignal,
): Promise<TimelinePage> {
  const page = { cursor, limit: PAGE_SIZE };
  const options = { signal };
  const client = clients.notes();
  switch (source.kind) {
    case "home": {
      const response = await client.getHomeTimeline({ page }, options);
      return { entries: response.entries, nextCursor: response.page?.nextCursor ?? "" };
    }
    case "following": {
      const response = await client.getFollowingTimeline({ page }, options);
      return { entries: response.entries, nextCursor: response.page?.nextCursor ?? "" };
    }
    case "global": {
      const response = await client.getGlobalTimeline({ page }, options);
      return { entries: response.entries, nextCursor: response.page?.nextCursor ?? "" };
    }
    case "mentions": {
      const response = await client.getMentionsTimeline({ page }, options);
      return { entries: response.entries, nextCursor: response.page?.nextCursor ?? "" };
    }
    case "user": {
      const response = await client.getUserTimeline(
        { userId: source.userId, filter: source.filter, page },
        options,
      );
      return {
        entries: response.entries,
        nextCursor: response.page?.nextCursor ?? "",
        pinnedNotes: response.pinnedNotes,
      };
    }
    case "hashtag": {
      const response = await client.getHashtagTimeline({ tag: source.tag, page }, options);
      return {
        entries: response.entries,
        nextCursor: response.page?.nextCursor ?? "",
        hashtag: response.hashtag,
      };
    }
    case "list": {
      const response = await clients
        .lists()
        .getListTimeline({ listId: source.listId, page }, options);
      return { entries: response.entries, nextCursor: response.page?.nextCursor ?? "" };
    }
    case "antenna": {
      const response = await clients
        .antennas()
        .getAntennaTimeline({ antennaId: source.antennaId, page }, options);
      return { entries: response.entries, nextCursor: response.page?.nextCursor ?? "" };
    }
  }
}

/**
 * The query of a timeline, newest first, paged. It never goes stale by
 * itself: a refetch would read every loaded page again. New notes are put on
 * top instead (`lookForNewEntries`). Its reads go through the journal, so a
 * push made while a page is read is applied to it (`docs/realtime-consistency.md`).
 */
export function timelineQuery(clients: TimelineClients, source: TimelineSource) {
  return {
    queryKey: notesKeys.timeline(source),
    queryFn: ({ pageParam, signal }: { pageParam: string; signal: AbortSignal }) =>
      loadTimelinePage(clients, source, pageParam, signal),
    initialPageParam: "",
    getNextPageParam: (last: TimelinePage) => last.nextCursor || undefined,
    staleTime: Number.POSITIVE_INFINITY,
    persister: journal,
  };
}

/**
 * Reads the newest page of a timeline and puts its new rows on top
 * (`withFreshEntries`) once `take` agrees: at once (`true`), not at all
 * (`false`), or when the promise it returns settles (the "new notes" pill).
 * `take` is called as soon as the page is read, with how many of its rows are
 * new (`countNewEntries`).
 *
 * A look is a read that writes the timeline (`pushAfter`): what was pushed
 * from its start until its rows go on top — a person forgotten, a note's
 * counters — is applied after them again, so a page read before a removal
 * brings nothing back.
 */
export function lookForNewEntries(
  queryClient: QueryClient,
  source: TimelineSource,
  read: () => Promise<TimelinePage>,
  take: (count: number | undefined) => boolean | Promise<boolean>,
): Promise<void> {
  const key = notesKeys.timeline(source);
  return pushAfter<TimelineData>(queryClient, key, async () => {
    const page = await read();
    const count = countNewEntries(queryClient.getQueryData<TimelineData>(key), page);
    if (!(await take(count))) return undefined;
    return (current) => withFreshEntries(current, page);
  });
}

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
