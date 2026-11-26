import { useQueryClient } from "@tanstack/react-query";
import { applyPlansEvent, planKeys } from "@tokuly/core/plans";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";

/**
 * Runs while someone is signed in: keeps loaded plans current from the
 * realtime connection (invitations, changes, answers, a plan called off).
 */
export default function PlansBackground() {
  const queryClient = useQueryClient();

  useEffect(() => {
    const client = realtime();
    let welcomed = false;
    const off = client.onFrame((frame) => {
      if (frame.body.case === "welcome") {
        // What changed while the connection was down was not announced.
        if (welcomed) void queryClient.invalidateQueries({ queryKey: planKeys.all });
        welcomed = true;
        return;
      }
      if (frame.body.case === "plans") applyPlansEvent(queryClient, frame.body.value.event);
    });
    client.start();
    return off;
  }, [queryClient]);

  return null;
}
