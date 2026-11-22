import {
  type InfiniteData,
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { toMillis, video } from "@tokuly/api";
import { GripVertical, ListVideo, ListX, Pencil, Play, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Sortable } from "#/components/ui/sortable";
import { EmojiText } from "#/features/emoji";
import { errorMessage } from "#/lib/errors";
import { nextCursor, playlistClient, videoKeys } from "../api";
import { formatAge } from "../format";
import { announceTrafficSource } from "../playback-session";
import { playlistTitle } from "../save-dialog";
import { ChannelName, imageUrl, VideoCard, VideoCardSkeleton, VisibilityChip } from "../video-card";
import { movedItem } from "./order";
import { PlaylistFormDialog } from "./playlist-form";

const { PlaylistKind, TrafficSource } = video;
type ItemPages = InfiniteData<video.ListPlaylistItemsResponse>;

/**
 * `/playlist/<id>`: a playlist with "play all". Its owner can rename it,
 * change who sees it, drag the videos into another order and take videos
 * out. `WL` and `LL` are the viewer's watch-later list and liked videos.
 */
export function PlaylistScreen({ playlistId }: { playlistId: string }) {
  const { t } = useTranslation("video");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [editing, setEditing] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const playlistQuery = useQuery({
    queryKey: videoKeys.playlist(playlistId),
    queryFn: async () => (await playlistClient().getPlaylist({ playlistId })).playlist,
  });
  const itemsKey = videoKeys.playlistItems(playlistId);
  const items = useInfiniteQuery({
    queryKey: itemsKey,
    queryFn: ({ pageParam }) =>
      playlistClient().listPlaylistItems({ playlistId, page: { limit: 50, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });
  const playlist = playlistQuery.data;
  const rows = items.data?.pages.flatMap((page) => page.items) ?? [];
  const videos = rows.flatMap((row) => (row.video ? [row.video] : []));

  /** Rewrites the cached list as one page holding `next` (later pages keep their cursors, empty). */
  const setRows = (next: video.PlaylistItem[]) =>
    queryClient.setQueryData<ItemPages>(itemsKey, (data) =>
      data
        ? {
            ...data,
            pages: data.pages.map((page, index) => ({ ...page, items: index === 0 ? next : [] })),
          }
        : data,
    );
  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: videoKeys.playlists });
  };

  const move = useMutation({
    mutationFn: (moved: { id: string; afterId: string }) =>
      playlistClient().movePlaylistItem({
        playlistId,
        videoId: moved.id,
        afterVideoId: moved.afterId,
      }),
    onError: (error) => {
      toast.error(errorMessage(error));
      void queryClient.invalidateQueries({ queryKey: itemsKey });
    },
  });
  const reorder = (next: video.PlaylistItem[]) => {
    const moved = movedItem(
      rows.map((row) => row.video?.id ?? ""),
      next.map((row) => row.video?.id ?? ""),
    );
    if (!moved) return;
    setRows(next);
    move.mutate(moved);
  };
  const removeItem = useMutation({
    mutationFn: (videoId: string) => playlistClient().removePlaylistItem({ playlistId, videoId }),
    onMutate: (videoId) => setRows(rows.filter((row) => row.video?.id !== videoId)),
    onSuccess: () => {
      toast.success(t("playlist.toast.itemRemoved"));
      refresh();
    },
    onError: (error) => {
      toast.error(errorMessage(error));
      void queryClient.invalidateQueries({ queryKey: itemsKey });
    },
  });
  const remove = useMutation({
    mutationFn: () => playlistClient().deletePlaylist({ playlistId: playlist?.id ?? playlistId }),
    onSuccess: () => {
      setDeleting(false);
      refresh();
      toast.success(t("playlist.toast.deleted"));
      void navigate({ to: "/watch/playlists" });
    },
    onError: (error) => {
      setDeleting(false);
      toast.error(errorMessage(error));
    },
  });

  if (playlistQuery.isPending) {
    return (
      <Page title={t("playlist.title")} back="/watch/playlists">
        <Skeleton className="h-40 rounded-card" />
        <div className="mt-5 flex flex-col gap-3">
          {[0, 1, 2].map((row) => (
            <VideoCardSkeleton key={row} layout="row" />
          ))}
        </div>
      </Page>
    );
  }
  if (playlistQuery.isError || !playlist) {
    return (
      <Page title={t("playlist.title")} back="/watch/playlists">
        <EmptyState emoji="🫥" title={t("playlist.notFound")} body={t("playlist.notFoundBody")} />
      </Page>
    );
  }

  const custom = playlist.kind === PlaylistKind.CUSTOM;
  const canReorder = playlist.isOwner && custom;
  const canRemove = playlist.isOwner && playlist.kind !== PlaylistKind.LIKED;
  const name = playlistTitle(playlist, t);
  const first = videos[0];
  const cover = imageUrl(playlist.thumbnail ?? first?.thumbnail, 320);
  const removeAction = (summary: video.VideoSummary) =>
    canRemove
      ? [
          {
            label: t("playlist.removeItem"),
            icon: <ListX />,
            onSelect: () => removeItem.mutate(summary.id),
          },
        ]
      : [];

  return (
    <Page title={name} back={playlist.isOwner ? "/watch/playlists" : "/watch"}>
      <header className="sticker flex flex-col gap-4 rounded-card bg-surface p-4 sm:flex-row">
        <div
          className="aspect-video w-full shrink-0 overflow-hidden rounded-2xl bg-surface-3 sm:w-56"
          style={
            playlist.thumbnail?.dominantColor
              ? { background: playlist.thumbnail.dominantColor }
              : undefined
          }
        >
          {cover ? (
            <img src={cover} alt="" className="size-full object-cover" />
          ) : (
            <ListVideo className="m-auto size-10 h-full text-ink-faint" />
          )}
        </div>
        <div className="flex min-w-0 flex-1 flex-col gap-2">
          <h2 className="text-xl leading-snug font-black break-words" data-testid="playlist-title">
            {name}
          </h2>
          <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm font-bold text-ink-soft">
            {playlist.channel ? (
              <Link
                to="/c/$handle"
                params={{ handle: playlist.channel.handle }}
                className="hover:text-ink"
              >
                <ChannelName channel={playlist.channel} />
              </Link>
            ) : (
              playlist.owner && (
                <Link
                  to="/u/$handle"
                  params={{ handle: playlist.owner.handle }}
                  className="hover:text-ink"
                >
                  <EmojiText text={playlist.owner.displayName} />
                </Link>
              )
            )}
            <span>{t("playlist.items", { count: playlist.itemCount })}</span>
            {custom && playlist.updatedAt && (
              <span>{t("playlist.updated", { age: formatAge(toMillis(playlist.updatedAt)) })}</span>
            )}
            <VisibilityChip visibility={playlist.visibility} />
          </p>
          {playlist.description && (
            <p className="text-sm leading-relaxed break-words whitespace-pre-wrap">
              {playlist.description}
            </p>
          )}
          {playlist.kind === PlaylistKind.LIKED && (
            <p className="text-sm text-ink-soft">{t("playlist.likedHint")}</p>
          )}
          <div className="mt-auto flex flex-wrap gap-2 pt-2">
            {first && (
              <Link
                to="/watch/$videoId"
                params={{ videoId: first.shortId || first.id }}
                search={{ list: playlist.shortId || playlistId }}
                onClick={() => announceTrafficSource(TrafficSource.PLAYLIST)}
                className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-brand px-5 font-bold text-on-brand"
              >
                <Play className="size-4 fill-current" />
                {t("playlist.playAll")}
              </Link>
            )}
            {canReorder && (
              <>
                <Button
                  variant="secondary"
                  icon={<Pencil className="size-4" />}
                  onClick={() => setEditing(true)}
                >
                  {t("actions.edit")}
                </Button>
                <Button
                  variant="ghost"
                  className="text-danger"
                  icon={<Trash2 className="size-4" />}
                  onClick={() => setDeleting(true)}
                >
                  {t("actions.delete")}
                </Button>
              </>
            )}
          </div>
        </div>
      </header>

      <div className="mt-5">
        {items.isPending ? (
          <div className="flex flex-col gap-3">
            {[0, 1, 2].map((row) => (
              <VideoCardSkeleton key={row} layout="row" />
            ))}
          </div>
        ) : items.isError ? (
          <EmptyState emoji="😵‍💫" title={errorMessage(items.error)} />
        ) : rows.length === 0 ? (
          <EmptyState
            emoji="📼"
            title={t("playlist.empty")}
            body={playlist.isOwner ? t("playlist.emptyOwnerBody") : undefined}
          />
        ) : canReorder ? (
          <>
            <p className="mb-2 px-1 text-xs font-bold text-ink-faint">
              {t("playlist.reorderHint")}
            </p>
            <Sortable
              items={rows}
              getKey={(row) => row.video?.id ?? ""}
              onReorder={reorder}
              className="flex flex-col gap-1"
              itemClassName="rounded-2xl bg-bg"
            >
              {(row, { grip, dragging }) =>
                row.video && (
                  <VideoCard
                    video={row.video}
                    layout="row"
                    source={TrafficSource.PLAYLIST}
                    playlistId={playlist.shortId || playlistId}
                    className={dragging ? "rounded-2xl bg-surface p-1.5" : "p-1.5"}
                    actions={removeAction(row.video)}
                    leading={
                      <button
                        type="button"
                        aria-label={t("playlist.move", { title: row.video.title })}
                        className="grid h-full min-h-16 w-7 shrink-0 place-items-center self-stretch rounded-lg text-ink-faint hover:bg-surface-2 hover:text-ink"
                        {...grip}
                      >
                        <GripVertical className="size-5" />
                      </button>
                    }
                  />
                )
              }
            </Sortable>
            <LoadMore query={items} />
          </>
        ) : (
          <>
            <ol className="flex flex-col gap-3">
              {videos.map((summary, index) => (
                <li key={summary.id}>
                  <VideoCard
                    video={summary}
                    layout="row"
                    source={TrafficSource.PLAYLIST}
                    playlistId={playlist.shortId || playlistId}
                    actions={removeAction(summary)}
                    leading={
                      <span className="w-5 shrink-0 self-center text-center text-xs font-bold text-ink-faint tabular-nums">
                        {index + 1}
                      </span>
                    }
                  />
                </li>
              ))}
            </ol>
            <LoadMore query={items} />
          </>
        )}
      </div>

      <PlaylistFormDialog
        open={editing}
        onOpenChange={setEditing}
        playlist={playlist}
        onSaved={(saved) => {
          queryClient.setQueryData(videoKeys.playlist(playlistId), saved);
          setEditing(false);
        }}
      />
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("playlist.confirmDelete", { name })}
        description={t("playlist.confirmDeleteBody")}
        confirmLabel={t("actions.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </Page>
  );
}
