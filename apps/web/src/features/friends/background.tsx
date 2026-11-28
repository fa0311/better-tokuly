import { useQueryClient } from "@tanstack/react-query";
import { friendKeys } from "@tokuly/core/friends/keys";
import { rereadOnWelcome } from "@tokuly/core/query";
import { useEffect } from "react";
import { onFramesLoading, realtime } from "#/lib/realtime";

/**
 * Runs while someone is signed in: the friends lists and requests follow the
 * pushes (`./frames`), and a friendship the other side ended takes that
 * person out of every feature at once, as the viewer's own unfriend or block
 * does. A new connection reads them again: what changed before it was up, or
 * while it was down, was not announced.
 */
export default function FriendsBackground() {
  const queryClient = useQueryClient();

  useEffect(() => {
    const client = realtime();
    const offWelcome = rereadOnWelcome(queryClient, client, [friendKeys.all]);
    const offSocial = onFramesLoading("social", () =>
      import("./frames").then(
        ({ applySocialEvent }) =>
          ({ event }) =>
            applySocialEvent(queryClient, event),
      ),
    );
    client.start();
    return () => {
      offWelcome();
      offSocial();
    };
  }, [queryClient]);

  return null;
}
