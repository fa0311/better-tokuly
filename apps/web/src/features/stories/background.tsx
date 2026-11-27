import { useQueryClient } from "@tanstack/react-query";
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
    let connectedBefore = false;
    const offWelcome = client.onFrame((frame) => {
      if (frame.body.case !== "welcome") return;
      // Stories posted while the connection was down were not announced.
      if (connectedBefore) void queryClient.invalidateQueries({ queryKey: storyKeys.tray });
      connectedBefore = true;
    });
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
    const offSocial = onFramesLoading("social", () =>
      import("./frames").then(
        ({ applySocialEvent }) =>
          ({ event }) =>
            applySocialEvent(queryClient, event),
      ),
    );
    client.start();
    return () => {
      offWelcome();
      offNotes();
      offSocial();
    };
  }, [queryClient]);

  return <StoryViewerHost />;
}
