import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { common, live, toMillis } from "@tokuly/api";
import {
  Bell,
  BellRing,
  ChevronDown,
  ChevronLeft,
  LayoutDashboard,
  MoreHorizontal,
  Scissors,
  Swords,
} from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Coins } from "#/components/ui/coins";
import { Dialog } from "#/components/ui/dialog";
import { Menu } from "#/components/ui/menu";
import { EmptyState } from "#/components/ui/page";
import { PaneSpinner } from "#/components/ui/spinner";
import { EmojiText } from "#/features/emoji";
import { LikeButton, ShareButton, useBurst, useSubjectMenu } from "#/features/engagement";
import { type PlayerHandle, SubscribeButton } from "#/features/video";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { useHideTabBar } from "#/stores/shell";
import { giftClient, liveKeys, refreshStream, streamClient, useStream } from "./api";
import { ArchiveStage, ChatReplay } from "./archive";
import { LiveChat } from "./chat/chat-panel";
import { ClipStrip } from "./clips/clip-list";
import { categoryOf, isOpenVisibility } from "./labels";
import {
  ChannelFace,
  ChannelTitle,
  Countdown,
  formatCount,
  formatMoment,
  imageUrl,
  LiveMark,
  StreamCard,
  useNow,
  ViewerCount,
} from "./parts";
import { LivePlayer, type LivePlayerHandle } from "./player/live-player";
import { useLiveRoom, useWatchingReport } from "./room";

const { StreamStatus, ArchiveStatus } = live;

/** `/live/<id>`: a stream with its chat, whether it is scheduled, on air or over. */
export function WatchLiveScreen({ streamId }: { streamId: string }) {
  const { t } = useTranslation("live");
  const query = useStream(streamId);
  useHideTabBar();
  if (query.isPending) return <PaneSpinner />;
  if (!query.data)
    return (
      <div className="grid h-full place-items-center">
        <EmptyState
          emoji="📡"
          title={t("notFound.title")}
          body={t("notFound.body")}
          action={
            <Link
              to="/live"
              className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand"
            >
              {t("notFound.action")}
            </Link>
          }
        />
      </div>
    );
  return <StreamRoom key={query.data.summary?.id} stream={query.data} />;
}

function StreamRoom({ stream }: { stream: live.Stream }) {
  const { t } = useTranslation("live");
  const queryClient = useQueryClient();
  const summary = stream.summary;
  const streamId = summary?.id ?? "";
  const status = summary?.status ?? StreamStatus.UNSPECIFIED;
  const room = useLiveRoom(stream);
  useWatchingReport(stream, true);
  const livePlayer = useRef<LivePlayerHandle>(null);
  const archivePlayer = useRef<PlayerHandle>(null);
  const [infoOpen, setInfoOpen] = useState(false);

  // The recording becomes a video a little after the end; only the streamer is told when.
  const archivePending =
    status === StreamStatus.ENDED &&
    (stream.archiveStatus === ArchiveStatus.PENDING ||
      stream.archiveStatus === ArchiveStatus.PROCESSING);
  useEffect(() => {
    if (!archivePending) return;
    const timer = setInterval(() => refreshStream(queryClient, streamId), 6000);
    return () => clearInterval(timer);
  }, [archivePending, queryClient, streamId]);

  if (!summary) return null;
  const replay = status === StreamStatus.ENDED && Boolean(stream.archiveVideoId);

  const stage =
    status === StreamStatus.LIVE && stream.playback ? (
      <LivePlayer
        ref={livePlayer}
        playback={stream.playback}
        startedAtMs={summary.startedAt ? toMillis(summary.startedAt) : Date.now()}
        lowLatency={summary.lowLatency}
        title={summary.title}
        poster={imageUrl(summary.thumbnail, 640)}
        onSessionLost={() => refreshStream(queryClient, streamId)}
      />
    ) : status === StreamStatus.ENDED ? (
      <ArchiveStage stream={stream} player={archivePlayer} />
    ) : (
      <UpcomingStage stream={stream} />
    );

  return (
    <div className="flex h-full flex-col md:flex-row" data-testid="live-room" data-status={status}>
      <div className="shrink-0 md:min-w-0 md:flex-1 md:overflow-y-auto md:px-6 md:pt-4 md:pb-10">
        <div className="relative mx-auto max-w-5xl">
          <Link
            to="/live"
            aria-label={t("back")}
            className="absolute top-2 left-2 z-10 grid size-9 place-items-center rounded-full bg-black/55 text-white md:hidden"
          >
            <ChevronLeft className="size-6" />
          </Link>
          {stage}
          {(room.raid || stream.raidTargetStreamId) && (
            <RaidBanner
              target={room.raid?.target}
              targetId={room.raid?.target?.shortId ?? stream.raidTargetStreamId}
              automatic={room.raid !== null}
            />
          )}
          {/* Phones: one line under the picture; the rest of the height is the chat. */}
          <button
            type="button"
            onClick={() => setInfoOpen(true)}
            className="flex w-full items-center gap-2.5 border-b-2 border-line px-3 py-2 text-left md:hidden"
            aria-label={t("info.open")}
          >
            {summary.channel && <ChannelFace channel={summary.channel} />}
            <span className="min-w-0 flex-1">
              <span className="block truncate text-sm font-black">
                <EmojiText text={summary.title} />
              </span>
              <span className="flex items-center gap-2 text-xs font-bold text-ink-soft">
                <StatusLine stream={stream} />
              </span>
            </span>
            <ChevronDown className="size-5 shrink-0 text-ink-soft" />
          </button>
          <div className="hidden md:block">
            <StreamInfo stream={stream} offset={() => livePlayer.current?.offsetSeconds() ?? 0} />
          </div>
        </div>
      </div>
      {replay ? (
        <ChatReplay
          streamId={streamId}
          player={archivePlayer}
          className="min-h-0 flex-1 md:w-[23rem] md:flex-none md:border-l-2 md:border-line"
        />
      ) : (
        <LiveChat
          stream={stream}
          className="min-h-0 flex-1 md:w-[23rem] md:flex-none md:border-l-2 md:border-line"
        />
      )}
      <Dialog open={infoOpen} onOpenChange={setInfoOpen} title={t("info.title")}>
        <StreamInfo
          stream={stream}
          offset={() => livePlayer.current?.offsetSeconds() ?? 0}
          inSheet
        />
      </Dialog>
    </div>
  );
}

/** "LIVE · 12人が視聴中", "あと 3:20", or "配信終了". */
function StatusLine({ stream }: { stream: live.Stream }) {
  const { t, i18n } = useTranslation("live");
  const summary = stream.summary;
  if (!summary) return null;
  if (summary.status === StreamStatus.LIVE)
    return (
      <>
        <LiveMark className="h-5" />
        <ViewerCount count={summary.viewerCount} className="text-xs" />
      </>
    );
  if (summary.status === StreamStatus.ENDED) return <span>{t("status.ended")}</span>;
  return summary.scheduledAt ? (
    <>
      <span>{formatMoment(toMillis(summary.scheduledAt), i18n.language)}</span>
      <Countdown targetMs={toMillis(summary.scheduledAt)} />
    </>
  ) : (
    <span>{t("status.waiting")}</span>
  );
}

/** A stream that has not started: its picture, when it starts, and "tell me when it does". */
function UpcomingStage({ stream }: { stream: live.Stream }) {
  const { t, i18n } = useTranslation("live");
  const queryClient = useQueryClient();
  const summary = stream.summary;
  const streamId = summary?.id ?? "";
  const scheduledMs = summary?.scheduledAt ? toMillis(summary.scheduledAt) : undefined;
  const poster = imageUrl(summary?.thumbnail, 640);
  const [reminder, setReminder] = useState(Boolean(stream.viewer?.reminderSet));
  const remind = useMutation({
    mutationFn: (enabled: boolean) => streamClient().setReminder({ streamId, enabled }),
    onMutate: (enabled) => setReminder(enabled),
    onSuccess: ({ enabled }) => {
      setReminder(enabled);
      toast.success(t(enabled ? "upcoming.reminderOn" : "upcoming.reminderOff"));
      refreshStream(queryClient, streamId);
    },
    onError: (error, enabled) => {
      setReminder(!enabled);
      toast.error(errorMessage(error, { namespace: "live" }));
    },
  });
  return (
    <div
      className="relative grid aspect-video w-full place-items-center overflow-hidden bg-[#17122b] text-white md:rounded-2xl"
      data-testid="upcoming-stage"
    >
      {poster && (
        <img src={poster} alt="" className="absolute inset-0 size-full object-cover opacity-45" />
      )}
      <div className="relative flex flex-col items-center gap-2 p-4 text-center">
        <p className="rounded-full bg-white/15 px-3 py-1 text-xs font-black tracking-wide">
          {t("upcoming.badge")}
        </p>
        {scheduledMs !== undefined ? (
          <>
            <p className="text-3xl font-black sm:text-4xl">
              <Countdown targetMs={scheduledMs} />
            </p>
            <p className="text-sm font-bold text-white/85">
              {t("upcoming.startsAt", { time: formatMoment(scheduledMs, i18n.language) })}
            </p>
          </>
        ) : (
          <p className="text-xl font-black">{t("upcoming.unscheduled")}</p>
        )}
        {stream.viewer?.isOwner ? (
          <Link
            to="/studio/live/$streamId"
            params={{ streamId: summary?.shortId ?? "" }}
            className="pressable mt-1 inline-flex h-10 items-center gap-1.5 rounded-full bg-white px-4 text-sm font-black text-[#17122b]"
          >
            <LayoutDashboard className="size-4" />
            {t("upcoming.goToStudio")}
          </Link>
        ) : (
          <button
            type="button"
            aria-pressed={reminder}
            disabled={remind.isPending}
            onClick={() => remind.mutate(!reminder)}
            className={cn(
              "pressable mt-1 inline-flex h-10 items-center gap-1.5 rounded-full px-4 text-sm font-black",
              reminder ? "bg-lime text-[#17122b]" : "bg-white text-[#17122b]",
            )}
          >
            {reminder ? <BellRing className="size-4" /> : <Bell className="size-4" />}
            {t(reminder ? "upcoming.reminderSet" : "upcoming.remind")}
          </button>
        )}
      </div>
    </div>
  );
}

/** The end of a stream that sent its viewers on: where to, and a ride there. */
function RaidBanner({
  target,
  targetId,
  automatic,
}: {
  target: live.StreamSummary | undefined;
  targetId: string;
  /** The raid just happened in front of the viewer: they are taken along unless they stay. */
  automatic: boolean;
}) {
  const { t } = useTranslation("live");
  const navigate = useNavigate();
  const loaded = useStream(target ? "" : targetId);
  const summary = target ?? loaded.data?.summary;
  const [left, setLeft] = useState(automatic ? 8 : null);
  useEffect(() => {
    if (left === null) return;
    if (left <= 0) {
      void navigate({ to: "/live/$streamId", params: { streamId: targetId } });
      return;
    }
    const timer = setTimeout(() => setLeft(left - 1), 1000);
    return () => clearTimeout(timer);
  }, [left, navigate, targetId]);
  if (!summary) return null;
  return (
    <section
      className="mx-3 mt-3 rounded-2xl bg-brand-soft p-3 md:mx-0"
      aria-label={t("raid.title")}
      data-testid="raid-banner"
    >
      <p className="flex items-center gap-2 text-sm font-black text-brand-strong">
        <Swords className="size-4" />
        {t("raid.banner", { name: summary.channel?.name ?? "" })}
      </p>
      <StreamCard stream={summary} layout="row" className="mt-2" />
      {left !== null && (
        <div className="mt-2 flex items-center justify-between gap-2">
          <span className="text-sm font-bold text-ink-soft">
            {t("raid.moving", { count: left })}
          </span>
          <Button variant="ghost" size="sm" onClick={() => setLeft(null)}>
            {t("raid.stay")}
          </Button>
        </div>
      )}
    </section>
  );
}

/** A like that also throws hearts when someone else's like arrives. */
function LiveLike({ stream }: { stream: live.Stream }) {
  const likeCount = stream.summary?.likeCount ?? 0n;
  const { fire, burst } = useBurst(["💖", "💜", "✨", "🎉"]);
  const seen = useRef(likeCount);
  useEffect(() => {
    if (likeCount > seen.current) fire();
    seen.current = likeCount;
  });
  return (
    <span className="relative inline-flex">
      <LikeButton
        subject={{ type: common.SubjectType.LIVE_STREAM, id: stream.summary?.id ?? "" }}
      />
      {burst}
    </span>
  );
}

/** Who gave the most on this stream. */
function TopSupporters({ streamId }: { streamId: string }) {
  const { t } = useTranslation("live");
  const query = useQuery({
    queryKey: liveKeys.supporters(streamId),
    queryFn: () => giftClient().listTopSupporters({ streamId, limit: 5 }),
  });
  const supporters = query.data?.supporters ?? [];
  if (supporters.length === 0) return null;
  return (
    <section className="mt-4">
      <h3 className="text-sm font-black text-ink-soft">{t("supporters.title")}</h3>
      <ol className="mt-2 flex flex-wrap gap-2">
        {supporters.map((supporter, index) => (
          <li
            key={supporter.user?.id ?? index}
            className="flex items-center gap-2 rounded-full bg-surface-2 py-1 pr-3 pl-1"
          >
            <Avatar user={supporter.user} size="xs" />
            <span className="max-w-28 truncate text-sm font-bold">
              <EmojiText text={supporter.user?.displayName ?? t("supporters.gone")} />
            </span>
            <Coins amount={supporter.coins} size="sm" />
          </li>
        ))}
      </ol>
    </section>
  );
}

/** Everything about a stream besides its picture and its chat. */
function StreamInfo({
  stream,
  offset,
  inSheet,
}: {
  stream: live.Stream;
  /** Where the live player is on the stream's timeline, for "clip this". */
  offset(): number;
  inSheet?: boolean;
}) {
  const { t, i18n } = useTranslation("live");
  const navigate = useNavigate();
  const summary = stream.summary;
  const subject = { type: common.SubjectType.LIVE_STREAM, id: summary?.id ?? "" };
  const owner = Boolean(stream.viewer?.isOwner);
  const more = useSubjectMenu(subject, {
    title: summary?.title,
    label: summary?.title,
    report: !owner,
  });
  const now = useNow(30_000);
  if (!summary) return null;
  const channel = summary.channel;
  const category = categoryOf(summary.category);
  const onAir = summary.status === StreamStatus.LIVE;
  const canClip = onAir && stream.dvrEnabled && isOpenVisibility(summary.visibility);
  const actions = [
    ...(owner
      ? [
          {
            label: t("info.studio"),
            icon: <LayoutDashboard />,
            onSelect: () =>
              void navigate({
                to: "/studio/live/$streamId",
                params: { streamId: summary.shortId },
              }),
          },
        ]
      : []),
    ...(canClip
      ? [
          {
            label: t("clip.make"),
            icon: <Scissors />,
            onSelect: () =>
              void navigate({
                to: "/live/clips/new",
                search: { stream: summary.shortId, at: Math.floor(offset()) },
              }),
          },
        ]
      : []),
    ...more.actions,
  ];
  let meta: ReactNode = null;
  if (onAir && summary.startedAt)
    meta = t("info.startedAgo", {
      minutes: Math.max(1, Math.round((now - toMillis(summary.startedAt)) / 60_000)),
    });
  else if (summary.status === StreamStatus.ENDED && summary.endedAt)
    meta = t("info.endedAt", { time: formatMoment(toMillis(summary.endedAt), i18n.language) });
  else if (summary.scheduledAt)
    meta = t("upcoming.startsAt", {
      time: formatMoment(toMillis(summary.scheduledAt), i18n.language),
    });

  return (
    <div className={cn(!inSheet && "mt-3")}>
      <h1 className="text-xl leading-snug font-black">
        <EmojiText text={summary.title} />
      </h1>
      <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-sm font-bold text-ink-soft">
        {onAir && <LiveMark />}
        {onAir && <ViewerCount count={summary.viewerCount} />}
        {meta && <span>{meta}</span>}
        {summary.game ? (
          <span>🎮 {summary.game}</span>
        ) : (
          category && (
            <span>
              {category.emoji} {t(`category.${category.key}`)}
            </span>
          )
        )}
      </div>
      <div className="mt-3 flex flex-wrap items-center gap-2">
        {channel && (
          <Link
            to="/c/$handle"
            params={{ handle: channel.handle }}
            className="flex min-w-0 flex-1 items-center gap-2.5"
          >
            <ChannelFace channel={channel} size="md" />
            <span className="min-w-0">
              <ChannelTitle channel={channel} className="font-black" />
              <span className="block text-xs font-bold text-ink-faint">
                {t("info.subscribers", {
                  formatted: formatCount(channel.subscriberCount, i18n.language),
                })}
              </span>
            </span>
          </Link>
        )}
        {channel && !owner && (
          <SubscribeButton
            channel={channel}
            subscribed={Boolean(stream.viewer?.subscribed)}
            size="sm"
          />
        )}
      </div>
      <div className="mt-3 flex flex-wrap items-center gap-1.5">
        <LiveLike stream={stream} />
        <ShareButton subject={subject} title={summary.title} />
        <span className="flex-1" />
        {actions.length > 0 && (
          <Menu
            trigger={
              <button
                type="button"
                aria-label={t("info.more")}
                className="grid size-9 place-items-center rounded-full text-ink-soft hover:bg-surface-2 hover:text-ink"
              >
                <MoreHorizontal className="size-5" />
              </button>
            }
            actions={actions}
          />
        )}
        {more.dialogs}
      </div>
      {(stream.description || stream.tags.length > 0) && (
        <div className="mt-3 rounded-2xl bg-surface-2 p-3 text-sm leading-relaxed">
          {stream.description && (
            <p className="whitespace-pre-wrap break-words">
              <EmojiText text={stream.description} />
            </p>
          )}
          {stream.tags.length > 0 && (
            <p className="mt-1.5 flex flex-wrap gap-x-2 font-bold text-brand-strong">
              {stream.tags.map((tag) => (
                <span key={tag}>#{tag}</span>
              ))}
            </p>
          )}
        </div>
      )}
      <TopSupporters streamId={summary.id} />
      {isOpenVisibility(summary.visibility) && summary.status !== StreamStatus.UPCOMING && (
        <ClipStrip kind="stream" id={summary.id} className="mt-4" />
      )}
    </div>
  );
}
