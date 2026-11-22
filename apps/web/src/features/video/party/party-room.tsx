import { useMutation } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { video } from "@tokuly/api";
import {
  DoorOpen,
  ListVideo,
  MoreVertical,
  PartyPopper,
  Play,
  Power,
  UserPlus,
  Users,
} from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { partyClient, useVideo } from "../api";
import { type PlayerHandle, VideoPlayer } from "../player/player";
import { ChannelAvatar } from "../video-card";
import { ChangeVideoDialog } from "./change-video";
import { PartyChat } from "./chat";
import { InviteDialog } from "./invite";
import { PartyMembers } from "./members";
import { differsFromShared, syncAction } from "./sync";
import { type PartyExit, usePartyRoom } from "./use-party";

const { PartyRole, PartyEndReason } = video;

/** How often every player is checked against the party's playback. */
const SYNC_INTERVAL_MS = 2500;

export interface PartyRoomProps {
  /** The seat this viewer got from creating or joining the party. */
  session: video.PartySession;
  /** A guest's proof of their seat; empty for members with an account. */
  guestToken?: string;
  /** Shown without the app's navigation around it (the guest page). */
  standalone?: boolean;
  /** Takes a seat again after being dropped for silence. */
  onRejoin?(): void;
  /** Where "leave" goes once the seat is given up. */
  onLeft(): void;
}

/**
 * A watch party's room: the video, played in step with the host; the chat;
 * the people in it. The host and co-hosts drive playback with the player's
 * own controls — everyone else's player follows, and says so.
 */
export function PartyRoomView({
  session,
  guestToken = "",
  standalone,
  onRejoin,
  onLeft,
}: PartyRoomProps) {
  const { t } = useTranslation("video");
  const room = usePartyRoom(session, guestToken);
  const { state } = room;
  const { party, self } = state;
  const player = useRef<PlayerHandle>(null);
  /** Changes this member sent that the server has not answered yet: the player is left alone meanwhile. */
  const pending = useRef(0);
  const [panel, setPanel] = useState<"chat" | "members">("chat");
  const [inviting, setInviting] = useState(false);
  const [changing, setChanging] = useState(false);
  const [ending, setEnding] = useState(false);
  const [needsTap, setNeedsTap] = useState(false);

  const isHost = self.role === PartyRole.HOST;
  const canControl = isHost || self.role === PartyRole.COHOST;
  const isGuest = self.isGuest;
  const videoId = party.video?.id ?? "";
  const duration = party.video?.durationSeconds ?? 0;
  const details = useVideo(videoId);
  const playback = details.data?.playback;

  const local = () => {
    const handle = player.current;
    return handle
      ? {
          positionSeconds: handle.currentTime(),
          paused: handle.paused(),
          rate: handle.element()?.playbackRate ?? 1,
        }
      : null;
  };
  const serverNow = () => Date.now() + room.latest.current.clockOffsetMs;

  /** Brings this member's player in line with the party. */
  const sync = () => {
    const handle = player.current;
    const mine = local();
    if (!handle || !mine || pending.current > 0 || room.latest.current.exit) return;
    const action = syncAction(room.latest.current.playback, mine, serverNow(), duration);
    if (action.rate !== undefined) handle.setRate(action.rate);
    if (action.seekTo !== undefined) handle.seek(action.seekTo);
    if (action.pause) handle.pause();
    // A browser may refuse to start sound nobody asked for: then the member taps once to join in.
    if (action.play)
      handle.play().then(
        () => setNeedsTap(false),
        () => setNeedsTap(true),
      );
  };
  const syncRef = useRef(sync);
  useEffect(() => {
    syncRef.current = sync;
  });

  // Every change of the party's playback is applied at once; a slow check catches drift and late-loading players.
  const sequence = state.playback.sequence;
  // biome-ignore lint/correctness/useExhaustiveDependencies: re-run for each new playback state and each new video.
  useEffect(() => {
    syncRef.current();
    const timer = setInterval(() => syncRef.current(), SYNC_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [sequence, videoId, playback?.hlsUrl]);

  /** Something happened in a controlling member's player: if it is their doing, the party follows. */
  const report = () => {
    const mine = local();
    if (!canControl || !mine || room.latest.current.exit) return;
    if (!differsFromShared(room.latest.current.playback, mine, serverNow(), duration)) return;
    pending.current += 1;
    room
      .setPlayback({
        playing: !mine.paused,
        positionSeconds: Math.min(mine.positionSeconds, Math.max(0, duration - 0.1)),
        rate: mine.rate,
      })
      .catch((error) => toast.error(errorMessage(error)))
      .finally(() => {
        pending.current -= 1;
      });
  };

  const end = useMutation({
    mutationFn: () => partyClient().endParty({ partyId: party.id }),
    onSuccess: () => setEnding(false),
    onError: (error) => {
      setEnding(false);
      toast.error(errorMessage(error));
    },
  });
  const guests = useMutation({
    mutationFn: (allowGuests: boolean) =>
      partyClient().updateParty({ partyId: party.id, allowGuests }),
    onSuccess: (_response, allowGuests) => {
      toast.success(allowGuests ? t("party.guestsOn") : t("party.guestsOff"));
      void room.reload();
    },
    onError: (error) => toast.error(errorMessage(error)),
  });
  const leave = async () => {
    await room.leave();
    onLeft();
  };

  if (state.exit && state.exit.kind !== "left")
    return (
      <Farewell
        exit={state.exit}
        videoId={party.video?.shortId}
        standalone={standalone}
        onRejoin={onRejoin}
      />
    );

  const menu: MenuAction[] = [
    ...(isHost
      ? [
          {
            label: party.allowGuests ? t("party.disallowGuests") : t("party.allowGuestsAction"),
            icon: <Users />,
            onSelect: () => guests.mutate(!party.allowGuests),
          },
        ]
      : []),
    { label: t("party.leave"), icon: <DoorOpen />, onSelect: () => void leave() },
    ...(isHost
      ? [{ label: t("party.end"), icon: <Power />, danger: true, onSelect: () => setEnding(true) }]
      : []),
  ];

  return (
    <div className="flex h-full flex-col lg:flex-row" data-testid="party-room">
      <div className="flex min-w-0 flex-col lg:flex-1 lg:overflow-y-auto">
        <header className="flex h-14 shrink-0 items-center gap-2 px-3">
          <span className="sticker-sm grid size-9 shrink-0 place-items-center rounded-xl bg-pink text-white">
            <PartyPopper className="size-5" />
          </span>
          <div className="min-w-0 flex-1">
            <h1 className="truncate leading-tight font-black">{t("party.title")}</h1>
            <p className="truncate text-xs font-bold text-ink-soft" data-testid="party-count">
              {t("party.watching", { count: party.memberCount })}
            </p>
          </div>
          <Button
            size="sm"
            icon={<UserPlus className="size-4" />}
            onClick={() => setInviting(true)}
          >
            {t("party.invite.action")}
          </Button>
          {canControl && (
            <Button
              size="icon-sm"
              variant="ghost"
              aria-label={t("party.change.title")}
              onClick={() => setChanging(true)}
            >
              <ListVideo className="size-5" />
            </Button>
          )}
          <Menu
            trigger={
              <Button size="icon-sm" variant="ghost" aria-label={t("party.menu")}>
                <MoreVertical className="size-5" />
              </Button>
            }
            actions={menu}
          />
        </header>

        <div className="relative lg:px-4">
          {playback ? (
            <VideoPlayer
              key={videoId}
              ref={player}
              source={playback}
              title={party.video?.title}
              chapters={details.data?.chapters}
              subtitles={details.data?.subtitles}
              shortcuts
              lockedReason={canControl ? undefined : t("party.locked")}
              onPlay={report}
              onPause={report}
              onSeeked={report}
              onRateChange={report}
              className="lg:rounded-card"
              endScreen={
                canControl ? (
                  <button
                    type="button"
                    className="pressable inline-flex h-12 items-center gap-2 rounded-full bg-white px-6 font-black text-[#17122b]"
                    onClick={() => setChanging(true)}
                  >
                    <ListVideo className="size-5" />
                    {t("party.change.next")}
                  </button>
                ) : (
                  <p className="max-w-xs text-center text-sm font-bold">{t("party.waitForHost")}</p>
                )
              }
            />
          ) : (
            <div className="grid aspect-video w-full place-items-center bg-black text-center text-sm font-bold text-white lg:rounded-card">
              {details.isError || (!details.isPending && !playback) ? (
                <p className="max-w-xs p-4">{t("party.videoUnavailable")}</p>
              ) : (
                <Skeleton className="size-full rounded-none bg-white/10" />
              )}
            </div>
          )}
          {needsTap && (
            <button
              type="button"
              className="absolute inset-0 grid place-items-center bg-black/60 lg:mx-4 lg:rounded-card"
              onClick={() => syncRef.current()}
            >
              <span className="pressable sticker inline-flex h-14 items-center gap-2 rounded-full bg-brand px-7 text-lg font-bold text-on-brand">
                <Play className="size-5 fill-current" />
                {t("party.tapToJoin")}
              </span>
            </button>
          )}
        </div>

        <div className="hidden items-center gap-3 px-4 py-3 lg:flex">
          {party.video?.channel && <ChannelAvatar channel={party.video.channel} size="md" />}
          <div className="min-w-0 flex-1">
            <p className="truncate text-lg font-black">{party.video?.title}</p>
            <p className="truncate text-sm text-ink-soft">{party.video?.channel?.name}</p>
          </div>
        </div>
        <p className="truncate px-4 py-2 text-sm font-black lg:hidden">{party.video?.title}</p>
      </div>

      <aside
        className={cn(
          "flex min-h-0 flex-1 flex-col border-line lg:w-96 lg:flex-none lg:border-l-2",
          !standalone && "pb-24 md:pb-0",
        )}
      >
        <div className="px-3 pt-1 pb-2 lg:pt-3">
          <Tabs<"chat" | "members">
            value={panel}
            onChange={setPanel}
            items={[
              { value: "chat", label: t("party.chat.title") },
              {
                value: "members",
                label: t("party.members.title", { count: state.members.length }),
              },
            ]}
          />
        </div>
        {panel === "chat" ? (
          <PartyChat room={room} canStamp={!isGuest} />
        ) : (
          <PartyMembers room={room} />
        )}
      </aside>

      <InviteDialog
        open={inviting}
        onOpenChange={setInviting}
        partyId={party.id}
        code={party.code}
        allowGuests={party.allowGuests}
        canInviteFriends={!isGuest}
      />
      {canControl && (
        <ChangeVideoDialog
          open={changing}
          onOpenChange={setChanging}
          party={party}
          onChanged={() => void room.reload()}
        />
      )}
      <ConfirmDialog
        open={ending}
        onOpenChange={setEnding}
        title={t("party.confirmEnd")}
        description={t("party.confirmEndBody")}
        confirmLabel={t("party.end")}
        danger
        loading={end.isPending}
        onConfirm={() => end.mutate()}
      />
    </div>
  );
}

const END_REASON_KEYS: Partial<Record<video.PartyEndReason, string>> = {
  [PartyEndReason.ENDED_BY_HOST]: "party.ended.byHost",
  [PartyEndReason.EMPTY]: "party.ended.empty",
  [PartyEndReason.EXPIRED]: "party.ended.expired",
  [PartyEndReason.VIDEO_UNAVAILABLE]: "party.ended.videoUnavailable",
};

/** What is left of the room once the viewer is no longer in it. */
function Farewell({
  exit,
  videoId,
  standalone,
  onRejoin,
}: {
  exit: PartyExit;
  videoId?: string;
  standalone?: boolean;
  onRejoin?(): void;
}) {
  const { t } = useTranslation("video");
  let emoji = "👋";
  let title = t("party.ended.title");
  let body: string | undefined =
    exit.kind === "ended" ? t(END_REASON_KEYS[exit.reason] ?? "party.ended.generic") : undefined;
  let action: ReactNode = null;
  if (exit.kind === "removed") {
    emoji = "🚪";
    title = t("party.removed.title");
    body = t("party.removed.body");
  } else if (exit.kind === "timedOut") {
    emoji = "😴";
    title = t("party.timedOut.title");
    body = t("party.timedOut.body");
    action = onRejoin && (
      <Button icon={<PartyPopper className="size-4" />} onClick={onRejoin}>
        {t("party.timedOut.rejoin")}
      </Button>
    );
  }
  return (
    <div className="grid h-full place-items-center p-6" data-testid="party-farewell">
      <div className="sticker max-w-sm rounded-card bg-surface p-6 text-center">
        <p className="text-5xl" aria-hidden>
          {emoji}
        </p>
        <h1 className="mt-3 text-xl font-black">{title}</h1>
        {body && <p className="mt-1 text-sm leading-relaxed text-ink-soft">{body}</p>}
        <div className="mt-5 flex flex-wrap justify-center gap-2">
          {action}
          {!standalone && videoId && (
            <Link
              to="/watch/$videoId"
              params={{ videoId }}
              search={{}}
              className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-surface px-5 font-bold"
            >
              <Play className="size-4" />
              {t("party.keepWatching")}
            </Link>
          )}
        </div>
      </div>
    </div>
  );
}
