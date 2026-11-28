import { useQuery, useQueryClient } from "@tanstack/react-query";
import { momentKeys, momentQueries } from "@tokuly/core/moments";
import { useEffect } from "react";
import { momentClient } from "#/features/moments/data";
import { realtime } from "#/lib/realtime";

export function useMapMoments(enabled: boolean) {
  const queryClient = useQueryClient();
  const query = useQuery({
    ...momentQueries.map(momentClient),
    enabled,
  });

  useEffect(() => {
    if (!enabled) return;
    const client = realtime();
    const invalidate = () => void queryClient.invalidateQueries({ queryKey: momentKeys.map });
    const offFrame = client.onFrame((frame) => {
      if (frame.topic) return;
      const { body } = frame;
      if (body.case === "geo" && body.value.event.case === "locationRemovedUserId") {
        invalidate();
      }
    });
    client.start();
    return offFrame;
  }, [enabled, queryClient]);

  return query;
}
