import { useQuery } from "@tanstack/react-query";
import { shorts } from "@tokuly/api";
import { shortKeys } from "@tokuly/core/shorts";
import { api } from "#/lib/api";

export const shortClient = () => api().service(shorts.ShortService);
export const feedClient = () => api().service(shorts.ShortFeedService);
export const discoveryClient = () => api().service(shorts.ShortDiscoveryService);
export const analyticsClient = () => api().service(shorts.ShortAnalyticsService);

// The shorts keys live in `@tokuly/core/shorts`; the folder reads them from here.
export { shortKeys };

/** One short as the viewer may see it. `shortId` is the UUID or the 11-character ID. */
export function useShort(shortId: string | undefined) {
  return useQuery({
    queryKey: shortKeys.short(shortId ?? ""),
    queryFn: async () => (await shortClient().getShort({ shortId })).short!,
    enabled: Boolean(shortId),
    retry: false,
  });
}

export { nextCursor } from "@tokuly/core/query";

// The cache edits of shorts live in `@tokuly/core/shorts` too.
export { patchShortEverywhere, patchViewer, removeShortEverywhere } from "@tokuly/core/shorts";
