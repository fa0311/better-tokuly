import { useInfiniteQuery } from "@tanstack/react-query";
import type { video } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { nextCursor, playlistClient, videoKeys } from "../api";
import { PlaylistCard } from "../playlists/playlist-card";

/** The channel's playlists tab: the public playlists published on it. */
export function ChannelPlaylists({ channel }: { channel: video.Channel }) {
  const { t } = useTranslation("video");
  const handle = channel.summary?.handle ?? "";
  const query = useInfiniteQuery({
    queryKey: videoKeys.channelPlaylists(handle),
    queryFn: ({ pageParam }) => playlistClient().listChannelPlaylists({ channelId: handle, page: { limit: 24, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });
  const playlists = query.data?.pages.flatMap((page) => page.playlists) ?? [];
  const grid = "grid grid-cols-2 gap-x-4 gap-y-6 md:grid-cols-3 lg:grid-cols-4";

  if (query.isPending) {
    return (
      <div className={grid}>
        {[0, 1, 2, 3].map((card) => (
          <Skeleton key={card} className="aspect-video" />
        ))}
      </div>
    );
  }
  if (query.isError) return <EmptyState emoji="😵‍💫" title={errorMessage(query.error)} />;
  if (playlists.length === 0) return <EmptyState emoji="📼" title={t("channel.noPlaylists")} body={channel.viewer?.isOwner ? t("channel.noPlaylistsOwnerBody") : undefined} />;
  return (
    <>
      <div className={grid}>
        {playlists.map((playlist) => (
          <PlaylistCard key={playlist.id} playlist={playlist} />
        ))}
      </div>
      <LoadMore query={query} />
    </>
  );
}
