import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { notes } from "@tokuly/api";
import { rereadOnWelcome } from "@tokuly/core/query";
import { applyStoryEvent, storyKeys, storyQueries } from "@tokuly/core/stories";
import { useEffect } from "react";
import { useMe } from "#/features/session/store";
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
 * deleted one, someone watched or reacted to one of the viewer's own; a new
 * connection reads the tray again.
 */
export function useStoriesSync(): void {
  const queryClient = useQueryClient();
  const myId = useMe()?.user?.id ?? "";
  useEffect(() => {
    if (!myId) return;
    const client = realtime();
    // Stories posted before the connection was up, or while it was down, were not announced.
    const offWelcome = rereadOnWelcome(queryClient, client, [storyKeys.tray]);
    const off = client.onFrame((frame) => {
      if (frame.topic) return;
      if (frame.body.case === "notes") applyStoryEvent(queryClient, frame.body.value.event, myId);
    });
    client.start();
    return () => {
      off();
      offWelcome();
    };
  }, [queryClient, myId]);
}
