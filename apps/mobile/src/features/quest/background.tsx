import { useQueryClient } from "@tanstack/react-query";
import { rereadOnWelcome } from "@tokuly/core/query";
import { applyQuestEvent, questKeys } from "@tokuly/core/quest";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";

/**
 * Keeps what is loaded about quests current: XP, levels, missions and
 * badges move with the pushed `QuestEvent`s. After a gap in the connection
 * everything is read again (the coins with it): nothing announced what
 * changed in between.
 */
export default function QuestBackground() {
  const queryClient = useQueryClient();
  useEffect(() => {
    const client = realtime();
    const off = client.onFrame((frame) => {
      if (frame.body.case === "quest") applyQuestEvent(queryClient, frame.body.value);
    });
    // Pushes sent before the connection was up, or while it was down, are lost: read again.
    const offWelcome = rereadOnWelcome(queryClient, client, [questKeys.all]);
    return () => {
      off();
      offWelcome();
    };
  }, [queryClient]);
  return null;
}
