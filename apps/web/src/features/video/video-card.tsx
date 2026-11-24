import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { common, toMillis, video } from "@tokuly/api";
import type { TFunction } from "i18next";
import { BadgeCheck, Clock3, ListPlus, MoreVertical, Radio } from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar, type AvatarProps } from "#/components/ui/avatar";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { Skeleton } from "#/components/ui/page";
import { useSubjectMenu } from "#/features/engagement";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { playlistClient, useMyChannels, videoKeys } from "./api";
import { formatAge, formatClock, formatCount } from "./format";
import { visibilityOption } from "./labels";
import { announceTrafficSource } from "./playback-session";
import { openSaveDialog } from "./save-dialog";

const { ProcessingState, VideoStatus, VideoOrigin } = video;

/** A channel shaped like the user `Avatar` draws. */
export function channelFace(channel: common.ChannelSummary): NonNullable<AvatarProps["user"]> {
  return {
    id: channel.id,
    displayName: channel.name,
    handle: channel.handle,
    avatar: channel.icon,
    deleted: false,
  };
}

export function ChannelAvatar({
  channel,
  size = "sm",
  outlined,
  className,
}: {
  channel: common.ChannelSummary;
  size?: AvatarProps["size"];
  outlined?: boolean;
  className?: string;
}) {
  return (
    <Avatar user={channelFace(channel)} size={size} outlined={outlined} className={className} />
  );
}

/** A channel's name with its verified mark. */
export function ChannelName({
  channel,
  className,
}: {
  channel: common.ChannelSummary;
  className?: string;
}) {
  const { t } = useTranslation("video");
  return (
    <span className={cn("inline-flex min-w-0 items-center gap-1", className)}>
      <span className="truncate">{channel.name}</span>
      {channel.verified && (
        <BadgeCheck
          className="size-4 shrink-0 text-brand"
          role="img"
          aria-label={t("channel.verified")}
        />
      )}
    </span>
  );
}

/** The smallest stored rendition that still covers `pixels` on a 2x screen. */
export function imageUrl(image: common.Image | undefined, pixels: number): string | undefined {
  if (!image) return undefined;
  const fitting = image.variants
    .filter((variant) => variant.size >= pixels * 2)
    .sort((a, b) => a.size - b.size)[0];
  return fitting?.url ?? image.url ?? undefined;
}

/** How long a pointer rests on a card before its moving preview starts. */
const PREVIEW_DELAY_MS = 450;

/**
 * A video's picture: the thumbnail with its length, how far the viewer got,
 * and — after hovering or pressing for a moment — the short moving preview.
 */
export function Thumbnail({
  video: summary,
  className,
  sizes = 360,
}: {
  video: video.VideoSummary;
  className?: string /** Displayed width in CSS pixels, to pick a rendition. */;
  sizes?: number;
}) {
  const { t } = useTranslation("video");
  const [previewing, setPreviewing] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);

  const url = imageUrl(summary.thumbnail, sizes);
  const canPreview = Boolean(summary.previewUrl) && !summary.ageRestricted;
  const start = () => {
    if (!canPreview) return;
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setPreviewing(true), PREVIEW_DELAY_MS);
  };
  const stop = () => {
    clearTimeout(timer.current);
    setPreviewing(false);
  };
  const processing = summary.processing === ProcessingState.PROCESSING;
  const failed = summary.processing === ProcessingState.FAILED;
  const progress = summary.watched
    ? 1
    : summary.durationSeconds > 0
      ? Math.min(1, summary.resumePositionSeconds / summary.durationSeconds)
      : 0;

  return (
    <span
      className={cn(
        "relative block aspect-video overflow-hidden rounded-2xl bg-surface-3",
        className,
      )}
      style={
        summary.thumbnail?.dominantColor
          ? { background: summary.thumbnail.dominantColor }
          : undefined
      }
      onPointerEnter={(event) => event.pointerType === "mouse" && start()}
      onPointerLeave={stop}
      onPointerDown={(event) => event.pointerType !== "mouse" && start()}
      onPointerUp={(event) => event.pointerType !== "mouse" && stop()}
      onPointerCancel={stop}
    >
      {url ? (
        <img
          src={url}
          alt=""
          loading="lazy"
          decoding="async"
          draggable={false}
          className="absolute inset-0 size-full object-cover"
        />
      ) : (
        <span className="absolute inset-0 grid place-items-center text-xs font-bold text-ink-soft">
          {processing ? t("card.processing") : failed ? t("card.failed") : ""}
        </span>
      )}
      {previewing && summary.previewUrl && (
        <img
          src={summary.previewUrl}
          alt=""
          draggable={false}
          className="absolute inset-0 size-full animate-fade-in object-cover"
        />
      )}
      {summary.origin === VideoOrigin.LIVE_ARCHIVE && (
        <span className="absolute top-1.5 left-1.5 inline-flex h-5 items-center gap-1 rounded-md bg-black/75 px-1.5 text-[11px] font-black text-white">
          <Radio className="size-3" />
          {t("card.liveArchive")}
        </span>
      )}
      {summary.ageRestricted && (
        <span className="absolute top-1.5 right-1.5 rounded-md bg-danger px-1.5 text-[11px] leading-5 font-black text-white">
          18+
        </span>
      )}
      {summary.durationSeconds > 0 && (
        <span className="absolute right-1.5 bottom-1.5 rounded-md bg-black/80 px-1.5 text-xs leading-5 font-black text-white tabular-nums">
          {summary.watched ? t("card.watched") : formatClock(summary.durationSeconds)}
        </span>
      )}
      {progress > 0 && (
        <span className="absolute inset-x-0 bottom-0 h-1 bg-white/40" aria-hidden>
          <span className="block h-full bg-brand" style={{ width: `${progress * 100}%` }} />
        </span>
      )}
    </span>
  );
}

/** "1.2万回視聴・3日前" — or what state an unpublished video is in. */
export function videoMeta(summary: video.VideoSummary, t: TFunction<"video">): string {
  if (summary.status === VideoStatus.SCHEDULED && summary.scheduledAt)
    return t("card.scheduled", {
      date: new Date(toMillis(summary.scheduledAt)).toLocaleString(undefined, {
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      }),
    });
  if (summary.status === VideoStatus.DRAFT) return t("card.draft");
  const views = t("card.views", {
    count: Number(summary.viewCount),
    formatted: formatCount(summary.viewCount),
  });
  return summary.publishedAt ? `${views}・${formatAge(toMillis(summary.publishedAt))}` : views;
}

export interface VideoCardProps {
  video: video.VideoSummary;
  /**
   * `grid` (default): picture on top, for feeds. `row`: picture on the left,
   * for side lists, playlists and search results.
   */
  layout?: "grid" | "row";
  /** Where this card sits, for the creator's traffic analytics. */
  source?: video.TrafficSource;
  /** Play it as part of this playlist (`?list=`). */
  playlistId?: string;
  /** Leave out the channel line (on the channel's own page). */
  hideChannel?: boolean;
  /** More entries for the card's menu, after "watch later" and "save to playlist". */
  actions?: MenuAction[];
  /** Something to put before the picture in `row` layout (a position, a drag handle). */
  leading?: ReactNode;
  className?: string;
}

/**
 * A video as a card: picture, title, channel, views and age, leading to the
 * watch page. Every card carries "watch later" and "save to playlist".
 */
export function VideoCard({
  video: summary,
  layout = "grid",
  source,
  playlistId,
  hideChannel,
  actions = [],
  leading,
  className,
}: VideoCardProps) {
  const { t } = useTranslation("video");
  const queryClient = useQueryClient();
  const channel = summary.channel;

  const watchLater = useMutation({
    mutationFn: () => playlistClient().addPlaylistItem({ playlistId: "WL", videoId: summary.id }),
    onSuccess: () => {
      toast.success(t("save.addedWatchLater"));
      void queryClient.invalidateQueries({ queryKey: videoKeys.playlists });
    },
    onError: (error) => toast.error(errorMessage(error)),
  });
  // One's own video cannot be reported (the server refuses with CANNOT_REPORT_OWN).
  const myChannels = useMyChannels().data?.channels ?? [];
  const mine = myChannels.some((own) => own.summary?.id === channel?.id);
  const more = useSubjectMenu(
    { type: common.SubjectType.VIDEO, id: summary.id },
    { title: summary.title, label: summary.title, report: !mine },
  );
  const menu: MenuAction[] = [
    { label: t("save.watchLater"), icon: <Clock3 />, onSelect: () => watchLater.mutate() },
    { label: t("save.toPlaylist"), icon: <ListPlus />, onSelect: () => openSaveDialog(summary) },
    ...actions,
    // Send in chat, copy the link, report.
    ...more.actions,
  ];

  const link = {
    to: "/watch/$videoId",
    params: { videoId: summary.shortId || summary.id },
    search: playlistId ? { list: playlistId } : {},
  } as const;
  const follow = () => source !== undefined && announceTrafficSource(source);
  const row = layout === "row";

  return (
    <article
      data-testid="video-card"
      className={cn(
        "group relative flex min-w-0",
        row ? "items-start gap-3" : "flex-col gap-2.5",
        className,
      )}
    >
      {leading}
      <Link
        {...link}
        onClick={follow}
        tabIndex={-1}
        aria-hidden
        className={cn("shrink-0", row ? "w-40 sm:w-44" : "w-full")}
      >
        <Thumbnail video={summary} sizes={row ? 176 : 360} />
      </Link>
      <div className="flex min-w-0 flex-1 gap-2.5">
        {!row && channel && !hideChannel && (
          <Link
            to="/c/$handle"
            params={{ handle: channel.handle }}
            className="shrink-0 self-start"
            aria-label={channel.name}
          >
            <ChannelAvatar channel={channel} />
          </Link>
        )}
        <div className="min-w-0 flex-1">
          <h3
            className={cn(
              "line-clamp-2 leading-snug font-black break-words",
              row ? "text-sm" : "text-[15px]",
            )}
          >
            <Link {...link} onClick={follow} className="hover:text-brand">
              {summary.title}
            </Link>
          </h3>
          {channel && !hideChannel && (
            <Link
              to="/c/$handle"
              params={{ handle: channel.handle }}
              className="mt-0.5 flex max-w-full text-xs font-bold text-ink-soft hover:text-ink sm:text-sm"
            >
              <ChannelName channel={channel} />
            </Link>
          )}
          <p className="truncate text-xs text-ink-soft sm:text-sm">{videoMeta(summary, t)}</p>
        </div>
        <Menu
          trigger={
            <button
              type="button"
              aria-label={t("card.menu", { title: summary.title })}
              className="-mr-1.5 grid size-8 shrink-0 place-items-center self-start rounded-full text-ink-soft hover:bg-surface-2 hover:text-ink"
            >
              <MoreVertical className="size-4" />
            </button>
          }
          actions={menu}
        />
      </div>
      {more.dialogs}
    </article>
  );
}

/** Placeholder with the shape of a `VideoCard` while a feed loads. */
export function VideoCardSkeleton({ layout = "grid" }: { layout?: "grid" | "row" }) {
  if (layout === "row") {
    return (
      <div className="flex gap-3" aria-hidden>
        <Skeleton className="aspect-video w-40 shrink-0 sm:w-44" />
        <div className="flex-1 py-1">
          <Skeleton className="h-4 w-full" />
          <Skeleton className="mt-2 h-3 w-2/3" />
        </div>
      </div>
    );
  }
  return (
    <div aria-hidden>
      <Skeleton className="aspect-video w-full" />
      <div className="mt-2.5 flex gap-2.5">
        <Skeleton className="size-8 shrink-0 rounded-[0.6rem]" />
        <div className="flex-1">
          <Skeleton className="h-4 w-full" />
          <Skeleton className="mt-2 h-3 w-1/2" />
        </div>
      </div>
    </div>
  );
}

/** The responsive grid every feed of cards uses: one column on phones, up to four on wide screens. */
export function VideoGrid({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={cn(
        "grid grid-cols-1 gap-x-4 gap-y-6 sm:grid-cols-2 lg:grid-cols-3 2xl:grid-cols-4",
        className,
      )}
    >
      {children}
    </div>
  );
}

/** A chip saying who can see a video. An audience this client does not know reads as private. */
export function VisibilityChip({
  visibility,
  className,
}: {
  visibility: common.Visibility;
  className?: string;
}) {
  const { t } = useTranslation("video");
  const { key } = visibilityOption(visibility);
  return (
    <span
      className={cn(
        "inline-flex h-6 items-center rounded-full bg-surface-2 px-2.5 text-xs font-black text-ink-soft",
        className,
      )}
    >
      {t(`visibility.${key}`)}
    </span>
  );
}
