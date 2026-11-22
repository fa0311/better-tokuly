import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";
import { useSessionStore } from "#/stores/session";
import { addStory, patchStory, removeStory, storyKeys } from "./api";
import { StoryViewerHost } from "./viewer";

/**
 * Runs while someone is signed in: keeps the story tray current from the
 * realtime connection (a friend posted or deleted a story; someone watched
 * or reacted to one of the viewer's own), and hosts the full-screen viewer
 * so any page can play stories.
 */
export default function StoriesBackground() {
  const queryClient = useQueryClient();

  useEffect(() => {
    const client = realtime();
    let connectedBefore = false;
    const off = client.onFrame((frame) => {
      if (frame.body.case === "welcome") {
        // Stories posted while the connection was down were not announced.
        if (connectedBefore) void queryClient.invalidateQueries({ queryKey: storyKeys.tray });
        connectedBefore = true;
        return;
      }
      if (frame.body.case !== "notes") return;
      const event = frame.body.value.event;
      const session = useSessionStore.getState().state;
      const myId = session.status === "signedIn" ? (session.me.user?.id ?? "") : "";
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
    });
    client.start();
    return off;
  }, [queryClient]);

  return <StoryViewerHost />;
}
