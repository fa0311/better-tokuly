import { useQueryClient } from "@tanstack/react-query";
import { common } from "@tokuly/api";
import { communityKeys } from "@tokuly/core/community/keys";
import { rereadOnWelcome } from "@tokuly/core/query";
import { lazy, useEffect } from "react";
import { onFramesLoading, realtime } from "#/lib/realtime";

const CommunitySubjectCard = lazy(() =>
  import("./subject-card").then((module) => ({ default: module.CommunitySubjectCard })),
);

// Saved and reacted-to community pages open at this app's addresses for them. Registered once
// engagement has loaded, off the way of the first page: a list drawn before then draws again.
void import("#/features/engagement").then(({ registerSubjectCard }) => {
  for (const type of [
    common.SubjectType.COMMUNITY,
    common.SubjectType.COMMUNITY_THREAD,
    common.SubjectType.COMMUNITY_REPLY,
    common.SubjectType.COMMUNITY_EVENT,
  ])
    registerSubjectCard(type, CommunitySubjectCard);
});

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
    const off = onFramesLoading("community", () =>
      import("./frames").then(
        ({ applyCommunityFrame }) =>
          ({ event }) =>
            applyCommunityFrame(queryClient, event),
      ),
    );
    // What changed while the connection was down was not announced: the lists on screen (my
    // communities, invitations, queues) are read again. An open community's boards read
    // themselves again when its topic is back (`CommunityLayout`).
    const stop = rereadOnWelcome(
      queryClient,
      client,
      [communityKeys.all],
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
