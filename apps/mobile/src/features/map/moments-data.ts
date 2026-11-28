import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { momentClient } from "#/features/moments/data";
import { realtime } from "#/lib/realtime";

export const momentMapKey = ["moments", "map"] as const;

export function useMapMoments(enabled: boolean) {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: momentMapKey,
    queryFn: () => momentClient().listMapMoments({}),
    enabled,
    staleTime: 0,
  });

  useEffect(() => {
    if (!enabled) return;
    const client = realtime();
    const invalidate = () => void queryClient.invalidateQueries({ queryKey: momentMapKey });
    const offFrame = client.onFrame((frame) => {
      if (frame.topic) return;
      const { body } = frame;
      if (body.case === "moments") {
        const event = body.value.event;
        if (event.case === "friendPosted" || event.case === "deleted") invalidate();
      } else if (body.case === "geo" && body.value.event.case === "locationRemovedUserId") {
        invalidate();
      } else if (body.case === "social" && body.value.event.case === "friendRemovedUserId") {
        invalidate();
      }
    });
    client.start();
    return offFrame;
  }, [enabled, queryClient]);

  return query;
}
