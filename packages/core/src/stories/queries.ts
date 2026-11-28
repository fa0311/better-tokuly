import type { notes } from "@tokuly/api";
import { type ClientSource, journal, nextCursor } from "../query";
import { storyKeys } from "./keys";

type Stories = ClientSource<typeof notes.StoryService>;

/**
 * Query options of stories. Each app adds `enabled` and its hooks around them.
 * The lists realtime changes read through the journal (`pushTo` in `./cache`).
 */
export const storyQueries = {
  /** The story tray: the viewer's own reel first, then friends with something unseen, then the rest. */
  tray: (client: Stories) => ({
    queryKey: storyKeys.tray,
    queryFn: async ({ signal }: { signal: AbortSignal }) =>
      (await client().getStoryTray({}, { signal })).reels,
    persister: journal,
    staleTime: 60_000,
  }),

  /** The live stories of one person that the viewer may see, oldest first. */
  user: (client: Stories, userId: string) => ({
    queryKey: storyKeys.user(userId),
    queryFn: async ({ signal }: { signal: AbortSignal }) =>
      (await client().listUserStories({ userId }, { signal })).stories,
    persister: journal,
    staleTime: 60_000,
  }),

  story: (client: Stories, storyId: string) => ({
    queryKey: storyKeys.story(storyId),
    queryFn: async () => (await client().getStory({ storyId })).story,
  }),

  /** Who watched one of the viewer's stories and how they reacted, most recent first. */
  viewers: (client: Stories, storyId: string) => ({
    queryKey: storyKeys.viewers(storyId),
    queryFn: ({ pageParam }: { pageParam: string }) =>
      client().listStoryViewers({ storyId, page: { cursor: pageParam, limit: 30 } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  }),

  /** The days of a month on which the viewer posted stories. */
  calendar: (client: Stories, year: number, month: number) => ({
    queryKey: storyKeys.calendar(year, month),
    queryFn: async () => (await client().getStoryCalendar({ year, month })).days,
  }),

  /** The viewer's own stories, live and archived, newest first; `date` (`YYYY-MM-DD`) narrows to one day. */
  archive: (client: Stories, date: string) => ({
    queryKey: storyKeys.archive(date),
    queryFn: ({ pageParam, signal }: { pageParam: string; signal: AbortSignal }) =>
      client().listArchivedStories({ date, page: { cursor: pageParam, limit: 30 } }, { signal }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
    persister: journal,
  }),

  /** The highlights on a profile that hold something the viewer may see. */
  highlights: (client: Stories, userId: string) => ({
    queryKey: storyKeys.highlights(userId),
    queryFn: async () => (await client().listHighlights({ userId })).highlights,
  }),
};
