import { useQueryClient } from "@tanstack/react-query";
import { common } from "@tokuly/api";
import { useEffect } from "react";
import { registerSubjectCard } from "#/features/engagement";
import { realtime } from "#/lib/realtime";
import { applyCommunityEvent } from "./live";
import { CommunitySubjectCard } from "./subject-card";

// Saved and reacted-to community pages open at this app's addresses for them.
for (const type of [
  common.SubjectType.COMMUNITY,
  common.SubjectType.COMMUNITY_THREAD,
  common.SubjectType.COMMUNITY_REPLY,
  common.SubjectType.COMMUNITY_EVENT,
])
  registerSubjectCard(type, CommunitySubjectCard);

/**
 * Runs while someone is signed in: keeps what is loaded about communities
 * current from the realtime connection. Events of a community's topic
 * arrive while one of its pages is open (it follows the topic); the
 * caller's own standing and the staff's queues arrive always.
 */
export default function CommunityBackground() {
  const queryClient = useQueryClient();

  useEffect(() => {
    const client = realtime();
    // An open community reads its boards again when its topic is back after a gap
    // (`CommunityLayout`).
    const off = client.onFrame((frame) => {
      if (frame.body.case === "community") applyCommunityEvent(queryClient, frame.body.value.event);
    });
    client.start();
    return off;
  }, [queryClient]);

  return null;
}
