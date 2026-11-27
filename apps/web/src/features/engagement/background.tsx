import { useQueryClient } from "@tanstack/react-query";
import { readReaction } from "@tokuly/core/engagement";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";
import { applyCounters, engagementKeys } from "./api";
import { CollectionChooserHost } from "./collection-chooser-host";
import { emitPop } from "./pop-events";

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
      if (subject && event.case === "reactionAdded") {
        const reaction = readReaction(event.value.reaction);
        if (reaction) emitPop(subject.type, subject.id, reaction);
      }
    });
    client.start();
    return off;
  }, [queryClient]);

  return <CollectionChooserHost />;
}
