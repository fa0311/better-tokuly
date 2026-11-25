import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { notes } from "@tokuly/api";
import { storyQueries } from "@tokuly/core/stories";
import { api } from "#/lib/api";

// Keys, query options and cache edits live in `@tokuly/core/stories`; the folder reads them from here.
export {
  addStory,
  forgetUserStories,
  patchStory,
  removeStory,
  storyCover,
  storyKeys,
} from "@tokuly/core/stories";

export const storyClient = () => api().service(notes.StoryService);

/** The story tray: the viewer's own reel first, then friends with something unseen, then the rest. */
export function useStoryTray() {
  return useQuery(storyQueries.tray(storyClient));
}

/** The live stories of one person that the viewer may see, oldest first. */
export function useUserStories(userId: string, enabled = true) {
  return useQuery({
    ...storyQueries.user(storyClient, userId),
    enabled: enabled && Boolean(userId),
  });
}

export function useStory(storyId: string) {
  return useQuery({ ...storyQueries.story(storyClient, storyId), enabled: Boolean(storyId) });
}

/** Who watched one of the viewer's stories and how they reacted, most recent first. */
export function useStoryViewers(storyId: string) {
  return useInfiniteQuery(storyQueries.viewers(storyClient, storyId));
}

/** The days of a month on which the viewer posted stories. */
export function useStoryCalendar(year: number, month: number) {
  return useQuery(storyQueries.calendar(storyClient, year, month));
}

/** The viewer's own stories, live and archived, newest first; `date` (`YYYY-MM-DD`) narrows to one day. */
export function useArchivedStories(date: string, enabled = true) {
  return useInfiniteQuery({ ...storyQueries.archive(storyClient, date), enabled });
}

/** The highlights on a profile that hold something the viewer may see. */
export function useHighlights(userId: string) {
  return useQuery({ ...storyQueries.highlights(storyClient, userId), enabled: Boolean(userId) });
}
