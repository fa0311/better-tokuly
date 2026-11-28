import { useQueryClient } from "@tanstack/react-query";
import { applyAskEvent, askKeys } from "@tokuly/core/ask";
import { rereadOnWelcome } from "@tokuly/core/query";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";

/**
 * Runs while someone is signed in: questions, answers, flames and prompt
 * responses pushed over the realtime connection go into the query cache, so
 * the inbox, the badge and "your question was answered" are live everywhere.
 */
export default function AskBackground() {
  const queryClient = useQueryClient();
  useEffect(() => {
    const client = realtime();
    const off = client.onFrame((frame) => {
      if (frame.body.case === "ask") applyAskEvent(queryClient, frame.body.value.event);
    });
    // Pushes sent before the connection was up, or while it was down, are lost: read again.
    const offWelcome = rereadOnWelcome(queryClient, client, [askKeys.all]);
    client.start();
    return () => {
      off();
      offWelcome();
    };
  }, [queryClient]);
  return null;
}
