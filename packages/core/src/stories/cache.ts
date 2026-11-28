import type { InfiniteData, QueryClient } from "@tanstack/query-core";
import { type common, create, notes, toMillis } from "@tokuly/api";
import { pushTo, pushToAll, reread } from "../query";
import { storyKeys } from "./keys";

/**
 * The story caches kept in step with what the viewer does and what realtime
 * announces. Every change goes through the journal (`pushTo`), so a read in
 * flight when it was pushed (the tray read again on a Welcome) does not undo
 * it: a deleted story, or a person forgotten, does not come back.
 */

type Reels = notes.StoryReel[];

function sortReels(reels: Reels, myId: string): Reels {
  const rank = (reel: notes.StoryReel) => (reel.user?.id === myId ? 0 : reel.hasUnseen ? 1 : 2);
  return [...reels].sort(
    (a, b) => rank(a) - rank(b) || toMillis(b.latestAt) - toMillis(a.latestAt),
  );
}

/** Applies `update` to a story wherever it is cached: the tray, a person's reel, the story itself, the archive, a highlight. */
export function patchStory(
  queryClient: QueryClient,
  storyId: string,
  update: (story: notes.Story) => notes.Story,
): void {
  const apply = (story: notes.Story) => (story.id === storyId ? update(story) : story);
  const applyAll = (stories: notes.Story[]) => {
    const next = stories.map(apply);
    return next.some((story, index) => story !== stories[index]) ? next : stories;
  };
  pushTo<Reels>(queryClient, storyKeys.tray, (reels) => {
    const next = reels.map((reel) => {
      const stories = applyAll(reel.stories);
      return stories === reel.stories
        ? reel
        : { ...reel, stories, hasUnseen: stories.some((story) => !story.viewed) };
    });
    return next.some((reel, index) => reel !== reels[index]) ? next : reels;
  });
  pushToAll<notes.Story[]>(queryClient, ["stories", "user"], applyAll);
  pushTo<notes.Story>(queryClient, storyKeys.story(storyId), apply);
  pushToAll<InfiniteData<notes.ListArchivedStoriesResponse>>(
    queryClient,
    storyKeys.archives,
    (data) => onArchivePages(data, applyAll),
  );
  pushToAll<notes.GetHighlightResponse>(queryClient, ["stories", "highlight"], (data) => {
    const stories = applyAll(data.stories);
    return stories === data.stories ? data : { ...data, stories };
  });
}

/** The archive with `edit` applied to every page; the same archive when no page changed. */
function onArchivePages(
  data: InfiniteData<notes.ListArchivedStoriesResponse>,
  edit: (stories: notes.Story[]) => notes.Story[],
): InfiniteData<notes.ListArchivedStoriesResponse> {
  const pages = data.pages.map((page) => {
    const stories = edit(page.stories);
    return stories === page.stories ? page : { ...page, stories };
  });
  return pages.some((page, index) => page !== data.pages[index]) ? { ...data, pages } : data;
}

/** A story that was just posted (by the viewer, or announced over realtime) joins its owner's reel in the tray. */
export function addStory(queryClient: QueryClient, story: notes.Story, myId: string): void {
  const owner = story.owner;
  if (!owner) return;
  const mine = owner.id === myId;
  pushTo<Reels>(queryClient, storyKeys.tray, (reels) => {
    const existing = reels.find((reel) => reel.user?.id === owner.id);
    const stories = [...(existing?.stories.filter((item) => item.id !== story.id) ?? []), story];
    const reel = create(notes.StoryReelSchema, {
      user: owner,
      stories,
      hasUnseen: !mine && stories.some((item) => !item.viewed),
      latestAt: story.createdAt,
    });
    return sortReels([...reels.filter((item) => item.user?.id !== owner.id), reel], myId);
  });
  pushTo<notes.Story[]>(queryClient, storyKeys.user(owner.id), (stories) => [
    ...stories.filter((item) => item.id !== story.id),
    story,
  ]);
  if (mine) void reread(queryClient, [storyKeys.archives, storyKeys.calendars]);
}

/** A deleted story leaves every list; a reel left without stories leaves the tray. */
export function removeStory(queryClient: QueryClient, storyId: string): void {
  const without = (stories: notes.Story[]) =>
    stories.some((story) => story.id === storyId)
      ? stories.filter((story) => story.id !== storyId)
      : stories;
  pushTo<Reels>(queryClient, storyKeys.tray, (reels) => {
    const next = reels.flatMap((reel) => {
      const stories = without(reel.stories);
      if (stories === reel.stories) return [reel];
      return stories.length === 0
        ? []
        : [{ ...reel, stories, hasUnseen: stories.some((story) => !story.viewed) }];
    });
    return next.length === reels.length && next.every((reel, index) => reel === reels[index])
      ? reels
      : next;
  });
  pushToAll<notes.Story[]>(queryClient, ["stories", "user"], without);
  queryClient.removeQueries({ queryKey: storyKeys.story(storyId) });
  pushToAll<InfiniteData<notes.ListArchivedStoriesResponse>>(
    queryClient,
    storyKeys.archives,
    (data) => onArchivePages(data, without),
  );
  void reread(queryClient, [
    storyKeys.calendars,
    ["stories", "highlights"],
    ["stories", "highlight"],
  ]);
}

/**
 * A friendship ended or a block went up: that person's reel leaves the tray
 * and what was cached of their stories and highlights is thrown away.
 */
export function forgetUserStories(queryClient: QueryClient, userId: string): void {
  if (!userId) return;
  pushTo<Reels>(queryClient, storyKeys.tray, (reels) =>
    reels.some((reel) => reel.user?.id === userId)
      ? reels.filter((reel) => reel.user?.id !== userId)
      : reels,
  );
  queryClient.removeQueries({ queryKey: storyKeys.user(userId) });
  queryClient.removeQueries({ queryKey: storyKeys.highlights(userId) });
  for (const query of queryClient.getQueryCache().findAll({ queryKey: ["stories", "story"] })) {
    if ((query.state.data as notes.Story | undefined)?.owner?.id === userId)
      queryClient.removeQueries({ queryKey: query.queryKey, exact: true });
  }
  for (const query of queryClient.getQueryCache().findAll({ queryKey: ["stories", "highlight"] })) {
    if ((query.state.data as notes.GetHighlightResponse | undefined)?.highlight?.ownerId === userId)
      queryClient.removeQueries({ queryKey: query.queryKey, exact: true });
  }
}

/**
 * What a pushed stories event changes in the cache: a friend posted or
 * deleted a story; someone watched or reacted to one of the viewer's own.
 */
export function applyStoryEvent(
  queryClient: QueryClient,
  event: notes.NotesEvent["event"],
  myId: string,
): void {
  switch (event.case) {
    case "storyPosted":
      addStory(queryClient, event.value, myId);
      break;
    case "storyDeleted":
      removeStory(queryClient, event.value.storyId);
      break;
    case "storyViewed": {
      const { storyId, viewCount } = event.value;
      patchStory(queryClient, storyId, (story) => ({ ...story, viewCount }));
      void reread(queryClient, [storyKeys.viewers(storyId)]);
      break;
    }
    case "storyReaction": {
      // Whether it is a new reaction or a changed one is not said: read the story again.
      const { storyId } = event.value;
      void reread(queryClient, [storyKeys.story(storyId), storyKeys.viewers(storyId)]);
      break;
    }
    default:
      break;
  }
}

/** The picture that stands for a story: its first photo, or its first video's poster. */
export function storyCover(story: notes.Story): common.Image | undefined {
  const first = story.items[0]?.media;
  return first?.video?.poster ?? first?.image;
}
