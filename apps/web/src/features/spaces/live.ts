import { type QueryClient, useQueryClient } from "@tanstack/react-query";
import type { spaces } from "@tokuly/api";
import { applySpacesEvent as applyToCache, spacesKeys } from "@tokuly/core/spaces";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";

type Event = spaces.SpacesEvent["event"];

/**
 * The rooms followed in this app, each subscribed once however many components
 * follow it (the room's screen and the audio session in the background), so a
 * room is read again once per confirmation, not once per follower.
 */
const followedRooms = new Map<string, { holders: number; release(): void }>();

function followRoom(queryClient: QueryClient, spaceId: string): () => void {
  let room = followedRooms.get(spaceId);
  if (!room) {
    const reread = () => {
      void queryClient.invalidateQueries({ queryKey: spacesKeys.space(spaceId) });
      void queryClient.invalidateQueries({ queryKey: spacesKeys.everyParticipants(spaceId) });
    };
    const { release } = realtime().subscribe(`space:${spaceId}`, {
      // In effect (first, or again after a gap): the room as read before may lack what was
      // pushed in between.
      onSubscribed: reread,
      // Banned, removed, blocked, the room deleted: reading it again shows what is left to see
      // (for a ban, "not found").
      onRevoked: reread,
    });
    room = { holders: 0, release };
    followedRooms.set(spaceId, room);
  }
  const held = room;
  held.holders++;
  let released = false;
  return () => {
    if (released) return;
    released = true;
    held.holders--;
    // Let go a moment later: one follower leaving as another arrives (the screen closing while
    // the session goes on) does not unsubscribe and subscribe again.
    queueMicrotask(() => {
      if (held.holders > 0 || followedRooms.get(spaceId) !== held) return;
      followedRooms.delete(spaceId);
      held.release();
    });
  };
}

/** Follows a room's realtime topic for as long as the calling component is mounted. */
export function useSpaceTopic(spaceId: string | undefined): void {
  const queryClient = useQueryClient();
  useEffect(() => {
    if (!spaceId) return;
    const release = followRoom(queryClient, spaceId);
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

/**
 * Brings everything that is loaded in line with an event of the spaces
 * domain, and draws the emoji bursts of others.
 */
export function applySpacesEvent(queryClient: QueryClient, event: Event, myId: string): void {
  if (event.case === "reaction") {
    const { spaceId, emoji, nonce } = event.value;
    // The sender drew their own burst when they tapped.
    if (nonce && sentNonces.delete(nonce)) return;
    emitReaction({ spaceId, emoji });
    return;
  }
  applyToCache(queryClient, event, myId);
}
