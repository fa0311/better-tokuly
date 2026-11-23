import { errorReason, type Timestamp, toMillis, video } from "@tokuly/api";
import { useEffect, useRef, useState } from "react";
import { API_PATH, api } from "#/lib/api";
import { realtime } from "#/lib/realtime";
import { partyClient } from "../api";
import type { SharedPlayback } from "./sync";

const { PartyLeaveReason, PartyRole } = video;

/** Someone came or went: a line in the chat that is not a message. */
export interface PartyNotice {
  id: string;
  kind: "joined" | "left" | "removed";
  name: string;
  atMs: number;
}

/** Why the viewer is no longer in the room. */
export type PartyExit =
  | { kind: "ended"; reason: video.PartyEndReason }
  | { kind: "removed" }
  | { kind: "timedOut" }
  | { kind: "left" };

export interface PartyRoomState {
  party: video.Party;
  self: video.PartyMember;
  /** Everyone present, host first. */
  members: video.PartyMember[];
  messages: video.PartyMessage[];
  notices: PartyNotice[];
  /** The party's playback as last set, with the sequence number it carried. */
  playback: SharedPlayback & { sequence: bigint; changedByMemberId: string };
  /** Server clock minus this device's clock, in milliseconds. */
  clockOffsetMs: number;
  exit?: PartyExit;
}

function shared(playback: video.PartyPlayback | undefined): PartyRoomState["playback"] {
  return {
    playing: playback?.playing ?? false,
    positionSeconds: playback?.positionSeconds ?? 0,
    rate: playback?.rate || 1,
    updatedAtMs: toMillis(playback?.updatedAt),
    sequence: playback?.sequence ?? 0n,
    changedByMemberId: playback?.changedByMemberId ?? "",
  };
}

function offsetFrom(serverTime: Timestamp | undefined, fallback: number): number {
  return serverTime ? toMillis(serverTime) - Date.now() : fallback;
}

/** Host first, then co-hosts, then by arrival: the order the member list is shown in. */
function ordered(members: video.PartyMember[]): video.PartyMember[] {
  const rank = (member: video.PartyMember) =>
    member.role === PartyRole.HOST ? 0 : member.role === PartyRole.COHOST ? 1 : 2;
  return [...members].sort(
    (a, b) => rank(a) - rank(b) || toMillis(a.joinedAt) - toMillis(b.joinedAt),
  );
}

function fromSession(session: video.PartySession, previous?: PartyRoomState): PartyRoomState {
  const party = session.party as video.Party;
  return {
    party,
    self: session.self as video.PartyMember,
    members: ordered(session.members),
    messages: session.recentMessages,
    notices: previous?.notices ?? [],
    playback: shared(party.playback),
    clockOffsetMs: offsetFrom(session.serverTime, previous?.clockOffsetMs ?? 0),
    exit: party.endedAt
      ? { kind: "ended", reason: video.PartyEndReason.UNSPECIFIED }
      : previous?.exit,
  };
}

const HEARTBEAT_MS = 20_000;
/** A long party's chat keeps its latest entries only; what scrolled away hours ago is let go. */
export const CHAT_LIMIT = 200;

function latest<T>(entries: T[]): T[] {
  return entries.length > CHAT_LIMIT ? entries.slice(-CHAT_LIMIT) : entries;
}

/**
 * A seat in a watch party, kept alive and current: the party's realtime
 * topic delivers playback changes, arrivals, departures, messages and the
 * end; a heartbeat every 20 seconds holds the seat and re-reads the
 * playback in case a push was lost.
 */
export function usePartyRoom(initial: video.PartySession, guestToken: string) {
  const [state, setState] = useState<PartyRoomState>(() => fromSession(initial));
  const partyId = state.party.id;
  const selfId = state.self.id;
  const topic = initial.realtimeTopic;
  const gone = state.exit !== undefined;
  // The latest state for callbacks that outlive a render.
  const latestState = useRef(state);
  useEffect(() => {
    latestState.current = state;
  });

  const applyPlayback = (playback: video.PartyPlayback | undefined, serverTime?: Timestamp) =>
    setState((current) => {
      const next = shared(playback);
      const clockOffsetMs = offsetFrom(serverTime, current.clockOffsetMs);
      // Older states (a push overtaken by a response, or the other way round) are ignored.
      return playback && next.sequence > current.playback.sequence
        ? { ...current, playback: next, clockOffsetMs }
        : { ...current, clockOffsetMs };
    });

  const reload = async () => {
    try {
      const { session } = await partyClient().getParty({ partyId, guestToken });
      if (session?.party && session.self) setState((current) => fromSession(session, current));
    } catch (error) {
      const reason = errorReason(error);
      if (reason === "NOT_A_PARTY_MEMBER")
        setState((current) => ({ ...current, exit: current.exit ?? { kind: "timedOut" } }));
      else if (reason === "PARTY_NOT_FOUND")
        setState((current) => ({
          ...current,
          exit: current.exit ?? { kind: "ended", reason: video.PartyEndReason.UNSPECIFIED },
        }));
    }
  };
  const reloadRef = useRef(reload);
  useEffect(() => {
    reloadRef.current = reload;
  });

  useEffect(() => {
    if (gone) return;
    const client = realtime();
    const offFrame = client.onFrame((frame) => {
      if (frame.body.case === "welcome") {
        // Back after a gap: whatever was pushed meanwhile is read in one go.
        void reloadRef.current();
        return;
      }
      if (frame.body.case !== "video" || frame.topic !== topic) return;
      const event = frame.body.value.event;
      switch (event.case) {
        case "partyPlayback":
          setState((current) => {
            const next = shared(event.value);
            return next.sequence > current.playback.sequence
              ? { ...current, playback: next }
              : current;
          });
          break;
        case "partyMemberJoined": {
          const member = event.value;
          setState((current) => {
            const known = current.members.some((existing) => existing.id === member.id);
            const members = ordered([
              ...current.members.filter((existing) => existing.id !== member.id),
              member,
            ]);
            return {
              ...current,
              members,
              party: { ...current.party, memberCount: members.length },
              notices: known
                ? current.notices
                : latest([
                    ...current.notices,
                    {
                      id: `joined:${member.id}:${Date.now()}`,
                      kind: "joined" as const,
                      name: member.name,
                      atMs: Date.now() + current.clockOffsetMs,
                    },
                  ]),
            };
          });
          break;
        }
        case "partyMemberLeft": {
          const { memberId, reason, memberCount } = event.value;
          setState((current) => {
            if (memberId === current.self.id) {
              const exit: PartyExit =
                reason === PartyLeaveReason.REMOVED
                  ? { kind: "removed" }
                  : reason === PartyLeaveReason.TIMED_OUT
                    ? { kind: "timedOut" }
                    : { kind: "left" };
              return { ...current, exit: current.exit ?? exit };
            }
            const leaver = current.members.find((member) => member.id === memberId);
            return {
              ...current,
              members: current.members.filter((member) => member.id !== memberId),
              party: { ...current.party, memberCount },
              notices: leaver
                ? latest([
                    ...current.notices,
                    {
                      id: `left:${memberId}:${Date.now()}`,
                      kind:
                        reason === PartyLeaveReason.REMOVED
                          ? ("removed" as const)
                          : ("left" as const),
                      name: leaver.name,
                      atMs: Date.now() + current.clockOffsetMs,
                    },
                  ])
                : current.notices,
            };
          });
          break;
        }
        case "partyMemberUpdated": {
          const member = event.value;
          setState((current) => ({
            ...current,
            self: member.id === current.self.id ? member : current.self,
            members: ordered(
              current.members.map((existing) => (existing.id === member.id ? member : existing)),
            ),
          }));
          break;
        }
        case "partyMessage": {
          const message = event.value;
          setState((current) =>
            current.messages.some((existing) => existing.id === message.id)
              ? current
              : { ...current, messages: latest([...current.messages, message]) },
          );
          break;
        }
        case "partyUpdated":
          // The push leaves the video card out for friends-only videos: read the party as this member sees it.
          setState((current) => ({
            ...current,
            party: { ...current.party, allowGuests: event.value.allowGuests },
          }));
          void reloadRef.current();
          break;
        case "partyEnded":
          setState((current) => ({
            ...current,
            exit: { kind: "ended", reason: event.value.reason },
          }));
          break;
        default:
          break;
      }
    });
    // The seat went (removed, timed out) or the party ended without this page hearing why.
    const release = client.subscribe(topic, {
      onRevoked: (reason) =>
        setState((current) => ({
          ...current,
          exit:
            current.exit ??
            (reason === "TOPIC_CLOSED"
              ? { kind: "ended", reason: video.PartyEndReason.UNSPECIFIED }
              : { kind: "removed" }),
        })),
    });
    client.start();
    return () => {
      offFrame();
      release();
    };
  }, [topic, gone]);

  useEffect(() => {
    if (gone) return;
    const beat = async () => {
      try {
        const response = await partyClient().heartbeatParty({ partyId, guestToken });
        setState((current) => {
          const next = shared(response.playback);
          const clockOffsetMs = offsetFrom(response.serverTime, current.clockOffsetMs);
          return {
            ...current,
            clockOffsetMs,
            playback: next.sequence > current.playback.sequence ? next : current.playback,
            party: { ...current.party, memberCount: response.memberCount },
          };
        });
      } catch (error) {
        const reason = errorReason(error);
        if (reason === "NOT_A_PARTY_MEMBER")
          setState((current) => ({ ...current, exit: current.exit ?? { kind: "timedOut" } }));
        else if (reason === "PARTY_NOT_FOUND")
          setState((current) => ({
            ...current,
            exit: current.exit ?? { kind: "ended", reason: video.PartyEndReason.UNSPECIFIED },
          }));
      }
    };
    const timer = setInterval(() => void beat(), HEARTBEAT_MS);
    return () => clearInterval(timer);
  }, [partyId, guestToken, gone]);

  // Going elsewhere gives the seat up at once, so the others do not watch a ghost for a minute.
  // Inside the app that is an ordinary call; a page that is closing sends one that outlives it.
  const departure = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => {
    // (Development mounts every effect twice: a departure scheduled by the first pass is called off.)
    clearTimeout(departure.current);
    const present = () => !latestState.current.exit;
    const onPageHide = () => {
      if (!present()) return;
      const token = api().session.peekAccessToken();
      void fetch(`${API_PATH}/tokuly.video.v1.WatchPartyService/LeaveParty`, {
        method: "POST",
        keepalive: true,
        credentials: "include",
        headers: {
          "content-type": "application/json",
          "x-tokuly-client": "web",
          ...(token ? { authorization: `Bearer ${token}` } : {}),
        },
        body: JSON.stringify({ partyId, guestToken }),
      }).catch(() => {});
    };
    window.addEventListener("pagehide", onPageHide);
    return () => {
      window.removeEventListener("pagehide", onPageHide);
      departure.current = setTimeout(() => {
        if (present())
          void partyClient()
            .leaveParty({ partyId, guestToken })
            .catch(() => {});
      }, 0);
    };
  }, [partyId, guestToken]);

  return {
    state,
    latest: latestState,
    selfId,
    /** Reads the party again (after changing its video, a role, or the guest setting). */
    reload,
    /** Sets the party's playback; host and co-hosts only. */
    async setPlayback(next: { playing: boolean; positionSeconds: number; rate?: number }) {
      const response = await partyClient().setPartyPlayback({
        partyId,
        playing: next.playing,
        positionSeconds: next.positionSeconds,
        rate: next.rate ?? 0,
      });
      applyPlayback(response.playback, response.serverTime);
    },
    async sendMessage(message: { text?: string; stampId?: string }) {
      const response = await partyClient().sendPartyMessage({
        partyId,
        guestToken,
        text: message.text ?? "",
        stampId: message.stampId ?? "",
      });
      const sent = response.message;
      if (sent)
        setState((current) =>
          current.messages.some((existing) => existing.id === sent.id)
            ? current
            : { ...current, messages: latest([...current.messages, sent]) },
        );
    },
    async leave() {
      setState((current) => ({ ...current, exit: current.exit ?? { kind: "left" } }));
      await partyClient()
        .leaveParty({ partyId, guestToken })
        .catch(() => {});
    },
  };
}

export type PartyRoom = ReturnType<typeof usePartyRoom>;
