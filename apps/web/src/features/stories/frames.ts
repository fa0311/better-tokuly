import type { QueryClient } from "@tanstack/react-query";
import type { notes } from "@tokuly/api";
import { addStory, patchStory, removeStory, storyKeys } from "@tokuly/core/stories";

/**
 * What the realtime connection says about stories, applied to the tray and the viewer: a friend
 * posted or deleted a story; someone watched or reacted to one of the viewer's own. Loaded with
 * the first such event.
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
      void queryClient.invalidateQueries({ queryKey: storyKeys.viewers(storyId) });
      break;
    }
    case "storyReaction": {
      // Whether it is a new reaction or a changed one is not said: read the story again.
      const { storyId } = event.value;
      void queryClient.invalidateQueries({ queryKey: storyKeys.story(storyId) });
      void queryClient.invalidateQueries({ queryKey: storyKeys.viewers(storyId) });
      break;
    }
    default:
      break;
  }
}
