import { useQueryClient } from "@tanstack/react-query";
import { applyCommunityEvent, communityKeys } from "@tokuly/core/community";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";
import { boardClient } from "./data";

/**
 * Keeps what is loaded about communities current. Events of a community's
 * topic arrive while its screen is open (it follows the topic); the
 * caller's own standing and the staff's queues arrive always. After a gap
 * in the connection the lists are read again: nothing announced what
 * changed in between.
 */
export default function CommunityBackground() {
  const queryClient = useQueryClient();
  useEffect(() => {
    let welcomed = false;
    return realtime().onFrame((frame) => {
      if (frame.body.case === "community")
        applyCommunityEvent(queryClient, frame.body.value.event, boardClient);
      else if (frame.body.case === "welcome") {
        if (welcomed) void queryClient.invalidateQueries({ queryKey: communityKeys.my });
        welcomed = true;
      }
    });
  }, [queryClient]);
  return null;
}
