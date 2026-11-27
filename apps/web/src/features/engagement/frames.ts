import type { QueryClient } from "@tanstack/react-query";
import type { engagement } from "@tokuly/api";
import { readReaction } from "@tokuly/core/engagement";
import { applyCounters } from "./api";
import { emitPop } from "./pop-events";

/**
 * Applies the counters pushed for a subject on screen to the cached engagement, and lets a
 * reaction someone else added pop over its reaction bar. `background.tsx` loads this with the
 * first engagement event.
 */
export function applyEngagementEvent(
  queryClient: QueryClient,
  { subject, event }: engagement.EngagementEvent,
): void {
  if (subject && event.case === "countersChanged")
    applyCounters(queryClient, subject.type, subject.id, event.value);
  if (subject && event.case === "reactionAdded") {
    const reaction = readReaction(event.value.reaction);
    if (reaction) emitPop(subject.type, subject.id, reaction);
  }
}
