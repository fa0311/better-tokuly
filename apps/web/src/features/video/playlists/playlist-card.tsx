import { Link } from "@tanstack/react-router";
import { video } from "@tokuly/api";
import { Clock3, ListVideo, ThumbsUp } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "#/lib/cn";
import { playlistTitle } from "../save-dialog";
import { imageUrl, VisibilityChip } from "../video-card";

const { PlaylistKind } = video;

/** A playlist as a card: the first video's picture under a "stack", its name and how many videos it holds. */
export function PlaylistCard({
  playlist,
  showVisibility,
  className,
}: {
  playlist: video.Playlist /** Say who can see it (in the owner's own library). */;
  showVisibility?: boolean;
  className?: string;
}) {
  const { t } = useTranslation("video");
  const cover = imageUrl(playlist.thumbnail, 320);
  const Icon =
    playlist.kind === PlaylistKind.WATCH_LATER
      ? Clock3
      : playlist.kind === PlaylistKind.LIKED
        ? ThumbsUp
        : ListVideo;
  // Built-in lists are addressed by their fixed names.
  const id =
    playlist.kind === PlaylistKind.WATCH_LATER
      ? "WL"
      : playlist.kind === PlaylistKind.LIKED
        ? "LL"
        : playlist.shortId || playlist.id;
  return (
    <Link
      to="/playlist/$playlistId"
      params={{ playlistId: id }}
      data-testid="playlist-card"
      className={cn("group block min-w-0", className)}
    >
      <span className="relative block pt-2">
        {/* Two slivers behind the picture: this is a pile of videos, not one. */}
        <span aria-hidden className="absolute inset-x-4 top-0 h-2 rounded-t-xl bg-surface-3" />
        <span aria-hidden className="absolute inset-x-2 top-1 h-2 rounded-t-xl bg-ink-faint/50" />
        <span
          className="relative block aspect-video overflow-hidden rounded-2xl bg-surface-3"
          style={
            playlist.thumbnail?.dominantColor
              ? { background: playlist.thumbnail.dominantColor }
              : undefined
          }
        >
          {cover ? (
            <img src={cover} alt="" loading="lazy" className="size-full object-cover" />
          ) : (
            <Icon className="absolute top-1/2 left-1/2 size-9 -translate-x-1/2 -translate-y-1/2 text-ink-faint" />
          )}
          <span className="absolute right-1.5 bottom-1.5 inline-flex h-6 items-center gap-1 rounded-md bg-black/80 px-1.5 text-xs font-black text-white">
            <Icon className="size-3.5" />
            {t("playlist.items", { count: playlist.itemCount })}
          </span>
        </span>
      </span>
      <span className="mt-2 block truncate font-black group-hover:text-brand">
        {playlistTitle(playlist, t)}
      </span>
      {showVisibility && playlist.kind === PlaylistKind.CUSTOM && (
        <VisibilityChip visibility={playlist.visibility} className="mt-1" />
      )}
    </Link>
  );
}
