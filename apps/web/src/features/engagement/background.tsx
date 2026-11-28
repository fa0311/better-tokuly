import { useQueryClient } from "@tanstack/react-query";
import { engagementKeys } from "@tokuly/core/engagement/keys";
import { rereadOnWelcome } from "@tokuly/core/query";
import { useEffect } from "react";
import { onFramesLoading, realtime } from "#/lib/realtime";
import { CollectionChooserHost } from "./collection-chooser-host";

/**
 * Runs while someone is signed in: applies the counters the realtime
 * connection pushes for subjects on screen (see `useSubjectTopic`) to the
 * cached engagement, so reaction chips, save and share counts move by
 * themselves, and lets the reactions others add pop over the subject's
 * reaction bar (`reaction_added`). It also hosts the collection chooser, which any save button
 * (or the toast after saving) opens.
 */
export default function EngagementBackground() {
  const queryClient = useQueryClient();

  useEffect(() => {
    const client = realtime();
    // Counters that changed before the connection was up, or while it was down, were missed.
    const offWelcome = rereadOnWelcome(queryClient, client, [engagementKeys.subjects]);
    const offEvents = onFramesLoading("engagement", () =>
      import("./frames").then(
        ({ applyEngagementEvent }) =>
          (value) =>
            applyEngagementEvent(queryClient, value),
      ),
    );
    client.start();
    return () => {
      offWelcome();
      offEvents();
    };
  }, [queryClient]);

  return <CollectionChooserHost />;
}
