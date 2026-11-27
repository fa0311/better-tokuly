import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { live, social, type video, wallet } from "@tokuly/api";
import { liveKeys, liveQueries } from "@tokuly/core/live";
import { nextCursor } from "@tokuly/core/query";
import { walletQueries } from "@tokuly/core/wallet";
import { Share } from "react-native";
import { api } from "#/lib/api";
import { webLink } from "#/lib/config";

/**
 * Live streams in the query cache. The keys, query options and cache edits
 * are `@tokuly/core/live`; this file wires them to the phone's API client.
 * Every client is `api()` when called: whoever is signed in now. A request
 * that goes out after an await takes its client before the first one.
 */
export const streamClient = () => api().service(live.LiveStreamService);
export const chatClient = () => api().service(live.LiveChatService);
export const pollClient = () => api().service(live.LivePollService);
export const giftClient = () => api().service(live.LiveGiftService);
const friendClient = () => api().service(social.FriendService);
const walletClient = () => api().service(wallet.WalletService);

/** A stream page as the viewer sees it; `null` when it is gone or not theirs to see. */
export function useStream(streamId: string) {
  return useQuery({ ...liveQueries.stream(streamClient, streamId), enabled: streamId !== "" });
}

/** The gift ladder (twelve colours by coin amount). */
export function useGiftTiers() {
  return useQuery(liveQueries.tiers(giftClient));
}

/** Whom the viewer blocked: their pushed lines stay out of every chat. */
export function useBlockedIds() {
  return useQuery(liveQueries.blocked(friendClient));
}

/** Friends' streams (friends-only ones included): on air first, then the scheduled ones. */
export function useFriendStreams() {
  return useQuery({
    queryKey: liveKeys.friends,
    queryFn: async () => (await streamClient().listFriendStreams({ page: { limit: 20 } })).streams,
    staleTime: 30_000,
  });
}

/** The channels the viewer subscribes to that are on air. */
export function useSubscribedStreams() {
  return useQuery({
    queryKey: liveKeys.subscribed,
    queryFn: async () =>
      (await streamClient().listSubscribedStreams({ page: { limit: 20 } })).streams,
    staleTime: 30_000,
  });
}

/** Public streams on air, most watched first, narrowed to a genre. */
export function useLiveNow(category: video.VideoCategory) {
  return useInfiniteQuery({
    queryKey: liveKeys.liveNow(category, ""),
    queryFn: ({ pageParam }) =>
      streamClient().listLiveStreams({
        category,
        game: "",
        page: { cursor: pageParam, limit: 20 },
      }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });
}

/** Public streams that are scheduled, the soonest first. */
export function useUpcoming(category: video.VideoCategory) {
  return useInfiniteQuery({
    queryKey: liveKeys.upcoming(category),
    queryFn: ({ pageParam }) =>
      streamClient().listUpcomingStreams({ category, page: { cursor: pageParam, limit: 20 } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });
}

/** Who gave the most on a stream. */
export function useTopSupporters(streamId: string) {
  return useQuery({
    queryKey: liveKeys.supporters(streamId),
    queryFn: async () => (await giftClient().listTopSupporters({ streamId, limit: 5 })).supporters,
  });
}

/**
 * The viewer's spendable coins: the wallet's own balance in the cache
 * (`walletKeys.balance`), so a bonus claimed in the wallet or a push shows
 * here too. Read again after every gift.
 */
export function useCoins() {
  const queryClient = useQueryClient();
  return useQuery({
    ...walletQueries.balance(walletClient, queryClient),
    select: (balance) => balance.coins,
    staleTime: 10_000,
  });
}

/** Opens the system share sheet with the stream's link. */
export async function shareStream(summary: live.StreamSummary): Promise<void> {
  const url = webLink(`/live/${summary.shortId}`);
  await Share.share({ url, message: `${summary.title}\n${url}` });
}
