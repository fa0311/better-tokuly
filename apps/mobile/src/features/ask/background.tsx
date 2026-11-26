import { useQueryClient } from "@tanstack/react-query";
import { applyAskEvent, askKeys } from "@tokuly/core/ask";
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
    let welcomed = false;
    const off = client.onFrame((frame) => {
      if (frame.body.case === "ask") applyAskEvent(queryClient, frame.body.value.event);
      else if (frame.body.case === "welcome") {
        // Pushes sent while the connection was down are lost: what is loaded is read again.
        if (welcomed) void queryClient.invalidateQueries({ queryKey: askKeys.all });
        welcomed = true;
      }
    });
    client.start();
    return off;
  }, [queryClient]);
  return null;
}
