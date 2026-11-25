import type { InfiniteData, QueryClient } from "@tanstack/query-core";
import { type common, create, notes, toMillis } from "@tokuly/api";
import { storyKeys } from "./keys";

/** The story caches kept in step with what the viewer does and what realtime announces. */

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
  queryClient.setQueryData<Reels>(storyKeys.tray, (reels) =>
    reels?.map((reel) => {
      const stories = applyAll(reel.stories);
      return stories === reel.stories
        ? reel
        : { ...reel, stories, hasUnseen: stories.some((story) => !story.viewed) };
    }),
  );
  queryClient.setQueriesData<notes.Story[]>(
    { queryKey: ["stories", "user"] },
    (stories) => stories && applyAll(stories),
  );
  queryClient.setQueryData<notes.Story>(storyKeys.story(storyId), (story) => story && apply(story));
  queryClient.setQueriesData<InfiniteData<notes.ListArchivedStoriesResponse>>(
    { queryKey: storyKeys.archives },
    (data) =>
      data && {
        ...data,
        pages: data.pages.map((page) => ({ ...page, stories: applyAll(page.stories) })),
      },
  );
  queryClient.setQueriesData<notes.GetHighlightResponse>(
    { queryKey: ["stories", "highlight"] },
    (data) => data && { ...data, stories: applyAll(data.stories) },
  );
}

/** A story that was just posted (by the viewer, or announced over realtime) joins its owner's reel in the tray. */
export function addStory(queryClient: QueryClient, story: notes.Story, myId: string): void {
  const owner = story.owner;
  if (!owner) return;
  const mine = owner.id === myId;
  queryClient.setQueryData<Reels>(storyKeys.tray, (reels) => {
    if (!reels) return reels;
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
  queryClient.setQueryData<notes.Story[]>(
    storyKeys.user(owner.id),
    (stories) => stories && [...stories.filter((item) => item.id !== story.id), story],
  );
  if (mine) {
    void queryClient.invalidateQueries({ queryKey: storyKeys.archives });
    void queryClient.invalidateQueries({ queryKey: storyKeys.calendars });
  }
}

/** A deleted story leaves every list; a reel left without stories leaves the tray. */
export function removeStory(queryClient: QueryClient, storyId: string): void {
  const without = (stories: notes.Story[]) =>
    stories.some((story) => story.id === storyId)
      ? stories.filter((story) => story.id !== storyId)
      : stories;
  queryClient.setQueryData<Reels>(storyKeys.tray, (reels) =>
    reels?.flatMap((reel) => {
      const stories = without(reel.stories);
      if (stories === reel.stories) return [reel];
      return stories.length === 0
        ? []
        : [{ ...reel, stories, hasUnseen: stories.some((story) => !story.viewed) }];
    }),
  );
  queryClient.setQueriesData<notes.Story[]>(
    { queryKey: ["stories", "user"] },
    (stories) => stories && without(stories),
  );
  queryClient.removeQueries({ queryKey: storyKeys.story(storyId) });
  queryClient.setQueriesData<InfiniteData<notes.ListArchivedStoriesResponse>>(
    { queryKey: storyKeys.archives },
    (data) =>
      data && {
        ...data,
        pages: data.pages.map((page) => ({ ...page, stories: without(page.stories) })),
      },
  );
  void queryClient.invalidateQueries({ queryKey: storyKeys.calendars });
  void queryClient.invalidateQueries({ queryKey: ["stories", "highlights"] });
  void queryClient.invalidateQueries({ queryKey: ["stories", "highlight"] });
}

/**
 * A friendship ended or a block went up: that person's reel leaves the tray
 * and what was cached of their stories and highlights is thrown away.
 */
export function forgetUserStories(queryClient: QueryClient, userId: string): void {
  if (!userId) return;
  queryClient.setQueryData<Reels>(storyKeys.tray, (reels) =>
    reels?.some((reel) => reel.user?.id === userId)
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

/** The picture that stands for a story: its first photo, or its first video's poster. */
export function storyCover(story: notes.Story): common.Image | undefined {
  const first = story.items[0]?.media;
  return first?.video?.poster ?? first?.image;
}
