import { useQueryClient } from "@tanstack/react-query";
import { useRouter } from "@tanstack/react-router";
import { toMillis } from "@tokuly/api";
import { applyAskEvent, askKeys } from "@tokuly/core/ask";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";
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
    const offFrame = client.onFrame((frame) => {
      if (frame.body.case === "ask") applyAskEvent(queryClient, frame.body.value.event);
      else if (frame.body.case === "welcome") {
        noteServerTime(toMillis(frame.body.value.serverTime));
        // Pushes sent while there was no connection (or before this one was up) are lost: read again what is on screen.
        void queryClient.invalidateQueries({ queryKey: askKeys.all });
      } else if (frame.body.case === "pong") noteServerTime(toMillis(frame.body.value.serverTime));
    });
    client.start();
    return offFrame;
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
