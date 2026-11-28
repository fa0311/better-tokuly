import { useQueryClient } from "@tanstack/react-query";
import { applyCommunityEvent, communityKeys } from "@tokuly/core/community";
import { rereadOnWelcome } from "@tokuly/core/query";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";
import { boardClient } from "./data";

/**
 * Keeps what is loaded about communities current. Events of a community's
 * topic arrive while its screen is open (it follows the topic); the
 * caller's own standing and the staff's queues arrive always. After a gap
 * in the connection the lists on screen are read again: nothing announced
 * what changed in between. An open community reads itself again when its
 * topic is back (`useCommunityTopic`).
 */
export default function CommunityBackground() {
  const queryClient = useQueryClient();
  useEffect(() => {
    const client = realtime();
    const off = client.onFrame((frame) => {
      if (frame.body.case === "community")
        applyCommunityEvent(queryClient, frame.body.value.event, boardClient);
    });
    const stop = rereadOnWelcome(
      queryClient,
      client,
      [communityKeys.all],
      (key) => key[1] === "in" && client.isSubscribed(`community:${String(key[2])}`),
    );
    return () => {
      off();
      stop();
    };
  }, [queryClient]);
  return null;
}
