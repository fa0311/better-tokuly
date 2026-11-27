import type { QueryClient } from "@tanstack/query-core";
import { spaces } from "@tokuly/api";
import { patchHangout, patchSpace } from "./cache";

/**
 * Why the person is not inside any more although they did not leave:
 * - `removed` / `banned`: a moderator's doing;
 * - `ended`: the room is over; `closed`: the hangout is gone (or they are no member any more);
 * - `elsewhere`: they entered with another device or tab;
 * - `lost`: the connection could not be brought back.
 */
export type ExitReason = "removed" | "banned" | "ended" | "closed" | "elsewhere" | "lost";

export interface Exit {
  kind: "space" | "hangout";
  id: string;
  reason: ExitReason;
}

/** How recent an entry elsewhere must be to explain being out of this venue. */
export const ELSEWHERE_WINDOW_MS = 15_000;

const Leave = spaces.SpaceLeaveReason;

/** Why the server says the person is outside, in the apps' words. */
export function exitReasonOf(reason: spaces.SpaceLeaveReason): ExitReason {
  if (reason === Leave.REMOVED) return "removed";
  if (reason === Leave.BANNED) return "banned";
  if (reason === Leave.ENDED) return "ended";
  if (reason === Leave.DISCONNECTED) return "lost";
  // They left with another device, or entered another room there.
  return "elsewhere";
}

/** The venue is not there for the person any more (not found, entering refused for good). */
export const goneReason = (kind: Exit["kind"]): ExitReason =>
  kind === "space" ? "removed" : "closed";

/**
 * Whether the person is still inside a room, by its own page (`undefined`:
 * not found): the reason they are not, otherwise. `cut` says the connection
 * was taken by a moderator or by the room's end rather than lost.
 */
export function whyOutsideSpace(space: spaces.Space | undefined, cut: boolean): ExitReason | null {
  if (!space) return "removed";
  if (space.state !== spaces.SpaceState.LIVE) return "ended";
  // Out on the server after a moderator's cut. Without one the connection was only gone too long.
  if (!space.viewer?.inside && cut) return "removed";
  return null;
}

/** Whether the person is still in a hangout, by its own page: the reason they are not, otherwise. */
export function whyOutsideHangout(hangout: spaces.Hangout | undefined): ExitReason | null {
  return hangout?.viewer?.state === spaces.HangoutMemberState.MEMBER ? null : "closed";
}

/** The person left a room: they are outside it, and their hand is down. */
export function markLeftSpace(queryClient: QueryClient, spaceId: string): void {
  patchSpace(queryClient, spaceId, (space) =>
    space.viewer
      ? { ...space, viewer: { ...space.viewer, inside: false, handRaised: false } }
      : space,
  );
}

/** The person left a hangout's voice room. */
export function markLeftHangout(queryClient: QueryClient, hangoutId: string): void {
  patchHangout(queryClient, hangoutId, (hangout) =>
    hangout.viewer ? { ...hangout, viewer: { ...hangout.viewer, inside: false } } : hangout,
  );
}

/** Who can be heard right now, by user ID. */
export interface Voices {
  /** Talking at this moment. */
  speaking: ReadonlySet<string>;
  /** Connected with an open microphone. */
  open: ReadonlySet<string>;
  /** Connected with audio (on stage, or in the hangout's voice room). */
  connected: ReadonlySet<string>;
}

export const SILENCE: Voices = { speaking: new Set(), open: new Set(), connected: new Set() };

/** A face stays lit this long after its voice went quiet, so it does not flicker between words. */
export const SPEAKING_LINGER_MS = 1200;

/** A participant of the audio connection, as far as the voices go (LiveKit's fields). */
export interface VoiceParticipant {
  identity: string;
  isSpeaking: boolean;
  isMicrophoneEnabled: boolean;
}

/**
 * The voices of an audio connection's participants at `now`; `lastSpoke`
 * holds when each was last heard talking (ms), so a voice lingers for
 * `SPEAKING_LINGER_MS`.
 */
export function voicesOf(
  participants: Iterable<VoiceParticipant>,
  lastSpoke: ReadonlyMap<string, number>,
  now: number,
): Voices {
  const speaking = new Set<string>();
  const open = new Set<string>();
  const connected = new Set<string>();
  for (const participant of participants) {
    connected.add(participant.identity);
    if (participant.isMicrophoneEnabled) open.add(participant.identity);
    const heard =
      participant.isSpeaking ||
      now - (lastSpoke.get(participant.identity) ?? 0) < SPEAKING_LINGER_MS;
    if (heard && participant.isMicrophoneEnabled) speaking.add(participant.identity);
  }
  return { speaking, open, connected };
}
