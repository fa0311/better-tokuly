import { useInfiniteQuery } from "@tanstack/react-query";
import { video } from "@tokuly/api";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { channelClient, nextCursor, videoKeys } from "../api";
import { VideoCard, VideoCardSkeleton, VideoGrid } from "../video-card";

const { ChannelVideoSort, VideoOrigin, TrafficSource } = video;

const SORTS = [
  { value: ChannelVideoSort.NEWEST, key: "channel.sort.newest" },
  { value: ChannelVideoSort.POPULAR, key: "channel.sort.popular" },
  { value: ChannelVideoSort.OLDEST, key: "channel.sort.oldest" },
] as const;
const ORIGINS = [
  { value: VideoOrigin.UNSPECIFIED, key: "channel.origin.all" },
  { value: VideoOrigin.UPLOAD, key: "channel.origin.uploads" },
  { value: VideoOrigin.LIVE_ARCHIVE, key: "channel.origin.liveArchives" },
] as const;

/** The channel's videos tab: everything the viewer may see listed, sorted and filtered. */
export function ChannelVideos({ channel }: { channel: video.Channel }) {
  const { t } = useTranslation("video");
  const handle = channel.summary?.handle ?? "";
  const [sort, setSort] = useState<video.ChannelVideoSort>(ChannelVideoSort.NEWEST);
  const [origin, setOrigin] = useState<video.VideoOrigin>(VideoOrigin.UNSPECIFIED);
  const query = useInfiniteQuery({
    queryKey: videoKeys.channelVideos(handle, sort, origin),
    queryFn: ({ pageParam }) => channelClient().listChannelVideos({ channelId: handle, sort, origin, page: { limit: 24, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });
  const videos = query.data?.pages.flatMap((page) => page.videos) ?? [];

  return (
    <div>
      <div className="scrollbar-none -mx-4 mb-4 flex gap-2 overflow-x-auto px-4">
        {SORTS.map((option) => (
          <Chip key={option.value} active={sort === option.value} onClick={() => setSort(option.value)}>
            {t(option.key)}
          </Chip>
        ))}
        <span className="mx-1 w-0.5 shrink-0 self-stretch rounded-full bg-line" aria-hidden />
        {ORIGINS.map((option) => (
          <Chip key={option.value} active={origin === option.value} onClick={() => setOrigin(option.value)}>
            {t(option.key)}
          </Chip>
        ))}
      </div>
      {query.isPending ? (
        <VideoGrid>
          {Array.from({ length: 6 }, (_, index) => (
            <VideoCardSkeleton key={index} />
          ))}
        </VideoGrid>
      ) : query.isError ? (
        <EmptyState emoji="😵‍💫" title={errorMessage(query.error)} />
      ) : videos.length === 0 ? (
        <EmptyState emoji="🌱" title={t("channel.noVideos")} />
      ) : (
        <>
          <VideoGrid>
            {videos.map((summary) => (
              <VideoCard key={summary.id} video={summary} source={TrafficSource.CHANNEL} hideChannel />
            ))}
          </VideoGrid>
          <LoadMore query={query} />
        </>
      )}
    </div>
  );
}

function Chip({ active, onClick, children }: { active: boolean; onClick(): void; children: ReactNode }) {
  return (
    <button type="button" aria-pressed={active} onClick={onClick} className={cn("h-9 shrink-0 rounded-full border-2 px-3.5 text-sm font-bold", active ? "border-ink bg-ink text-bg" : "border-line bg-surface text-ink-soft hover:text-ink")}>
      {children}
    </button>
  );
}
