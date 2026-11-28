import { useQueryClient } from "@tanstack/react-query";
import { rereadOnWelcome } from "@tokuly/core/query";
import { storyKeys } from "@tokuly/core/stories/keys";
import { useEffect } from "react";
import { onFramesLoading, realtime } from "#/lib/realtime";
import { useSessionStore } from "#/stores/session";
import { StoryViewerHost } from "./viewer-host";

/**
 * Runs while someone is signed in: keeps the story tray current from the
 * realtime connection (`./frames`), and hosts the full-screen viewer so any
 * page can play stories.
 */
export default function StoriesBackground() {
  const queryClient = useQueryClient();

  useEffect(() => {
    const client = realtime();
    // Stories posted before the connection was up, or while it was down, were not announced.
    const offWelcome = rereadOnWelcome(queryClient, client, [storyKeys.tray]);
    const offNotes = onFramesLoading("notes", () =>
      import("./frames").then(({ applyStoryEvent }) => ({ event }) => {
        const session = useSessionStore.getState().state;
        applyStoryEvent(
          queryClient,
          event,
          session.status === "signedIn" ? (session.me.user?.id ?? "") : "",
        );
      }),
    );
    client.start();
    return () => {
      offWelcome();
      offNotes();
    };
  }, [queryClient]);

  return <StoryViewerHost />;
}
