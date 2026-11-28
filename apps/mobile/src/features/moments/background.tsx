import { useQueryClient } from "@tanstack/react-query";
import { applyMomentsEvent, momentKeys } from "@tokuly/core/moments";
import { rereadOnWelcome } from "@tokuly/core/query";
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
    // Whatever happened before the connection was up, or while it was down, was not announced.
    const offWelcome = rereadOnWelcome(queryClient, client, [momentKeys.all]);
    const off = client.onFrame((frame) => {
      if (frame.topic) return;
      if (frame.body.case === "moments") {
        applyMomentsEvent(queryClient, frame.body.value.event);
      }
    });
    client.start();
    return () => {
      off();
      offWelcome();
    };
  }, [queryClient, myId]);
}

export default function MomentsBackground() {
  useMomentsSync();
  return null;
}
