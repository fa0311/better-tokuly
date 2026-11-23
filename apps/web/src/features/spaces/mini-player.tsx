import { useQueryClient } from "@tanstack/react-query";
import { Link, useLocation } from "@tanstack/react-router";
import { LogOut, Mic, MicOff, Volume2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";
import {
  type ActiveVenue,
  leaveVenue,
  toggleMicrophone,
  unblockAudio,
  useAudioSession,
  useVoices,
} from "./session";

/** The page of a venue: where the full view of what is playing lives. */
function pathOf(venue: Pick<ActiveVenue, "kind" | "id">): string {
  return venue.kind === "space" ? `/spaces/${venue.id}` : `/spaces/hangouts/${venue.id}`;
}

/**
 * The room the person is inside, as a pill that follows them around the app:
 * what is playing, whether someone talks, mute and leave. On the room's own
 * page the page itself is the player.
 */
export function MiniPlayer() {
  const active = useAudioSession((state) => state.active);
  const pathname = useLocation({ select: (location) => location.pathname });
  if (!active || pathname.replace(/\/+$/, "") === pathOf(active)) return null;
  return <Pill active={active} />;
}

function Pill({ active }: { active: ActiveVenue }) {
  const { t } = useTranslation("spaces");
  const queryClient = useQueryClient();
  const voices = useVoices(active.kind, active.id);
  const talking = voices.speaking.size > 0;
  const round = "grid size-9 shrink-0 place-items-center rounded-full";
  const open =
    active.kind === "space" ? (
      <Link
        to="/spaces/$spaceId"
        params={{ spaceId: active.id }}
        className="flex min-w-0 items-center gap-2"
        aria-label={t("mini.open", { title: active.title })}
      >
        <PillTitle active={active} talking={talking} />
      </Link>
    ) : (
      <Link
        to="/spaces/hangouts/$hangoutId"
        params={{ hangoutId: active.id }}
        className="flex min-w-0 items-center gap-2"
        aria-label={t("mini.open", { title: active.title })}
      >
        <PillTitle active={active} talking={talking} />
      </Link>
    );
  return (
    <div
      className="sticker fixed bottom-24 left-3 z-40 flex max-w-[calc(100vw-1.5rem)] animate-pop-in items-center gap-1.5 rounded-full bg-ink py-1.5 pr-1.5 pl-3 text-bg md:right-3 md:bottom-20 md:left-auto"
      data-testid="mini-player"
      data-phase={active.phase}
    >
      {open}
      {active.audioBlocked && (
        <button
          type="button"
          onClick={() => void unblockAudio()}
          aria-label={t("room.audioBlocked")}
          className={cn(round, "bg-lime text-[#17122b]")}
        >
          <Volume2 className="size-4" />
        </button>
      )}
      {active.canPublish && (
        <button
          type="button"
          aria-pressed={active.micOn}
          aria-label={t(active.micOn ? "room.mute" : "room.unmute")}
          disabled={active.phase !== "connected"}
          onClick={() => void toggleMicrophone(queryClient)}
          className={cn(
            round,
            "disabled:opacity-50",
            active.micOn ? "bg-lime text-[#17122b]" : "bg-surface text-ink",
          )}
        >
          {active.micOn ? <Mic className="size-4" /> : <MicOff className="size-4" />}
        </button>
      )}
      <button
        type="button"
        onClick={() => void leaveVenue(queryClient)}
        aria-label={t("mini.leave")}
        className={cn(round, "bg-danger text-white")}
      >
        <LogOut className="size-4" />
      </button>
    </div>
  );
}

function PillTitle({ active, talking }: { active: ActiveVenue; talking: boolean }) {
  const { t } = useTranslation("spaces");
  return (
    <>
      {active.emoji ? (
        <span aria-hidden>{active.emoji}</span>
      ) : (
        <span
          aria-hidden
          className={cn(
            "size-2.5 shrink-0 rounded-full bg-live",
            (talking || active.phase !== "connected") && "animate-pulse",
          )}
        />
      )}
      <span className="flex min-w-0 flex-col">
        <span className="max-w-36 truncate text-sm leading-tight font-black">
          <EmojiText text={active.title} />
        </span>
        <span className="text-[11px] leading-tight font-bold opacity-70">
          {t(
            active.phase === "connected"
              ? talking
                ? "mini.talking"
                : "mini.listening"
              : active.phase === "connecting"
                ? "room.connecting"
                : "mini.reconnecting",
          )}
        </span>
      </span>
    </>
  );
}
