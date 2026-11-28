import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { live, toMillis } from "@tokuly/api";
import {
  BarChart3,
  ChevronLeft,
  ExternalLink,
  Pencil,
  Play,
  Plus,
  Square,
  Swords,
  Trash2,
  Wifi,
  WifiOff,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { EmptyState } from "#/components/ui/page";
import { PaneSpinner } from "#/components/ui/spinner";
import { Tabs } from "#/components/ui/tabs";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { useHideTabBar } from "#/stores/shell";
import {
  analyticsClient,
  liveKeys,
  pollClient,
  refreshStream,
  storeStream,
  streamClient,
  useStream,
} from "../api";
import { LiveChat } from "../chat/chat-panel";
import { ViewerList } from "../chat/viewers";
import { ReceivedGifts } from "../gifts/history-screen";
import { END_REASON_KEYS } from "../labels";
import { formatCount, imageUrl, StreamPicture, useNow } from "../parts";
import { clock, LivePlayer } from "../player/live-player";
import { CreatePollDialog, PollCard } from "../polls";
import { useLiveRoom } from "../room";
import { ViewerChart } from "./analytics";
import { BrowserPublisher } from "./browser-publisher";
import { IngestPanel } from "./ingest-panel";
import { Section, Stat, StatusChip } from "./shared";

const { StreamStatus } = live;

type Pane = "stream" | "chat" | "viewers" | "polls" | "gifts";
type Method = "browser" | "encoder";

/** `/studio/live/<id>`: the control room of one stream of the caller. */
export function ControlRoomScreen({ streamId }: { streamId: string }) {
  const { t } = useTranslation("live");
  const query = useStream(streamId);
  useHideTabBar();
  if (query.isPending) return <PaneSpinner />;
  const stream = query.data;
  if (!stream?.viewer?.isOwner)
    return (
      <div className="grid h-full place-items-center">
        <EmptyState
          emoji="🎛️"
          title={t("studio.notYours")}
          action={
            <Link
              to="/studio/live"
              className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand"
            >
              {t("studio.backToStudio")}
            </Link>
          }
        />
      </div>
    );
  return <ControlRoom key={stream.summary?.id} stream={stream} />;
}

function ControlRoom({ stream }: { stream: live.Stream }) {
  const { t } = useTranslation("live");
  const summary = stream.summary;
  useLiveRoom(stream);
  const [pane, setPane] = useState<Pane>("stream");
  if (!summary) return null;
  const status = summary.status;
  const ended = status === StreamStatus.ENDED;
  const panes: { value: Pane; label: string }[] = [
    { value: "stream", label: t("control.panes.stream") },
    { value: "chat", label: t("control.panes.chat") },
    { value: "viewers", label: t("control.panes.viewers") },
    { value: "polls", label: t("control.panes.polls") },
    { value: "gifts", label: t("control.panes.gifts") },
  ];
  // Wide screens keep the broadcast and the chat side by side; the other panes share the right column.
  const side: Pane = pane === "stream" ? "chat" : pane;
  const sideBody = (which: Pane): ReactNode =>
    which === "chat" ? (
      <LiveChat stream={stream} header={false} className="h-full" />
    ) : (
      <div className="h-full overflow-y-auto p-4">
        {which === "viewers" ? (
          ended ? (
            <EmptyState emoji="👀" title={t("viewersList.endedTitle")} />
          ) : (
            <ViewerList stream={stream} />
          )
        ) : which === "polls" ? (
          <PollsPane stream={stream} />
        ) : (
          <ReceivedGifts channelId={summary.channel?.id ?? ""} streamId={summary.id} />
        )}
      </div>
    );

  return (
    <div className="flex h-full flex-col" data-testid="control-room" data-status={status}>
      <header className="glass z-20 flex h-14 shrink-0 items-center gap-2 border-b-2 border-line px-3">
        <Link
          to="/studio/live"
          aria-label={t("studio.backToStudio")}
          className="grid size-9 shrink-0 place-items-center rounded-full hover:bg-surface-2"
        >
          <ChevronLeft className="size-6" />
        </Link>
        <h1 className="min-w-0 flex-1 truncate text-lg font-black">
          <EmojiText text={summary.title} />
        </h1>
        <StatusChip status={status} encoderConnected={stream.studio?.encoderConnected} />
      </header>
      <div className="shrink-0 border-b-2 border-line px-3 py-2 md:hidden">
        <Tabs<Pane> items={panes} value={pane} onChange={setPane} />
      </div>
      <div className="flex min-h-0 flex-1">
        {/* The broadcast column stays mounted while other panes show: leaving it must not stop the camera. */}
        <div
          className={cn(
            "min-w-0 flex-1 overflow-y-auto px-4 pt-4 pb-10 md:block",
            pane !== "stream" && "hidden",
          )}
        >
          <div className="mx-auto flex max-w-3xl flex-col gap-4">
            <BroadcastColumn stream={stream} />
          </div>
        </div>
        <div
          className={cn(
            "min-h-0 flex-1 flex-col md:flex md:w-[24rem] md:flex-none md:border-l-2 md:border-line",
            pane === "stream" ? "hidden" : "flex",
          )}
        >
          <div className="hidden shrink-0 border-b-2 border-line px-3 py-2 md:block">
            <Tabs<Pane>
              items={panes.filter((item) => item.value !== "stream")}
              value={side}
              onChange={setPane}
            />
          </div>
          <div className="min-h-0 flex-1">{sideBody(side)}</div>
        </div>
      </div>
    </div>
  );
}

function BroadcastColumn({ stream }: { stream: live.Stream }) {
  const { t, i18n } = useTranslation("live");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const summary = stream.summary;
  const streamId = summary?.id ?? "";
  const status = summary?.status ?? StreamStatus.UNSPECIFIED;
  const ended = status === StreamStatus.ENDED;
  const onAir = status === StreamStatus.LIVE;
  const connected = Boolean(stream.studio?.encoderConnected);
  const [method, setMethod] = useState<Method>("browser");
  const [publishing, setPublishing] = useState(false);
  const [ending, setEnding] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [raiding, setRaiding] = useState(false);

  const start = useMutation({
    mutationFn: () => streamClient().startStream({ streamId }),
    onSuccess: (response) => {
      storeStream(queryClient, response.stream);
      toast.success(t("control.started"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "live" })),
  });
  const end = useMutation({
    mutationFn: () => streamClient().endStream({ streamId }),
    onSuccess: (response) => {
      storeStream(queryClient, response.stream);
      setEnding(false);
      toast.success(t(onAir ? "control.ended" : "control.cancelled"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "live" })),
  });
  const remove = useMutation({
    mutationFn: () => streamClient().deleteStream({ streamId }),
    onSuccess: () => {
      queryClient.removeQueries({ queryKey: liveKeys.streams });
      void queryClient.invalidateQueries({ queryKey: liveKeys.lists });
      toast.success(t("control.deleted"));
      void navigate({ to: "/studio/live" });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "live" })),
  });

  if (!summary) return null;
  const endReason = stream.studio ? END_REASON_KEYS[stream.studio.endReason] : undefined;
  return (
    <>
      <Section
        title={t(ended ? "control.overTitle" : onAir ? "control.onAirTitle" : "control.readyTitle")}
        lead={
          ended
            ? endReason && t(`endReason.${endReason}`)
            : onAir
              ? t("control.onAirLead")
              : connected
                ? t(stream.studio?.autoStart ? "control.startingLead" : "control.previewLead")
                : t("control.readyLead")
        }
        actions={
          <Link
            to="/live/$streamId"
            params={{ streamId: summary.shortId }}
            aria-label={t("control.openPage")}
            title={t("control.openPage")}
            className="grid size-9 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-2 hover:text-ink"
          >
            <ExternalLink className="size-5" />
          </Link>
        }
      >
        {/* The streamer's own monitor: what viewers get, silent so it does not echo. */}
        {!ended && stream.playback && connected && !publishing && (
          <LivePlayer
            playback={stream.playback}
            startedAtMs={summary.startedAt ? toMillis(summary.startedAt) : Date.now()}
            lowLatency
            muted
            title={summary.title}
            poster={imageUrl(summary.thumbnail, 640)}
            onSessionLost={() => refreshStream(queryClient, streamId)}
          />
        )}
        {!ended && (
          <p
            className={cn(
              "flex items-center gap-2 rounded-2xl px-3 py-2 text-sm font-bold",
              connected ? "bg-lime text-[#17122b]" : "bg-surface-2 text-ink-soft",
            )}
            data-testid="encoder-state"
            data-connected={connected}
          >
            {connected ? <Wifi className="size-4" /> : <WifiOff className="size-4" />}
            {connected
              ? t("control.encoderConnected", {
                  protocol: (stream.studio?.encoderProtocol || "").toUpperCase(),
                })
              : t("control.encoderWaiting")}
          </p>
        )}
        {summary.scheduledAt && status === StreamStatus.UPCOMING && (
          <p className="text-sm font-bold text-ink-soft">
            {t("upcoming.startsAt", {
              time: new Date(toMillis(summary.scheduledAt)).toLocaleString(i18n.language, {
                dateStyle: "medium",
                timeStyle: "short",
              }),
            })}
          </p>
        )}
        <div className="flex flex-wrap gap-2">
          {status === StreamStatus.UPCOMING && connected && !stream.studio?.autoStart && (
            <Button
              loading={start.isPending}
              icon={<Play className="size-4 fill-current" />}
              onClick={() => start.mutate()}
            >
              {t("control.goLive")}
            </Button>
          )}
          {onAir && (
            <>
              <Button
                variant="danger"
                icon={<Square className="size-4 fill-current" />}
                onClick={() => setEnding(true)}
              >
                {t("control.end")}
              </Button>
              <Button
                variant="secondary"
                icon={<Swords className="size-4" />}
                onClick={() => setRaiding(true)}
              >
                {t("raid.action")}
              </Button>
            </>
          )}
          {status === StreamStatus.UPCOMING && (
            <Button variant="secondary" onClick={() => setEnding(true)}>
              {t("control.cancel")}
            </Button>
          )}
          <Link
            to="/studio/live/$streamId/edit"
            params={{ streamId: summary.shortId }}
            className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-surface px-5 text-[15px] font-bold"
          >
            <Pencil className="size-4" />
            {t("control.edit")}
          </Link>
          {ended && (
            <>
              <Link
                to="/studio/live/$streamId/analytics"
                params={{ streamId: summary.shortId }}
                className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-surface px-5 text-[15px] font-bold"
              >
                <BarChart3 className="size-4" />
                {t("analytics.open")}
              </Link>
              <Button
                variant="ghost"
                icon={<Trash2 className="size-4" />}
                className="text-danger"
                onClick={() => setDeleting(true)}
              >
                {t("control.delete")}
              </Button>
            </>
          )}
        </div>
        {ended && stream.studio?.archiveError && (
          <p
            className="rounded-2xl bg-surface-2 px-3 py-2 text-sm font-bold text-danger"
            role="alert"
          >
            {t("archive.failed")}
          </p>
        )}
      </Section>

      {(onAir || (connected && !ended)) && <Health streamId={streamId} />}

      {!ended && (
        <Section title={t("control.methodTitle")} lead={t("control.methodLead")}>
          <Tabs
            items={[
              { value: "browser", label: t("control.method.browser") },
              { value: "encoder", label: t("control.method.encoder") },
            ]}
            value={method}
            onChange={(next) => {
              if (publishing) return toast.message(t("publish.stopFirst"));
              setMethod(next);
            }}
          />
          {method === "browser" ? (
            <BrowserPublisher streamId={streamId} onPublishingChange={setPublishing} />
          ) : (
            <IngestPanel streamId={streamId} />
          )}
        </Section>
      )}

      <ConfirmDialog
        open={ending}
        onOpenChange={setEnding}
        title={t(onAir ? "control.endTitle" : "control.cancelTitle")}
        description={t(
          onAir
            ? stream.autoArchive
              ? "control.endBodyArchive"
              : "control.endBody"
            : "control.cancelBody",
        )}
        confirmLabel={t(onAir ? "control.end" : "control.cancel")}
        danger
        loading={end.isPending}
        onConfirm={() => end.mutate()}
      />
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("control.deleteTitle")}
        description={t("control.deleteBody")}
        confirmLabel={t("control.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
      {onAir && <RaidDialog stream={stream} open={raiding} onOpenChange={setRaiding} />}
    </>
  );
}

/** The dashboard of a stream that is on air: how many watch, how busy the chat is, what came in. */
function Health({ streamId }: { streamId: string }) {
  const { t, i18n } = useTranslation("live");
  const query = useQuery({
    queryKey: liveKeys.realtimeStats(streamId),
    queryFn: async () => (await analyticsClient().getRealtimeStats({ streamId })).stats ?? null,
    refetchInterval: 5000,
  });
  const stats = query.data;
  const now = useNow();
  if (!stats) return null;
  const count = (value: bigint) => formatCount(value, i18n.language);
  // Between two answers the uptime keeps counting on its own.
  const uptime = Number(stats.uptimeSeconds) + Math.floor((now - query.dataUpdatedAt) / 1000);
  return (
    <Section title={t("health.title")}>
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-4" data-testid="health">
        <Stat label={t("health.viewers")} value={count(stats.viewers)} testId="health-viewers" />
        <Stat label={t("health.peak")} value={count(stats.peakViewers)} />
        <Stat
          label={t("health.uptime")}
          value={stats.status === StreamStatus.LIVE ? clock(uptime) : "–"}
        />
        <Stat label={t("health.likes")} value={count(stats.likeCount)} />
        <Stat label={t("health.chat")} value={count(stats.chatMessageCount)} />
        <Stat label={t("health.chatRate")} value={count(stats.chatMessagesPerMinute)} />
        <Stat label={t("health.gifts")} value={count(stats.giftCoins)} />
        <Stat label={t("health.subscribers")} value={count(stats.newSubscribers)} />
      </div>
      {stats.viewerSamples.length > 1 && (
        <ViewerChart samples={stats.viewerSamples} label={t("health.chart")} />
      )}
    </Section>
  );
}

/** The polls of a stream: the open one, a way to start the next, and the ones before. */
function PollsPane({ stream }: { stream: live.Stream }) {
  const { t } = useTranslation("live");
  const streamId = stream.summary?.id ?? "";
  const [creating, setCreating] = useState(false);
  const query = useQuery({
    queryKey: liveKeys.polls(streamId),
    queryFn: async () => (await pollClient().listPolls({ streamId, page: { limit: 30 } })).polls,
  });
  const open = stream.activePoll && !stream.activePoll.endedAt ? stream.activePoll : undefined;
  const past = (query.data ?? []).filter((poll) => poll.id !== open?.id);
  const writable = stream.summary?.status !== StreamStatus.ENDED;
  return (
    <div className="flex flex-col gap-3">
      {writable && !open && (
        <Button icon={<Plus className="size-4" />} onClick={() => setCreating(true)}>
          {t("poll.create")}
        </Button>
      )}
      {open && <PollCard poll={open} canVote={false} canManage />}
      {past.length > 0 && (
        <h3 className="mt-2 text-sm font-black text-ink-soft">{t("poll.past")}</h3>
      )}
      {past.map((poll) => (
        <PollCard key={poll.id} poll={poll} canVote={false} canManage={false} />
      ))}
      {!open && past.length === 0 && !query.isPending && (
        <EmptyState emoji="📊" title={t("poll.emptyTitle")} body={t("poll.emptyBody")} />
      )}
      <CreatePollDialog streamId={streamId} open={creating} onOpenChange={setCreating} />
    </div>
  );
}

/** Ends the stream by sending its viewers to another one that is on air. */
function RaidDialog({
  stream,
  open,
  onOpenChange,
}: {
  stream: live.Stream;
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t, i18n } = useTranslation("live");
  const queryClient = useQueryClient();
  const streamId = stream.summary?.id ?? "";
  const [target, setTarget] = useState<live.StreamSummary | null>(null);
  const candidates = useQuery({
    queryKey: ["live", "list", "raid-targets"],
    queryFn: async () => {
      const [friends, everyone] = await Promise.all([
        streamClient().listFriendStreams({ page: { limit: 20 } }),
        streamClient().listLiveStreams({ page: { limit: 30 } }),
      ]);
      const seen = new Set<string>();
      return [...friends.streams, ...everyone.streams].filter((candidate) => {
        if (candidate.status !== StreamStatus.LIVE || seen.has(candidate.id)) return false;
        seen.add(candidate.id);
        return candidate.id !== streamId;
      });
    },
    enabled: open,
  });
  const raid = useMutation({
    mutationFn: () => streamClient().raidStream({ streamId, targetStreamId: target?.id ?? "" }),
    onSuccess: (response) => {
      storeStream(queryClient, response.stream);
      toast.success(
        t("raid.done", {
          name: response.target?.channel?.name ?? "",
          formatted: formatCount(response.viewerCount, i18n.language),
        }),
      );
      onOpenChange(false);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "live" })),
  });
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) setTarget(null);
        onOpenChange(next);
      }}
      title={t("raid.title")}
      description={t("raid.lead")}
      footer={
        <Button
          variant="danger"
          disabled={!target}
          loading={raid.isPending}
          icon={<Swords className="size-4" />}
          onClick={() => raid.mutate()}
        >
          {t("raid.confirm")}
        </Button>
      }
    >
      {candidates.isPending ? (
        <PaneSpinner />
      ) : candidates.data?.length ? (
        <ul className="flex flex-col gap-2">
          {candidates.data.map((candidate) => (
            <li key={candidate.id}>
              <button
                type="button"
                aria-pressed={target?.id === candidate.id}
                onClick={() => setTarget(candidate)}
                className={cn(
                  "block w-full rounded-2xl border-2 p-2 text-left",
                  target?.id === candidate.id ? "border-ink bg-brand-soft" : "border-line",
                )}
              >
                <span className="flex items-center gap-3">
                  <StreamPicture stream={candidate} className="w-28 shrink-0" />
                  <span className="min-w-0 flex-1">
                    <span className="line-clamp-2 text-sm leading-snug font-black">
                      <EmojiText text={candidate.title} />
                    </span>
                    <span className="block truncate text-xs font-bold text-ink-soft">
                      {candidate.channel?.name}
                    </span>
                    <span className="block text-xs font-bold text-ink-faint">
                      {t("viewers", {
                        formatted: formatCount(candidate.viewerCount, i18n.language),
                      })}
                    </span>
                  </span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <EmptyState emoji="🛰️" title={t("raid.emptyTitle")} body={t("raid.emptyBody")} />
      )}
    </Dialog>
  );
}
