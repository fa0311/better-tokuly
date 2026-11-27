import { useInfiniteQuery } from "@tanstack/react-query";
import { live, type video } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState } from "#/components/ui/page";
import type { ChannelTab } from "#/features/video";
import { liveKeys, nextCursor, streamClient } from "./api";
import { ClipStrip } from "./clips/clip-list";
import { StreamCard, StreamCardSkeleton, StreamGrid } from "./parts";

const { StreamStatus } = live;

/** A channel's streams: on air and scheduled first, then the ones that are over; and the clips cut from it. */
function ChannelLive({ channel }: { channel: video.Channel }) {
  const { t } = useTranslation("live");
  const channelId = channel.summary?.id ?? "";
  const query = useInfiniteQuery({
    queryKey: liveKeys.channelStreams(channelId, StreamStatus.UNSPECIFIED),
    queryFn: ({ pageParam }) =>
      streamClient().listChannelStreams({ channelId, page: { cursor: pageParam, limit: 24 } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
    enabled: channelId !== "",
  });
  const streams = query.data?.pages.flatMap((page) => page.streams) ?? [];
  const rank = (stream: live.StreamSummary) =>
    stream.status === StreamStatus.LIVE ? 0 : stream.status === StreamStatus.UPCOMING ? 1 : 2;
  // The server's order (newest first) holds inside each group.
  const ordered = [...streams].sort((a, b) => rank(a) - rank(b));
  return (
    <div data-testid="channel-live">
      {query.isPending ? (
        <StreamGrid>
          <StreamCardSkeleton />
          <StreamCardSkeleton />
        </StreamGrid>
      ) : ordered.length === 0 ? (
        <EmptyState emoji="📡" title={t("channel.emptyTitle")} body={t("channel.emptyBody")} />
      ) : (
        <StreamGrid>
          {ordered.map((stream) => (
            <StreamCard key={stream.id} stream={stream} />
          ))}
        </StreamGrid>
      )}
      <LoadMore query={query} />
      <ClipStrip kind="channel" id={channelId} className="mt-8" />
    </div>
  );
}

export default {
  id: "live",
  order: 30,
  label: "live:channel.tab",
  Component: ChannelLive,
} satisfies ChannelTab;
