import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { notes } from "@tokuly/api";
import {
  addStory,
  forgetUserStories,
  patchStory,
  removeStory,
  storyKeys,
  storyQueries,
} from "@tokuly/core/stories";
import { useEffect } from "react";
import { useMe } from "#/features/session/session";
import { api } from "#/lib/api";
import { realtime } from "#/lib/realtime";

/**
 * Stories (記憶) in the query cache. Keys, query options and cache edits are
 * `@tokuly/core/stories`; this file wires them to the phone's API client and
 * the realtime connection.
 */
export const storyClient = () => api().service(notes.StoryService);

/** The tray: the viewer's own reel first, then friends with something unseen, then the rest. */
export function useStoryTray() {
  return useQuery(storyQueries.tray(storyClient));
}

/** The live stories of one person the viewer may see, oldest first. */
export function useUserStories(userId: string, enabled = true) {
  return useQuery({
    ...storyQueries.user(storyClient, userId),
    enabled: enabled && Boolean(userId),
  });
}

/** Who watched one of the viewer's stories, most recent first. */
export function useStoryViewers(storyId: string, enabled: boolean) {
  return useInfiniteQuery({ ...storyQueries.viewers(storyClient, storyId), enabled });
}

/**
 * Stories stay current while someone is signed in: a friend posted or
 * deleted one, someone watched or reacted to one of the viewer's own, a
 * friendship ended; a new connection reads the tray again.
 */
export function useStoriesSync(): void {
  const queryClient = useQueryClient();
  const myId = useMe()?.user?.id ?? "";
  useEffect(() => {
    if (!myId) return;
    const client = realtime();
    let connectedBefore = false;
    const off = client.onFrame((frame) => {
      if (frame.topic) return;
      if (frame.body.case === "welcome") {
        if (connectedBefore) void queryClient.invalidateQueries({ queryKey: storyKeys.tray });
        connectedBefore = true;
        return;
      }
      if (frame.body.case === "social" && frame.body.value.event.case === "friendRemovedUserId") {
        forgetUserStories(queryClient, frame.body.value.event.value);
        return;
      }
      if (frame.body.case !== "notes") return;
      const event = frame.body.value.event;
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
        case "storyReaction":
          void queryClient.invalidateQueries({ queryKey: storyKeys.story(event.value.storyId) });
          void queryClient.invalidateQueries({ queryKey: storyKeys.viewers(event.value.storyId) });
          break;
        default:
          break;
      }
    });
    client.start();
    return off;
  }, [queryClient, myId]);
}
