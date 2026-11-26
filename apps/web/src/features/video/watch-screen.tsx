import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { common, toMillis, video } from "@tokuly/api";
import {
  blockKind,
  startPosition,
  validateWatchSearch,
  type WatchSearch,
  withRating,
} from "@tokuly/core/video";
import {
  BarChart3,
  Clock3,
  ListPlus,
  PartyPopper,
  Pencil,
  ThumbsDown,
  ThumbsUp,
} from "lucide-react";
import { type ReactNode, type RefObject, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { CommentThread } from "#/features/comments";
import { SubjectMoreButton } from "#/features/engagement";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { feedClient, playlistClient, useVideo, videoClient, videoKeys } from "./api";
import { audienceNote, Description } from "./description";
import { formatCount } from "./format";
import { visibilityOption } from "./labels";
import { StartPartyDialog } from "./party/start-party";
import { takeTrafficSource, usePlaybackSession } from "./playback-session";
import { type PlayerHandle, VideoPlayer } from "./player/player";
import { PlaylistQueue, usePlaylistQueue } from "./playlists/queue";
import { openSaveDialog } from "./save-dialog";
import { SubscribeButton } from "./subscribe-button";
import { ChannelAvatar, ChannelName, imageUrl, VideoCard, VideoCardSkeleton } from "./video-card";

const { Rating, VideoStatus, TrafficSource } = video;

export { validateWatchSearch, type WatchSearch };

const THEATRE_KEY = "tokuly.player.theatre";

/** The player's position, read once a second: enough to mark the chapter that is playing. */
function useCurrentTime(player: RefObject<PlayerHandle | null>): number {
  const [time, setTime] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => setTime(Math.floor(player.current?.currentTime() ?? 0)), 1000);
    return () => clearInterval(timer);
  }, [player]);
  return time;
}

/**
 * `/watch/<id>`: the player, what the video is and who made it, the things
 * to do with it (like, subscribe, save, watch together), what to watch next,
 * and its comments.
 */
export function WatchScreen({ videoId, search }: { videoId: string; search: WatchSearch }) {
  const { t } = useTranslation("video");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const player = useRef<PlayerHandle>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const query = useVideo(videoId);
  const item = query.data;
  const summary = item?.summary;
  const [theatre, setTheatre] = useState(
    () => typeof window !== "undefined" && window.localStorage.getItem(THEATRE_KEY) === "1",
  );
  const [partyOpen, setPartyOpen] = useState(false);
  const [source] = useState(takeTrafficSource);
  const currentTime = useCurrentTime(player);

  const related = useQuery({
    queryKey: videoKeys.related(videoId),
    queryFn: () => feedClient().listRelatedVideos({ videoId, limit: 20 }),
    enabled: Boolean(summary),
  });
  const queue = usePlaylistQueue(search.list, summary?.id);
  const next = queue.next ?? related.data?.videos[0];

  const title = summary?.title;
  useEffect(() => {
    if (!title) return;
    const previous = document.title;
    document.title = `${title} - Tokuly`;
    return () => {
      document.title = previous;
    };
  }, [title]);

  const playback = usePlaybackSession({
    videoId: item?.playback ? summary?.id : undefined,
    player,
    source: search.list ? TrafficSource.PLAYLIST : source,
    onViewCount: (viewCount) =>
      queryClient.setQueryData<video.Video>(videoKeys.video(videoId), (current) =>
        current?.summary && current.summary.viewCount !== viewCount
          ? { ...current, summary: { ...current.summary, viewCount } }
          : current,
      ),
  });

  const seekTo = (seconds: number) => {
    player.current?.seek(seconds);
    void player.current?.play().catch(() => {});
    scroller.current?.scrollTo({ top: 0, behavior: "smooth" });
  };
  const changeTheatre = (value: boolean) => {
    setTheatre(value);
    window.localStorage.setItem(THEATRE_KEY, value ? "1" : "0");
  };
  const openNext = () => {
    if (!next) return;
    void navigate({
      to: "/watch/$videoId",
      params: { videoId: next.shortId || next.id },
      search: queue.next && search.list ? { list: search.list } : {},
    });
  };

  if (query.isPending) return <WatchSkeleton />;
  if (query.isError || !item || !summary) {
    return (
      <div className="grid h-full place-items-center">
        <EmptyState
          emoji="🫥"
          title={t("watch.notFound")}
          body={t("watch.notFoundBody")}
          action={
            <Link
              to="/watch"
              className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand"
            >
              {t("watch.backToFeeds")}
            </Link>
          }
        />
      </div>
    );
  }

  const channel = summary.channel;
  const isOwner = item.viewer?.isOwner ?? false;
  const note = audienceNote(summary.visibility, item.allowLink, t);

  return (
    <div ref={scroller} className="h-full overflow-y-auto pb-32 md:pb-10" data-testid="watch-page">
      {/* The side column spans both rows; whatever height it adds goes to the second row, never under the player. */}
      <div className="mx-auto grid max-w-[1500px] grid-cols-1 lg:grid-cols-[minmax(0,1fr)_23rem] lg:grid-rows-[auto_1fr] lg:gap-x-6 lg:px-6 lg:pt-4">
        <div
          className={cn(
            "portrait:max-lg:sticky portrait:max-lg:top-0 portrait:max-lg:z-20",
            theatre && "lg:col-span-2 lg:-mx-6 lg:-mt-4",
          )}
        >
          {item.playback ? (
            <VideoPlayer
              key={summary.id}
              ref={player}
              source={item.playback}
              title={summary.title}
              chapters={item.chapters}
              subtitles={item.subtitles}
              startAt={startPosition(search.t, summary)}
              autoPlay
              shortcuts
              theatre={theatre}
              onTheatreChange={changeTheatre}
              className={cn("lg:rounded-card", theatre && "lg:max-h-[78dvh] lg:rounded-none")}
              endScreen={
                next ? (
                  <UpNext key={next.id} next={next} onGo={openNext} onReplay={() => seekTo(0)} />
                ) : undefined
              }
              {...playback}
            />
          ) : (
            <Blocked item={item} />
          )}
        </div>

        <div className="min-w-0 px-4 pt-4 lg:col-start-1 lg:px-0">
          {isOwner && <OwnerNotice item={item} />}
          <h1
            className="text-xl leading-snug font-black break-words md:text-2xl"
            data-testid="video-title"
          >
            {summary.title}
          </h1>
          {(note || summary.ageRestricted) && (
            <p className="mt-1.5 flex flex-wrap items-center gap-2 text-sm font-bold text-ink-soft">
              {summary.ageRestricted && (
                <span className="rounded-md bg-danger px-1.5 text-xs leading-5 font-black text-white">
                  18+
                </span>
              )}
              {note}
            </p>
          )}

          <div className="mt-3 flex flex-wrap items-center gap-x-4 gap-y-3">
            {channel && (
              <div className="flex min-w-0 items-center gap-3">
                <Link
                  to="/c/$handle"
                  params={{ handle: channel.handle }}
                  className="shrink-0"
                  aria-label={channel.name}
                >
                  <ChannelAvatar channel={channel} size="md" outlined />
                </Link>
                <Link to="/c/$handle" params={{ handle: channel.handle }} className="min-w-0">
                  <ChannelName channel={channel} className="max-w-full font-black" />
                  <span className="block truncate text-sm text-ink-soft">
                    {t("channel.subscribers", {
                      count: Number(channel.subscriberCount),
                      formatted: formatCount(channel.subscriberCount),
                    })}
                  </span>
                </Link>
                {isOwner ? (
                  <Link
                    to="/studio/videos/$videoId"
                    params={{ videoId: summary.shortId }}
                    className="pressable sticker inline-flex h-11 shrink-0 items-center gap-2 rounded-full bg-surface px-5 font-bold"
                  >
                    <Pencil className="size-4" />
                    {t("watch.edit")}
                  </Link>
                ) : (
                  <SubscribeButton
                    channel={channel}
                    subscribed={item.viewer?.subscribed ?? false}
                    notificationLevel={item.viewer?.notificationLevel}
                    sourceVideoId={summary.id}
                    className="shrink-0"
                  />
                )}
              </div>
            )}
            <span className="flex-1" />
            <Actions item={item} videoId={videoId} onParty={() => setPartyOpen(true)} />
          </div>

          <div className="mt-4">
            <Description video={item} currentTime={currentTime} onSeek={seekTo} />
          </div>

          <div className="mt-6">
            <CommentThread
              subject={{ type: common.SubjectType.VIDEO, id: summary.id }}
              subjectAuthor={channel ? ({ channel } as common.Author) : undefined}
              focusCommentId={search.comment}
              video={
                item.playback
                  ? { currentTime: () => player.current?.currentTime() ?? 0, seekTo }
                  : undefined
              }
              defaultChannelId={isOwner ? channel?.id : undefined}
            />
          </div>
        </div>

        <aside
          className={cn(
            "min-w-0 px-4 pt-6 lg:col-start-2 lg:px-0 lg:pt-0",
            theatre ? "lg:row-start-2 lg:pt-4" : "lg:row-span-2 lg:row-start-1",
          )}
          aria-label={t("watch.upNext")}
        >
          {search.list && (
            <PlaylistQueue playlistId={search.list} currentId={summary.id} queue={queue} />
          )}
          <h2 className="mb-3 text-lg font-black">{t("watch.related")}</h2>
          {related.isPending ? (
            <div className="flex flex-col gap-3">
              {[0, 1, 2, 3].map((row) => (
                <VideoCardSkeleton key={row} layout="row" />
              ))}
            </div>
          ) : related.data && related.data.videos.length > 0 ? (
            <div className="flex flex-col gap-3">
              {related.data.videos.map((entry) => (
                <VideoCard
                  key={entry.id}
                  video={entry}
                  layout="row"
                  source={TrafficSource.RELATED}
                />
              ))}
            </div>
          ) : (
            <p className="rounded-2xl bg-surface-2 px-4 py-6 text-center text-sm font-bold text-ink-soft">
              {t("watch.noRelated")}
            </p>
          )}
        </aside>
      </div>

      <StartPartyDialog
        open={partyOpen}
        onOpenChange={setPartyOpen}
        video={summary}
        playlistId={search.list}
        position={() => player.current?.currentTime() ?? 0}
      />
    </div>
  );
}

function WatchSkeleton() {
  return (
    <div
      className="mx-auto grid max-w-[1500px] grid-cols-1 lg:grid-cols-[minmax(0,1fr)_23rem] lg:gap-x-6 lg:px-6 lg:pt-4"
      aria-busy
    >
      <div>
        <Skeleton className="aspect-video w-full rounded-none lg:rounded-card" />
        <div className="px-4 pt-4 lg:px-0">
          <Skeleton className="h-7 w-3/4" />
          <Skeleton className="mt-4 h-11 w-64" />
          <Skeleton className="mt-4 h-28" />
        </div>
      </div>
      <div className="hidden flex-col gap-3 lg:flex">
        {[0, 1, 2, 3].map((row) => (
          <VideoCardSkeleton key={row} layout="row" />
        ))}
      </div>
    </div>
  );
}

/** What stands in for the player when the video may not be played: age restriction, or an upload that is not ready. */
function Blocked({ item }: { item: video.Video }) {
  const { t } = useTranslation("video");
  const summary = item.summary;
  const poster = imageUrl(summary?.thumbnail, 640);
  const kind = blockKind(item);
  const [title, body] = [t(`watch.blocked.${kind}Title`), t(`watch.blocked.${kind}Body`)];
  return (
    <div
      data-testid="video-blocked"
      className="relative grid aspect-video w-full place-items-center overflow-hidden bg-black text-center text-white lg:rounded-card"
    >
      {poster && (
        <img
          src={poster}
          alt=""
          className="absolute inset-0 size-full scale-110 object-cover opacity-40 blur-xl"
        />
      )}
      <div className="relative max-w-md p-6">
        <p className="text-4xl" aria-hidden>
          {kind === "age" ? "🔞" : kind === "failed" ? "😢" : "⏳"}
        </p>
        <p className="mt-3 text-lg font-black">{title}</p>
        <p className="mt-1 text-sm leading-relaxed text-white/80">{body}</p>
      </div>
    </div>
  );
}

/** What only the owner is told above the title: the video is a draft, scheduled, or still on its way. */
function OwnerNotice({ item }: { item: video.Video }) {
  const { t, i18n } = useTranslation("video");
  const summary = item.summary;
  if (!summary) return null;
  const scheduled =
    summary.status === VideoStatus.SCHEDULED && summary.scheduledAt
      ? new Date(toMillis(summary.scheduledAt))
      : undefined;
  const text =
    summary.status === VideoStatus.DRAFT
      ? t("watch.owner.draft")
      : scheduled
        ? t("watch.owner.scheduled", {
            date: scheduled.toLocaleString(i18n.language, {
              month: "long",
              day: "numeric",
              hour: "2-digit",
              minute: "2-digit",
            }),
          })
        : visibilityOption(summary.visibility).value === common.Visibility.PRIVATE
          ? t("watch.owner.private")
          : "";
  if (!text) return null;
  return (
    <p
      data-testid="owner-notice"
      className="sticker-sm mb-3 flex flex-wrap items-center gap-x-3 gap-y-1 rounded-2xl bg-sun px-4 py-2.5 text-sm font-bold text-[#17122b]"
    >
      <span className="min-w-0 flex-1">{text}</span>
      <Link
        to="/studio/videos/$videoId"
        params={{ videoId: summary.shortId }}
        className="font-black underline underline-offset-2"
      >
        {t("watch.owner.openStudio")}
      </Link>
    </p>
  );
}

/** Like / dislike, watch later, save, watch together — and, for the owner, the way to the numbers. */
function Actions({
  item,
  videoId,
  onParty,
}: {
  item: video.Video;
  videoId: string;
  onParty(): void;
}) {
  const { t } = useTranslation("video");
  const queryClient = useQueryClient();
  const summary = item.summary;
  const rating = item.viewer?.rating ?? Rating.NONE;
  const key = videoKeys.video(videoId);
  const patch = (change: (current: video.Video) => video.Video) =>
    queryClient.setQueryData<video.Video>(key, (current) => (current ? change(current) : current));

  const rate = useMutation({
    mutationFn: (next: video.Rating) =>
      videoClient().rateVideo({ videoId: summary?.id ?? videoId, rating: next }),
    // The thumb answers at once; the server's count follows.
    onMutate: (next) => {
      const previous = queryClient.getQueryData<video.Video>(key);
      patch((current) => withRating(current, next));
      return { previous };
    },
    onSuccess: (response) => {
      patch((current) => ({
        ...current,
        viewer: current.viewer && { ...current.viewer, rating: response.rating },
        summary: current.summary && { ...current.summary, likeCount: response.likeCount },
      }));
      void queryClient.invalidateQueries({ queryKey: videoKeys.playlistItems("LL") });
    },
    onError: (error, _next, context) => {
      queryClient.setQueryData(key, context?.previous);
      toast.error(errorMessage(error));
    },
  });
  const inWatchLater = item.viewer?.inWatchLater ?? false;
  const watchLater = useMutation({
    mutationFn: async (add: boolean) => {
      if (add)
        await playlistClient().addPlaylistItem({
          playlistId: "WL",
          videoId: summary?.id ?? videoId,
        });
      else
        await playlistClient().removePlaylistItem({
          playlistId: "WL",
          videoId: summary?.id ?? videoId,
        });
    },
    onMutate: (add) =>
      patch((current) => ({
        ...current,
        viewer: current.viewer && { ...current.viewer, inWatchLater: add },
      })),
    onSuccess: (_result, add) => {
      toast.success(add ? t("save.addedWatchLater") : t("save.removedWatchLater"));
      void queryClient.invalidateQueries({ queryKey: videoKeys.playlists });
    },
    onError: (error, add) => {
      patch((current) => ({
        ...current,
        viewer: current.viewer && { ...current.viewer, inWatchLater: !add },
      }));
      toast.error(errorMessage(error));
    },
  });

  if (!summary) return null;
  const published = summary.status === VideoStatus.PUBLISHED;
  const liked = rating === Rating.LIKE;
  const disliked = rating === Rating.DISLIKE;

  return (
    <div className="scrollbar-none -mx-4 flex max-w-[calc(100%+2rem)] items-center gap-2 overflow-x-auto px-4 py-1 sm:mx-0 sm:max-w-full sm:px-0">
      <div className="sticker-sm flex h-11 shrink-0 items-stretch overflow-hidden rounded-full bg-surface">
        <button
          type="button"
          aria-pressed={liked}
          aria-label={t("watch.like")}
          disabled={!published}
          className={cn(
            "flex items-center gap-1.5 pr-3 pl-4 font-bold hover:bg-surface-2 disabled:opacity-50",
            liked && "bg-lime text-[#17122b] hover:bg-lime",
          )}
          onClick={() => rate.mutate(liked ? Rating.NONE : Rating.LIKE)}
        >
          <ThumbsUp className={cn("size-5", liked && "fill-current")} />
          <span className="tabular-nums" data-testid="like-count">
            {formatCount(summary.likeCount)}
          </span>
        </button>
        <span className="w-0.5 bg-ink" aria-hidden />
        <button
          type="button"
          aria-pressed={disliked}
          aria-label={t("watch.dislike")}
          disabled={!published}
          className={cn(
            "grid place-items-center pr-4 pl-3 hover:bg-surface-2 disabled:opacity-50",
            disliked && "bg-surface-3",
          )}
          onClick={() => rate.mutate(disliked ? Rating.NONE : Rating.DISLIKE)}
        >
          <ThumbsDown className={cn("size-5", disliked && "fill-current")} />
        </button>
      </div>
      <ActionButton
        icon={<Clock3 className={cn("size-4", inWatchLater && "text-brand")} />}
        pressed={inWatchLater}
        onClick={() => watchLater.mutate(!inWatchLater)}
      >
        {inWatchLater ? t("save.inWatchLater") : t("save.watchLater")}
      </ActionButton>
      <ActionButton icon={<ListPlus className="size-4" />} onClick={() => openSaveDialog(summary)}>
        {t("save.action")}
      </ActionButton>
      {item.playback && published && (
        <ActionButton icon={<PartyPopper className="size-4" />} onClick={onParty}>
          {t("party.start")}
        </ActionButton>
      )}
      {published && (
        <SubjectMoreButton
          subject={{ type: common.SubjectType.VIDEO, id: summary.id }}
          title={summary.title}
          label={summary.title}
          report={!item.viewer?.isOwner}
        />
      )}
      {item.viewer?.isOwner && (
        <Link
          to="/studio/videos/$videoId"
          params={{ videoId: summary.shortId }}
          search={{ tab: "analytics" }}
          className="pressable sticker-sm inline-flex h-11 shrink-0 items-center gap-2 rounded-full bg-surface px-4 text-sm font-bold"
        >
          <BarChart3 className="size-4" />
          {t("watch.analytics")}
        </Link>
      )}
    </div>
  );
}

function ActionButton({
  icon,
  pressed,
  onClick,
  children,
}: {
  icon: ReactNode;
  pressed?: boolean;
  onClick(): void;
  children: ReactNode;
}) {
  return (
    <Button
      variant="secondary"
      className="sticker-sm shrink-0 px-4 text-sm"
      aria-pressed={pressed}
      icon={icon}
      onClick={onClick}
    >
      {children}
    </Button>
  );
}

const UP_NEXT_SECONDS = 8;

/** Over the picture once a video has ended: the next one, starting by itself after a moment unless cancelled. */
function UpNext({
  next,
  onGo,
  onReplay,
}: {
  next: video.VideoSummary;
  onGo(): void;
  onReplay(): void;
}) {
  const { t } = useTranslation("video");
  const [remaining, setRemaining] = useState<number | null>(UP_NEXT_SECONDS);
  const go = useRef(onGo);
  useEffect(() => {
    go.current = onGo;
  });
  useEffect(() => {
    if (remaining === null) return;
    if (remaining <= 0) {
      go.current();
      return;
    }
    const timer = setTimeout(
      () => setRemaining((value) => (value === null ? null : value - 1)),
      1000,
    );
    return () => clearTimeout(timer);
  }, [remaining]);

  const thumbnail = imageUrl(next.thumbnail, 240);
  return (
    <div
      className="flex w-full max-w-sm flex-col items-center gap-3 text-center"
      data-testid="up-next"
    >
      <p className="text-sm font-bold text-white/80">
        {remaining === null ? t("watch.upNext") : t("watch.upNextIn", { seconds: remaining })}
      </p>
      <button
        type="button"
        className="flex w-full items-center gap-3 rounded-2xl bg-white/10 p-2 text-left hover:bg-white/20"
        onClick={onGo}
      >
        {thumbnail && (
          <img
            src={thumbnail}
            alt=""
            className="aspect-video w-28 shrink-0 rounded-xl object-cover"
          />
        )}
        <span className="min-w-0 flex-1">
          <span className="line-clamp-2 text-sm leading-snug font-black">{next.title}</span>
          <span className="block truncate text-xs text-white/70">{next.channel?.name}</span>
        </span>
      </button>
      <div className="flex gap-2">
        {remaining !== null && (
          <button
            type="button"
            className="h-10 rounded-full bg-white/15 px-4 text-sm font-bold hover:bg-white/25"
            onClick={() => setRemaining(null)}
          >
            {t("actions.cancel")}
          </button>
        )}
        <button
          type="button"
          className="h-10 rounded-full bg-white/15 px-4 text-sm font-bold hover:bg-white/25"
          onClick={onReplay}
        >
          {t("player.replay")}
        </button>
        <button
          type="button"
          className="h-10 rounded-full bg-white px-4 text-sm font-black text-[#17122b]"
          onClick={onGo}
        >
          {t("watch.playNext")}
        </button>
      </div>
    </div>
  );
}
