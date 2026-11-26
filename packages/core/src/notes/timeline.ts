import type { InfiniteData } from "@tanstack/query-core";
import type { notes } from "@tokuly/api";
import type { ClientSource } from "../query";
import type { TimelineSource } from "./keys";

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
  limit = PAGE_SIZE,
): Promise<TimelinePage> {
  const page = { cursor, limit };
  const client = clients.notes();
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
    case "list": {
      const response = await clients.lists().getListTimeline({ listId: source.listId, page });
      return { entries: response.entries, nextCursor: response.page?.nextCursor ?? "" };
    }
    case "antenna": {
      const response = await clients
        .antennas()
        .getAntennaTimeline({ antennaId: source.antennaId, page });
      return { entries: response.entries, nextCursor: response.page?.nextCursor ?? "" };
    }
  }
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
