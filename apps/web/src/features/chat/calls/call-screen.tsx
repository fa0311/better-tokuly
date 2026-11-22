import { chat, type common, toMillis } from "@tokuly/api";
import type { Participant, Track } from "livekit-client";
import { Grid2x2, Maximize2, Mic, MicOff, Minimize2, MonitorUp, PhoneOff, RectangleHorizontal, Video, VideoOff, Volume2 } from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { cn } from "#/lib/cn";
import { useMe } from "#/stores/session";
import { useConversation } from "../conversations";
import { useLiveCall } from "../live";
import { displayName, titleOf } from "../people";
import { playTime } from "../text";
import { ring } from "./ringtone";
import { type ActiveCall, canShareScreen, currentRoom, hangUp, setMinimized, toggleCamera, toggleMicrophone, toggleScreenShare, unblockAudio, useCallSession } from "./session";

// LiveKit's track sources, by value: the client library itself is only loaded when a call starts.
const CAMERA = "camera" as Track.Source;
const SCREEN = "screen_share" as Track.Source;
const MICROPHONE = "microphone" as Track.Source;

/** One person (or a shared screen) in the call, as drawn. */
interface Tile {
  key: string;
  user: common.UserSummary | undefined;
  name: string;
  local: boolean;
  /** The picture to show: their camera, or the screen they share. */
  video: Track | undefined;
  screen: boolean;
  micOn: boolean;
  speaking: boolean;
  /** Being rung: not in the room yet. */
  waiting: boolean;
}

function videoOf(participant: Participant, source: Track.Source): Track | undefined {
  const publication = participant.getTrackPublication(source);
  return publication && !publication.isMuted ? (publication.track ?? undefined) : undefined;
}

/** The tiles of the call: everyone connected (shared screens first), then the people still being rung. */
function tilesOf(call: chat.Call | undefined, me: common.UserSummary, connected: boolean): Tile[] {
  const room = currentRoom();
  const known = new Map((call?.participants ?? []).flatMap((participant) => (participant.user ? [[participant.user.id, participant.user] as const] : [])));
  known.set(me.id, me);
  const tiles: Tile[] = [];
  const present = new Set<string>([me.id]);
  if (room && connected) {
    for (const participant of [room.localParticipant, ...room.remoteParticipants.values()]) {
      const local = participant === room.localParticipant;
      const user = known.get(participant.identity);
      present.add(participant.identity);
      const base = { user, name: user ? displayName(user) : participant.name || participant.identity, local, micOn: participant.isMicrophoneEnabled, speaking: participant.isSpeaking, waiting: false };
      const screen = videoOf(participant, SCREEN);
      if (screen) tiles.unshift({ ...base, key: `${participant.identity}:screen`, video: screen, screen: true });
      tiles.push({ ...base, key: participant.identity, video: videoOf(participant, CAMERA), screen: false });
    }
  } else {
    tiles.push({ key: me.id, user: me, name: displayName(me), local: true, video: undefined, screen: false, micOn: false, speaking: false, waiting: false });
  }
  for (const participant of call?.participants ?? []) {
    if (!participant.user || present.has(participant.user.id) || participant.state !== chat.CallParticipantState.RINGING) continue;
    tiles.push({ key: participant.user.id, user: participant.user, name: displayName(participant.user), local: false, video: undefined, screen: false, micOn: false, speaking: false, waiting: true });
  }
  return tiles;
}

/** Remote sound, one element per microphone track. */
function RemoteAudio() {
  // The LiveKit room is a mutable object outside React: read it afresh on every render (see `CallScreen`).
  "use no memo";
  useCallSession((state) => state.active?.revision);
  const room = currentRoom();
  const tracks = room ? [...room.remoteParticipants.values()].flatMap((participant) => participant.getTrackPublication(MICROPHONE)?.track ?? []) : [];
  return (
    <>
      {tracks.map((track) => (
        <AudioView key={track.sid} track={track} />
      ))}
    </>
  );
}

function AudioView({ track }: { track: Track }) {
  const element = useRef<HTMLAudioElement>(null);
  useEffect(() => {
    const audio = element.current;
    if (!audio) return;
    track.attach(audio);
    return () => {
      track.detach(audio);
    };
  }, [track]);
  return <audio ref={element} autoPlay />;
}

function VideoView({ track, mirrored, contain }: { track: Track; mirrored?: boolean; contain?: boolean }) {
  const element = useRef<HTMLVideoElement>(null);
  useEffect(() => {
    const video = element.current;
    if (!video) return;
    track.attach(video);
    return () => {
      track.detach(video);
    };
  }, [track]);
  return <video ref={element} autoPlay playsInline muted className={cn("size-full", contain ? "object-contain" : "object-cover", mirrored && "-scale-x-100")} />;
}

function TileView({ tile, small, className }: { tile: Tile; small?: boolean; className?: string }) {
  const { t } = useTranslation("chat");
  return (
    <div
      // The "speaking" mark is an inset outline: the strip of small tiles clips anything drawn outside them.
      className={cn("relative grid place-items-center overflow-hidden rounded-3xl bg-white/10", tile.speaking && "outline-4 -outline-offset-4 outline-lime", tile.waiting && "opacity-60", className)}
      data-testid="call-tile"
      data-waiting={tile.waiting || undefined}
    >
      {tile.video ? (
        <VideoView track={tile.video} mirrored={tile.local && !tile.screen} contain={tile.screen} />
      ) : (
        <span className={cn(tile.waiting && "animate-pulse")}>
          <Avatar user={tile.user} size={small ? "md" : "xl"} outlined />
        </span>
      )}
      <span className="absolute bottom-2 left-2 flex max-w-[calc(100%-1rem)] items-center gap-1 rounded-full bg-black/55 px-2.5 py-1 text-xs font-bold text-white">
        {!tile.micOn && !tile.waiting && !tile.screen && <MicOff className="size-3.5 shrink-0 text-live" />}
        <span className="truncate">{tile.screen ? t("call.screenOf", { name: tile.name }) : tile.local ? t("call.you") : tile.name}</span>
        {tile.waiting && <span className="shrink-0 text-white/70">· {t("call.ringing")}</span>}
      </span>
    </div>
  );
}

function RoundButton({ label, active, danger, onClick, children }: { label: string; active?: boolean; danger?: boolean; onClick(): void; children: ReactNode }) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "pressable grid size-14 shrink-0 place-items-center rounded-full border-2 border-white/20 text-white [&>svg]:size-6",
        danger ? "border-transparent bg-danger" : active ? "border-transparent bg-white text-[#17122b]" : "bg-white/15",
      )}
    >
      {children}
    </button>
  );
}

/** Seconds since `sinceMs`, ticking; 0 while unset. */
function useElapsed(sinceMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!sinceMs) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [sinceMs]);
  return sinceMs ? Math.max(0, (now - sinceMs) / 1000) : 0;
}

/** The call this browser is in: full screen, or shrunk to a pill while the app is used. */
export function CallScreen({ active }: { active: ActiveCall }) {
  // The LiveKit room changes in place (participants, tracks, who speaks) and `active.revision` says when.
  // The compiler would cache what was read from it, so this component is rendered without memoization.
  "use no memo";
  const { t } = useTranslation("chat");
  const me = useMe()?.user;
  const call = useLiveCall(active.callId);
  const conversation = useConversation(active.conversationId).data;
  const [view, setView] = useState<"speaker" | "grid">("speaker");
  const answered = call?.state === chat.CallState.ACTIVE;
  const elapsed = useElapsed(answered ? toMillis(call?.answeredAt) : 0);
  const waitingForAnswer = active.startedByMe && !answered;

  // The caller hears a soft tone until somebody answers.
  useEffect(() => (waitingForAnswer ? ring("outgoing") : undefined), [waitingForAnswer]);

  if (!me) return null;
  const connected = active.phase === "connected" || active.phase === "reconnecting";
  const tiles = tilesOf(call, me, connected);
  const title = conversation ? titleOf(conversation, me.id) : "";
  const status =
    active.phase === "reconnecting"
      ? t("call.status.reconnecting")
      : answered && connected
        ? playTime(elapsed)
        : waitingForAnswer
          ? t("call.status.calling")
          : t("call.status.connecting");

  if (active.minimized) {
    return (
      <div className="sticker fixed right-3 bottom-24 z-[60] flex items-center gap-2 rounded-full bg-success py-1.5 pr-1.5 pl-3 text-white md:bottom-6" data-testid="call-pill">
        <button type="button" onClick={() => setMinimized(false)} className="flex items-center gap-2 text-sm font-black" aria-label={t("call.expand")}>
          <Maximize2 className="size-4" />
          <span className="max-w-28 truncate">{title}</span>
          <span className="tabular-nums">{status}</span>
        </button>
        <button type="button" onClick={() => void hangUp()} aria-label={t("call.leave")} className="grid size-9 place-items-center rounded-full bg-danger">
          <PhoneOff className="size-4" />
        </button>
        <RemoteAudio />
      </div>
    );
  }

  // Speaker view: one big tile (a shared screen, else whoever talks, else the other person) and the rest in a strip.
  const others = tiles.filter((tile) => !tile.local);
  const main = tiles.find((tile) => tile.screen) ?? others.find((tile) => tile.speaking && !tile.waiting) ?? others.find((tile) => !tile.waiting) ?? others[0] ?? tiles[0]!;
  const strip = tiles.filter((tile) => tile !== main);
  const grid = view === "grid" && tiles.length > 2;

  return (
    <div
      role="dialog"
      aria-label={t("call.title", { name: title })}
      className="fixed inset-0 z-[60] flex flex-col bg-[#17122b] bg-[radial-gradient(120%_80%_at_50%_0%,color-mix(in_srgb,var(--brand)_55%,transparent),transparent)] text-white"
      data-testid="call-screen"
    >
      <header className="flex shrink-0 items-center gap-3 px-4 pt-safe pb-2">
        <button type="button" onClick={() => setMinimized(true)} aria-label={t("call.minimize")} title={t("call.minimize")} className="grid size-10 place-items-center rounded-full bg-white/15">
          <Minimize2 className="size-5" />
        </button>
        <div className="min-w-0 flex-1">
          <p className="truncate font-black">{title}</p>
          <p className="text-sm font-bold text-white/75 tabular-nums" data-testid="call-status">
            {status}
          </p>
        </div>
        {tiles.length > 2 && (
          <button
            type="button"
            onClick={() => setView(grid ? "speaker" : "grid")}
            aria-label={grid ? t("call.speakerView") : t("call.gridView")}
            title={grid ? t("call.speakerView") : t("call.gridView")}
            className="grid size-10 place-items-center rounded-full bg-white/15"
          >
            {grid ? <RectangleHorizontal className="size-5" /> : <Grid2x2 className="size-5" />}
          </button>
        )}
      </header>

      <div className="relative min-h-0 flex-1 p-3">
        {grid ? (
          <div className={cn("grid size-full auto-rows-fr gap-2", tiles.length <= 4 ? "grid-cols-2" : "grid-cols-2 md:grid-cols-3")}>
            {tiles.map((tile) => (
              <TileView key={tile.key} tile={tile} />
            ))}
          </div>
        ) : (
          <>
            <TileView tile={main} className="size-full" />
            {strip.length > 0 && (
              <div className="scrollbar-none absolute right-5 bottom-5 flex max-w-[calc(100%-2.5rem)] gap-2 overflow-x-auto">
                {strip.map((tile) => (
                  <TileView key={tile.key} tile={tile} small className="h-36 w-24 shrink-0 rounded-2xl border-2 border-white/30 bg-[#2d274c] md:h-40 md:w-28" />
                ))}
              </div>
            )}
          </>
        )}
        {active.audioBlocked && (
          <button type="button" onClick={() => void unblockAudio()} className="sticker absolute top-5 left-1/2 flex -translate-x-1/2 items-center gap-2 rounded-full bg-sun px-4 py-2 text-sm font-black text-[#17122b]">
            <Volume2 className="size-4" />
            {t("call.enableSound")}
          </button>
        )}
      </div>

      <footer className="flex shrink-0 items-center justify-center gap-3 px-4 pt-2 pb-safe">
        <RoundButton label={active.micOn ? t("call.mute") : t("call.unmute")} active={!active.micOn} onClick={() => void toggleMicrophone()}>
          {active.micOn ? <Mic /> : <MicOff />}
        </RoundButton>
        <RoundButton label={active.cameraOn ? t("call.cameraOff") : t("call.cameraOn")} active={!active.cameraOn} onClick={() => void toggleCamera()}>
          {active.cameraOn ? <Video /> : <VideoOff />}
        </RoundButton>
        {canShareScreen() && (
          <RoundButton label={active.screenOn ? t("call.stopSharing") : t("call.shareScreen")} active={active.screenOn} onClick={() => void toggleScreenShare()}>
            <MonitorUp />
          </RoundButton>
        )}
        <RoundButton label={t("call.leave")} danger onClick={() => void hangUp()}>
          <PhoneOff />
        </RoundButton>
      </footer>
      <RemoteAudio />
    </div>
  );
}
