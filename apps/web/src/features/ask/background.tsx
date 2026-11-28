import { useQueryClient } from "@tanstack/react-query";
import { useRouter } from "@tanstack/react-router";
import { toMillis } from "@tokuly/api";
import { askKeys } from "@tokuly/core/ask/keys";
import { rereadOnWelcome } from "@tokuly/core/query";
import { useEffect } from "react";
import { onFramesLoading, realtime } from "#/lib/realtime";
import { offerDraft, takePendingAsk } from "./guest-return";
import { noteServerTime } from "./server-clock";

/**
 * The question box, for as long as someone is signed in: new questions,
 * answers, flames and prompt responses pushed over the realtime connection
 * go into the query cache, so the inbox, the badges and "your question was
 * answered" are live on every page. It also finishes what a visitor started
 * on a public box page before they had an account.
 */
export default function AskBackground() {
  const queryClient = useQueryClient();
  const router = useRouter();

  useEffect(() => {
    const client = realtime();
    const offEvents = onFramesLoading("ask", () =>
      import("./frames").then(
        ({ applyAskEvent }) =>
          ({ event }) =>
            applyAskEvent(queryClient, event),
      ),
    );
    const offFrame = client.onFrame((frame) => {
      if (frame.body.case === "welcome") noteServerTime(toMillis(frame.body.value.serverTime));
      else if (frame.body.case === "pong") noteServerTime(toMillis(frame.body.value.serverTime));
    });
    // Pushes sent while there was no connection (or before this one was up) are lost: read again.
    const offWelcome = rereadOnWelcome(queryClient, client, [askKeys.all]);
    client.start();
    return () => {
      offEvents();
      offFrame();
      offWelcome();
    };
  }, [queryClient]);

  // Someone who wrote a question on `/q/<handle>` and then signed up (or in) goes back to that box.
  useEffect(() => {
    const pending = takePendingAsk();
    if (!pending) return;
    offerDraft(pending);
    void router.navigate({
      to: "/ask/u/$handle",
      params: { handle: pending.handle },
      replace: true,
    });
  }, [router]);

  return null;
}
