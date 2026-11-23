import { type QueryClient, useQueryClient } from "@tanstack/react-query";
import { fromDate, spaces } from "@tokuly/api";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";
import { forgetHangout, type HangoutBoard, keys, patchHangout, patchSpace } from "./api";
import {
  mergeHangout,
  mergeSpace,
  withCounts,
  withoutParticipant,
  withParticipant,
  withPresence,
} from "./model";

type Event = spaces.SpacesEvent["event"];

/** Follows a room's realtime topic for as long as the calling component is mounted. */
export function useSpaceTopic(spaceId: string | undefined): void {
  const queryClient = useQueryClient();
  useEffect(() => {
    if (!spaceId) return;
    // Banned, removed, blocked, the room deleted: reading it again shows what is left to see
    // (for a ban, "not found").
    const release = realtime().subscribe(`space:${spaceId}`, {
      onRevoked: () => void queryClient.invalidateQueries({ queryKey: keys.space(spaceId) }),
    });
    realtime().start();
    return release;
  }, [queryClient, spaceId]);
}

/** An emoji someone sent into a room. Bursts do not say who sent them. */
export interface Burst {
  spaceId: string;
  /** One emoji, or a custom emoji as `:shortcode:`. */
  emoji: string;
}

/** Nonces of the bursts this browser sent and has not heard back yet. */
const sentNonces = new Set<string>();
const MAX_SENT_NONCES = 64;

/**
 * A fresh tag for a burst this browser is about to send, so the copy that
 * comes back over realtime is recognised and not drawn twice. It is random
 * for every burst: everyone in the room sees it.
 */
export function newBurstNonce(): string {
  const nonce = crypto.randomUUID();
  sentNonces.add(nonce);
  if (sentNonces.size > MAX_SENT_NONCES) {
    const oldest = sentNonces.values().next().value;
    if (oldest !== undefined) sentNonces.delete(oldest);
  }
  return nonce;
}

const reactionListeners = new Set<(reaction: Burst) => void>();

/** Emoji bursts are not stored anywhere: whoever draws them listens here. */
export function onReaction(listener: (reaction: Burst) => void): () => void {
  reactionListeners.add(listener);
  return () => reactionListeners.delete(listener);
}

/** Shows a burst to this browser's own listeners (the sender sees theirs at once). */
export function emitReaction(reaction: Burst): void {
  for (const listener of reactionListeners) listener(reaction);
}

const pendingLists = new Map<string, ReturnType<typeof setTimeout>>();

/**
 * Asks for the lists of people inside again, a moment later and once for
 * everything that happened in between: a busy room announces many hands.
 */
function refreshPeople(queryClient: QueryClient, spaceId: string): void {
  if (pendingLists.has(spaceId)) return;
  pendingLists.set(
    spaceId,
    setTimeout(() => {
      pendingLists.delete(spaceId);
      void queryClient.invalidateQueries({ queryKey: keys.everyParticipants(spaceId) });
    }, 1200),
  );
}

/** Brings everything that is loaded in line with an event of the spaces domain. */
export function applySpacesEvent(queryClient: QueryClient, event: Event, myId: string): void {
  switch (event.case) {
    case "spaceUpdated": {
      const pushed = event.value.space;
      if (!pushed) return;
      const loaded = queryClient.getQueryData<spaces.Space | null>(keys.space(pushed.id));
      const changedState = loaded ? loaded.state !== pushed.state : true;
      patchSpace(queryClient, pushed.id, (space) => mergeSpace(space, pushed, myId));
      if (changedState) {
        // It moved between "upcoming", "live" and "over": the lists it is on changed.
        void queryClient.invalidateQueries({ queryKey: keys.lists });
        void queryClient.invalidateQueries({ queryKey: keys.friendsInRooms });
        refreshPeople(queryClient, pushed.id);
      }
      return;
    }
    case "participantUpdated": {
      const { spaceId, participant } = event.value;
      if (!participant) return;
      patchSpace(queryClient, spaceId, (space) => withParticipant(space, participant, myId));
      refreshPeople(queryClient, spaceId);
      return;
    }
    case "participantLeft": {
      const { spaceId, userId } = event.value;
      patchSpace(queryClient, spaceId, (space) => withoutParticipant(space, userId, myId));
      refreshPeople(queryClient, spaceId);
      if (userId === myId && event.value.reason === spaces.SpaceLeaveReason.BANNED) {
        // The room no longer exists for a banned person.
        queryClient.setQueryData(keys.space(spaceId), null);
        void queryClient.invalidateQueries({ queryKey: keys.lists });
      }
      return;
    }
    case "counts": {
      const counts = event.value;
      const loaded = queryClient.getQueryData<spaces.Space | null>(keys.space(counts.spaceId));
      patchSpace(queryClient, counts.spaceId, (space) => withCounts(space, counts));
      // Listeners come and go without being announced one by one, and so
      // do the hands of listeners who keep their presence to themselves (or
      // stopped sharing it): the count is all the room hears of those.
      if (
        loaded &&
        (loaded.listenerCount !== counts.listenerCount || loaded.handCount !== counts.handCount)
      )
        refreshPeople(queryClient, counts.spaceId);
      return;
    }
    case "reaction": {
      const { spaceId, emoji, nonce } = event.value;
      // The sender drew their own burst when they tapped.
      if (nonce && sentNonces.delete(nonce)) return;
      emitReaction({ spaceId, emoji });
      return;
    }
    case "stageInvitation": {
      const { spaceId, invited } = event.value;
      patchSpace(queryClient, spaceId, (space) =>
        space.viewer ? { ...space, viewer: { ...space.viewer, stageInvited: invited } } : space,
      );
      return;
    }
    case "spaceInvitation":
      void queryClient.invalidateQueries({ queryKey: keys.lists });
      if (event.value.space)
        void queryClient.invalidateQueries({ queryKey: keys.space(event.value.space.id) });
      return;
    case "hangoutUpdated": {
      const pushed = event.value.hangout;
      if (!pushed) return;
      const board = queryClient.getQueryData<HangoutBoard>(keys.hangouts);
      const known =
        !board ||
        [...board.hangouts, ...board.invitations].some((hangout) => hangout.id === pushed.id);
      patchHangout(queryClient, pushed.id, (hangout) => mergeHangout(hangout, pushed, myId));
      // New to the caller, or moved from "invited" to "member": the board's shape changed.
      const mine = pushed.members.find((member) => member.user?.id === myId);
      const listedAsMember = board?.hangouts.some((hangout) => hangout.id === pushed.id);
      const isMember = mine?.state === spaces.HangoutMemberState.MEMBER;
      if (!known || (board && listedAsMember !== isMember))
        void queryClient.invalidateQueries({ queryKey: keys.hangouts });
      return;
    }
    case "hangoutRemoved":
      forgetHangout(queryClient, event.value.hangoutId);
      void queryClient.invalidateQueries({ queryKey: keys.friendsInRooms });
      return;
    case "hangoutPresence": {
      const presence = event.value;
      patchHangout(queryClient, presence.hangoutId, (hangout) =>
        withPresence(hangout, presence, myId, fromDate(new Date())),
      );
      void queryClient.invalidateQueries({ queryKey: keys.friendsInRooms });
      return;
    }
    case "hangoutInvitation":
      void queryClient.invalidateQueries({ queryKey: keys.hangouts });
      return;
    default:
      return;
  }
}
