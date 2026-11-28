import { useQueryClient } from "@tanstack/react-query";
import { applyPlansEvent, planKeys } from "@tokuly/core/plans";
import { rereadOnWelcome } from "@tokuly/core/query";
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
    const off = client.onFrame((frame) => {
      if (frame.body.case === "plans") applyPlansEvent(queryClient, frame.body.value.event);
    });
    // What changed before the connection was up, or while it was down, was not announced.
    const offWelcome = rereadOnWelcome(queryClient, client, [planKeys.all]);
    client.start();
    return () => {
      off();
      offWelcome();
    };
  }, [queryClient]);

  return null;
}
