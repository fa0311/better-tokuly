import type { notes } from "@tokuly/api";

/** One photo or video of a reel, as the viewer plays it: stories hold up to ten each. */
export interface StorySegment {
  story: notes.Story;
  item: notes.StoryItem;
  /** Unique within the reel: `{story ID}:{item index}`. */
  key: string;
}

/** A reel's stories (oldest first) as the pieces that play one after the other. */
export function storySegments(stories: readonly notes.Story[]): StorySegment[] {
  return stories.flatMap((story) =>
    story.items.map((item, index) => ({ story, item, key: `${story.id}:${index}` })),
  );
}

/**
 * Where a reel starts: at the story asked for, else at the first one the
 * viewer has not watched yet, else at the beginning.
 */
export function reelStart(stories: readonly notes.Story[], storyId?: string): number {
  const segments = storySegments(stories);
  const wanted = storyId
    ? segments.findIndex((segment) => segment.story.id === storyId)
    : segments.findIndex((segment) => !segment.story.viewed);
  return Math.max(0, wanted);
}
