import { useQuery, useQueryClient } from "@tanstack/react-query";
import { social } from "@tokuly/api";
import { friendKeys, friendQueries } from "@tokuly/core/friends";
import { useEffect } from "react";
import { api } from "#/lib/api";
import { realtime } from "#/lib/realtime";
import type { BadgeSource } from "#/seams/badges";

const friendClient = () => api().service(social.FriendService);

import { refreshFollows, useFollowRequestCount } from "./follow-data";

/** The social pushes about follows: a request came or went, one was approved, a new follower. */
const FOLLOW_EVENTS = new Set([
  "followRequestReceived",
  "followRequestCancelledId",
  "followRequestApproved",
  "newFollower",
]);

/** Friend requests and follow requests waiting for an answer. */
function useFriendRequestsBadge(): number {
  const queryClient = useQueryClient();
  const requests = useQuery(friendQueries.incoming(friendClient));
  const followRequests = useFollowRequestCount();

  useEffect(
    () =>
      realtime().onFrame((frame) => {
        // A new friend request (or its answer) is announced as a notification of the social domain.
        if (frame.body.case === "notify") {
          const event = frame.body.value.event;
          if (event.case === "received" && event.value.notification?.kind.startsWith("social."))
            void queryClient.invalidateQueries({ queryKey: friendKeys.incoming });
          return;
        }
        // Follows have their own pushes: counts, requests and relations re-read.
        if (frame.body.case === "social" && FOLLOW_EVENTS.has(frame.body.value.event.case ?? ""))
          void refreshFollows(queryClient);
      }),
    [queryClient],
  );
  return (requests.data?.incomingCount ?? 0) + followRequests;
}

export default useFriendRequestsBadge satisfies BadgeSource;
