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
    // ListMapMoments applies live visibility rules, so a pin can expire without
    // a matching removal event. Recheck well inside the server's LIVE_WINDOW.
    refetchInterval: enabled ? 60_000 : false,
  });

  useEffect(() => {
    if (!enabled) return;
    const client = realtime();
    const MIN_REFRESH_GAP = 5_000;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let lastRefreshAt = 0;
    let refreshPending = false;
    const scheduleRefresh = () => {
      refreshPending = true;
      if (timer !== undefined) return;
      const delay = Math.max(1_000, MIN_REFRESH_GAP - (Date.now() - lastRefreshAt));
      timer = setTimeout(() => {
        timer = undefined;
        if (!refreshPending) return;
        if (queryClient.getQueryState(momentKeys.map)?.fetchStatus === "fetching") {
          // Let an older response finish, then request again so this geo event
          // cannot be lost to a request that started before it.
          scheduleRefresh();
          return;
        }
        refreshPending = false;
        lastRefreshAt = Date.now();
        void queryClient.invalidateQueries({ queryKey: momentKeys.map }, { cancelRefetch: false });
      }, delay);
    };
    const offFrame = client.onFrame((frame) => {
      if (frame.topic) return;
      const { body } = frame;
      // Location state can change whether its owner's Moment is visible. The
      // interval above also catches server-side expiry when no event arrives.
      if (
        body.case === "geo" &&
        (body.value.event.case === "location" || body.value.event.case === "locationRemovedUserId")
      )
        scheduleRefresh();
    });
    client.start();
    return () => {
      offFrame();
      if (timer !== undefined) clearTimeout(timer);
    };
  }, [enabled, queryClient]);

  return query;
}
