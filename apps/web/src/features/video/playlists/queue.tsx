import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { video } from "@tokuly/api";
import { nextInQueue, playlistTitle } from "@tokuly/core/video";
import { ListVideo, Play } from "lucide-react";
import { useEffect, useRef } from "react";
import { LoadMore } from "#/components/ui/load-more";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { nextCursor, playlistClient, videoKeys } from "../api";
import { announceTrafficSource } from "../playback-session";
import { Thumbnail } from "../video-card";

export interface PlaylistQueueState {
  playlist?: video.Playlist;
  items: video.VideoSummary[];
  /** The video after the one playing; undefined at the end of the list (or before it is loaded). */
  next?: video.VideoSummary;
  query: ReturnType<typeof usePlaylistItems>;
}

function usePlaylistItems(playlistId: string | undefined) {
  return useInfiniteQuery({
    queryKey: videoKeys.playlistItems(playlistId ?? ""),
    queryFn: ({ pageParam }) =>
      playlistClient().listPlaylistItems({
        playlistId: playlistId ?? "",
        page: { limit: 50, cursor: pageParam },
      }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
    enabled: Boolean(playlistId),
  });
}

/** The playlist a video is being played from (`?list=`): its videos and which one comes next. */
export function usePlaylistQueue(
  playlistId: string | undefined,
  currentId: string | undefined,
): PlaylistQueueState {
  const query = usePlaylistItems(playlistId);
  const playlist = useQuery({
    queryKey: videoKeys.playlist(playlistId ?? ""),
    queryFn: async () =>
      (await playlistClient().getPlaylist({ playlistId: playlistId ?? "" })).playlist,
    enabled: Boolean(playlistId),
  });
  const items =
    query.data?.pages.flatMap((page) =>
      page.items.flatMap((item) => (item.video ? [item.video] : [])),
    ) ?? [];
  return { playlist: playlist.data, items, next: nextInQueue(items, currentId), query };
}

/** The list being played, next to the player: where the viewer is in it and a tap to jump anywhere. */
export function PlaylistQueue({
  playlistId,
  currentId,
  queue,
}: {
  playlistId: string;
  currentId: string;
  queue: PlaylistQueueState;
}) {
  const { t } = useTranslation("video");
  const current = useRef<HTMLAnchorElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const index = queue.items.findIndex((item) => item.id === currentId);

  // Keep the playing entry in view inside the list, without moving the page.
  const loadedCount = queue.items.length;
  // re-centre when the playing video or the loaded list changes.
  useEffect(() => {
    const row = current.current;
    const box = list.current;
    if (row && box) box.scrollTop = row.offsetTop - box.clientHeight / 2 + row.clientHeight / 2;
  }, [currentId, loadedCount]);

  if (!queue.playlist && queue.items.length === 0) return null;
  return (
    <section
      data-testid="playlist-queue"
      className="sticker mb-6 overflow-hidden rounded-card bg-surface"
      aria-label={t("playlist.queue")}
    >
      <header className="flex items-center gap-3 border-b-2 border-line px-4 py-3">
        <ListVideo className="size-5 shrink-0 text-brand" />
        <div className="min-w-0 flex-1">
          <Link
            to="/playlist/$playlistId"
            params={{ playlistId }}
            className="block truncate font-black hover:text-brand"
          >
            {queue.playlist ? playlistTitle(queue.playlist, t) : t("playlist.queue")}
          </Link>
          <p className="text-xs font-bold text-ink-soft tabular-nums">
            {index >= 0
              ? `${index + 1} / ${queue.playlist?.itemCount ?? queue.items.length}`
              : t("playlist.items", { count: queue.playlist?.itemCount ?? queue.items.length })}
          </p>
        </div>
      </header>
      <div ref={list} className="relative max-h-80 overflow-y-auto p-1.5">
        {queue.items.map((item, position) => {
          const playing = item.id === currentId;
          return (
            <Link
              key={item.id}
              ref={playing ? current : undefined}
              to="/watch/$videoId"
              params={{ videoId: item.shortId || item.id }}
              search={{ list: playlistId }}
              aria-current={playing ? "true" : undefined}
              onClick={() => announceTrafficSource(video.TrafficSource.PLAYLIST)}
              className={cn(
                "flex items-center gap-2.5 rounded-xl p-1.5",
                playing ? "bg-brand-soft" : "hover:bg-surface-2",
              )}
            >
              <span className="grid w-5 shrink-0 place-items-center text-xs font-bold text-ink-faint tabular-nums">
                {playing ? <Play className="size-3.5 fill-current text-brand" /> : position + 1}
              </span>
              <Thumbnail video={item} sizes={96} className="w-24 shrink-0 rounded-lg" />
              <span className="min-w-0 flex-1">
                <span className="line-clamp-2 text-sm leading-snug font-bold">{item.title}</span>
                <span className="block truncate text-xs text-ink-soft">{item.channel?.name}</span>
              </span>
            </Link>
          );
        })}
        <LoadMore query={queue.query} />
      </div>
    </section>
  );
}
