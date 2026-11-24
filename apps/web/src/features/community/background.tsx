import { useQueryClient } from "@tanstack/react-query";
import { common } from "@tokuly/api";
import { useEffect } from "react";
import { registerSubjectCard } from "#/features/engagement";
import { realtime, rereadListsAfterReconnect } from "#/lib/realtime";
import { keys } from "./api";
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
    const off = client.onFrame((frame) => {
      if (frame.body.case === "community") applyCommunityEvent(queryClient, frame.body.value.event);
    });
    // What changed while the connection was down was not announced: the lists on screen (my
    // communities, invitations, queues) are read again. An open community's boards read
    // themselves again when its topic is back (`CommunityLayout`).
    const stop = rereadListsAfterReconnect(
      queryClient,
      keys.all,
      (key) => key[1] === "in" && client.isSubscribed(`community:${String(key[2])}`),
    );
    client.start();
    return () => {
      off();
      stop();
    };
  }, [queryClient]);

  return null;
}
