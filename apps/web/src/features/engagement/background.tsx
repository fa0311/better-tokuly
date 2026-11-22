import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";
import { applyCounters, engagementKeys } from "./api";
import { CollectionChooserHost } from "./collection-chooser";

/**
 * Runs while someone is signed in: applies the counters the realtime
 * connection pushes for subjects on screen (see `useSubjectTopic`) to the
 * cached engagement, so reaction chips, save and share counts move by
 * themselves. It also hosts the collection chooser, which any save button
 * (or the toast after saving) opens.
 */
export default function EngagementBackground() {
  const queryClient = useQueryClient();

  useEffect(() => {
    const client = realtime();
    let connectedBefore = false;
    const off = client.onFrame((frame) => {
      if (frame.body.case === "welcome") {
        // Counters that changed while the connection was down were missed: read them again.
        if (connectedBefore)
          void queryClient.invalidateQueries({ queryKey: engagementKeys.subjects });
        connectedBefore = true;
        return;
      }
      if (frame.body.case !== "engagement") return;
      const { subject, event } = frame.body.value;
      if (subject && event.case === "countersChanged")
        applyCounters(queryClient, subject.type, subject.id, event.value);
    });
    client.start();
    return off;
  }, [queryClient]);

  return <CollectionChooserHost />;
}
