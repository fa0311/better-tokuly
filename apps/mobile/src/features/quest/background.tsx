import { useQueryClient } from "@tanstack/react-query";
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
    let welcomed = false;
    return realtime().onFrame((frame) => {
      if (frame.body.case === "quest") applyQuestEvent(queryClient, frame.body.value);
      else if (frame.body.case === "welcome") {
        if (welcomed) void queryClient.invalidateQueries({ queryKey: questKeys.all });
        welcomed = true;
      }
    });
  }, [queryClient]);
  return null;
}
