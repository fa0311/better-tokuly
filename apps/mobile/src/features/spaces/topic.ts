import { type QueryClient, useQueryClient } from "@tanstack/react-query";
import { reread } from "@tokuly/core/query";
import { spacesKeys } from "@tokuly/core/spaces";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";

/**
 * The rooms followed in the app, each subscribed once however many
 * components follow it (the room's screen and the session in the
 * background), so a room is read again once per confirmation.
 */
const followed = new Map<string, { holders: number; release(): void }>();

function follow(queryClient: QueryClient, spaceId: string): () => void {
  let room = followed.get(spaceId);
  if (!room) {
    // A read already in flight may have been answered before: it is replaced, not joined.
    const readAgain = () =>
      void reread(queryClient, [spacesKeys.space(spaceId), spacesKeys.everyParticipants(spaceId)]);
    // In effect (first, or back after a gap), or revoked (banned, removed, blocked, deleted):
    // what was read before may be out of date, or no longer the viewer's to see.
    const { release } = realtime().subscribe(`space:${spaceId}`, {
      onSubscribed: readAgain,
      onRevoked: readAgain,
    });
    room = { holders: 0, release };
    followed.set(spaceId, room);
  }
  const held = room;
  held.holders++;
  let released = false;
  return () => {
    if (released) return;
    released = true;
    held.holders--;
    // A moment later: the screen closing as the session goes on does not resubscribe.
    queueMicrotask(() => {
      if (held.holders > 0 || followed.get(spaceId) !== held) return;
      followed.delete(spaceId);
      held.release();
    });
  };
}

/** Follows a room's realtime topic for as long as the calling component is mounted. */
export function useSpaceTopic(spaceId: string | undefined): void {
  const queryClient = useQueryClient();
  useEffect(() => {
    if (!spaceId) return;
    const release = follow(queryClient, spaceId);
    realtime().start();
    return release;
  }, [queryClient, spaceId]);
}
