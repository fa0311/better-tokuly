import type { QueryClient } from "@tanstack/react-query";
import type { notes } from "@tokuly/api";
import { patchStory } from "@tokuly/core/stories";
import { storyClient } from "./data";

/**
 * Watching a friend's story counts once, when it is really shown (its
 * picture is up, the app in front). The viewer's own stories never count,
 * nor one already watched. Every cached copy shows it watched once the
 * server has it; a failed mark is tried again the next time it is shown.
 */
export function seenMarker(
  queryClient: QueryClient,
  client = storyClient,
): (story: notes.Story, mine: boolean) => void {
  const marked = new Set<string>();
  return (story, mine) => {
    if (mine || story.viewed || marked.has(story.id)) return;
    marked.add(story.id);
    // The client is taken now: the mark belongs to the account that watched.
    const stories = client();
    stories
      .markStoryViewed({ storyId: story.id })
      .then(() => patchStory(queryClient, story.id, (current) => ({ ...current, viewed: true })))
      .catch(() => marked.delete(story.id));
  };
}
