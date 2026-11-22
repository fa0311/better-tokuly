import { useQuery, useQueryClient } from "@tanstack/react-query";
import { social } from "@tokuly/api";
import { useEffect } from "react";
import { api } from "#/lib/api";
import { realtime } from "#/lib/realtime";
import type { BadgeSources } from "#/seams/badges";

const INCOMING = ["social", "requests", "incoming"];

/** Friend requests waiting for an answer. */
function useFriendRequestsBadge(): number {
  const queryClient = useQueryClient();
  const requests = useQuery({
    queryKey: INCOMING,
    queryFn: () =>
      api()
        .service(social.FriendService)
        .listFriendRequests({ direction: social.FriendRequestDirection.INCOMING }),
  });

  // A new request (or its answer) is announced as a notification of the social domain.
  useEffect(
    () =>
      realtime().onFrame((frame) => {
        if (frame.body.case !== "notify") return;
        const event = frame.body.value.event;
        if (event.case === "received" && event.value.notification?.kind.startsWith("social."))
          void queryClient.invalidateQueries({ queryKey: INCOMING });
      }),
    [queryClient],
  );
  return requests.data?.incomingCount ?? 0;
}

/** `friends`: incoming friend requests. */
export default { friends: useFriendRequestsBadge } satisfies BadgeSources;
