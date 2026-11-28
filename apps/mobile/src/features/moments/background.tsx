import { useQueryClient } from "@tanstack/react-query";
import { applyMomentsEvent, forgetUserMoments, momentKeys } from "@tokuly/core/moments";
import { useEffect } from "react";
import { useMe } from "#/features/session/store";
import { realtime } from "#/lib/realtime";

/** Keeps the Moments cache current while the signed-in account is connected. */
export function useMomentsSync(): void {
  const queryClient = useQueryClient();
  const myId = useMe()?.user?.id ?? "";

  useEffect(() => {
    if (!myId) return;
    const client = realtime();
    let connectedBefore = false;
    const off = client.onFrame((frame) => {
      if (frame.topic) return;
      if (frame.body.case === "welcome") {
        if (connectedBefore) void queryClient.invalidateQueries({ queryKey: momentKeys.all });
        connectedBefore = true;
        return;
      }
      if (frame.body.case === "social" && frame.body.value.event.case === "friendRemovedUserId") {
        forgetUserMoments(queryClient, frame.body.value.event.value);
        return;
      }
      if (frame.body.case === "moments") {
        applyMomentsEvent(queryClient, frame.body.value.event);
      }
    });
    client.start();
    return off;
  }, [queryClient, myId]);
}

export default function MomentsBackground() {
  useMomentsSync();
  return null;
}
