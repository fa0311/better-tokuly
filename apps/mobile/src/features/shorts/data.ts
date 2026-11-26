import {
  infiniteQueryOptions,
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { create, shorts } from "@tokuly/api";
import { nextCursor } from "@tokuly/core/query";
import {
  type FeedContext,
  isSwipeOnly,
  type ListOptions,
  type ListPage,
  listKey,
  loadListPage,
  patchShortEverywhere,
  type ShortClients,
  type Stay,
  shortKeys,
} from "@tokuly/core/shorts";
import { Share } from "react-native";
import { api } from "#/lib/api";

/**
 * Shorts in the query cache. The pages of every feed and grid, the cache
 * edits and the stay meter are `@tokuly/core/shorts`; this file wires them
 * to the phone's API client.
 */
export const shortClient = () => api().service(shorts.ShortService);
export const feedClient = () => api().service(shorts.ShortFeedService);
export const discoveryClient = () => api().service(shorts.ShortDiscoveryService);

const clients: ShortClients = { shorts: shortClient, feed: feedClient, discovery: discoveryClient };

export function listQuery(context: FeedContext, options: ListOptions = {}) {
  return infiniteQueryOptions({
    queryKey: listKey(context, options),
    queryFn: ({ pageParam }): Promise<ListPage> =>
      loadListPage(clients, context, pageParam, options),
    initialPageParam: "",
    getNextPageParam: nextCursor,
    // A feed is walked through: it must not rearrange itself under the viewer.
    ...(isSwipeOnly(context) ? { staleTime: Number.POSITIVE_INFINITY } : {}),
  });
}

/** The list behind a feed context (a feed, a hashtag, a sound…), page by page. */
export function useShortList(context: FeedContext, options: ListOptions = {}) {
  return useInfiniteQuery(listQuery(context, options));
}

/** The creator's own shorts in every state, newest first; still-processing ones are looked at again. */
export function useMyShorts() {
  return useInfiniteQuery({
    queryKey: shortKeys.mine("", shorts.ShortStatus.UNSPECIFIED),
    queryFn: ({ pageParam }) =>
      shortClient().listMyShorts({ page: { cursor: pageParam, limit: 24 } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
    refetchInterval: (query) =>
      query.state.data?.pages.some((page) =>
        page.shorts.some((short) => short.status === shorts.ShortStatus.PROCESSING),
      )
        ? 5_000
        : false,
  });
}

export function useHashtag(tag: string) {
  return useQuery({
    queryKey: shortKeys.hashtag(tag),
    queryFn: async () => (await discoveryClient().getHashtag({ tag })).hashtag,
    retry: false,
  });
}

export function useSound(soundId: string) {
  return useQuery({
    queryKey: shortKeys.sound(soundId),
    queryFn: async () => (await discoveryClient().getSound({ soundId })).sound,
  });
}

export function useHashtagSuggestions(prefix: string) {
  return useQuery({
    queryKey: shortKeys.hashtagSuggestions(prefix),
    queryFn: async () => (await discoveryClient().suggestHashtags({ prefix, limit: 8 })).hashtags,
    enabled: prefix.length > 0,
    staleTime: 60_000,
  });
}

/** Records a share and opens the system share sheet with the server's link. */
export function useShareShort(short: shorts.Short): () => Promise<void> {
  const queryClient = useQueryClient();
  return async () => {
    const response = await shortClient().shareShort({
      shortId: short.id,
      target: shorts.ShareTarget.SYSTEM,
    });
    patchShortEverywhere(queryClient, short.id, (current) => ({
      ...current,
      shareCount: response.shareCount,
    }));
    await Share.share({
      url: response.url,
      message: short.caption ? `${short.caption}\n${response.url}` : response.url,
    });
  };
}

/**
 * Hands a finished stay to the server, as the account that watched (the
 * client is taken when the stay ends). A lost report is a lost view, not
 * something to bother the viewer with.
 */
export function reportStay(shortId: string, source: shorts.PlaybackSource, stay: Stay): void {
  if (stay.watchedMs <= 0 && stay.completions === 0) return;
  const report = create(shorts.PlaybackReportSchema, {
    shortId,
    source,
    watchedMs: BigInt(stay.watchedMs),
    completions: stay.completions,
  });
  feedClient()
    .reportPlayback({ reports: [report] })
    .catch(() => {});
}
