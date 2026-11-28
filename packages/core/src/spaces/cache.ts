import type { InfiniteData, QueryClient } from "@tanstack/query-core";
import { fromDate, spaces } from "@tokuly/api";
import { pushTo, pushToAll, reread } from "../query";
import { spacesKeys } from "./keys";
import {
  mergeHangout,
  mergeSpace,
  withCounts,
  withoutParticipant,
  withParticipant,
  withPresence,
} from "./model";
import type { HangoutBoard, ParticipantPage, SpacePage } from "./queries";

/**
 * Rooms and hangouts in the query cache: a room on its own page and on every
 * list that shows it, a hangout on its page and on the board, and what a
 * pushed `SpacesEvent` changes in them. Both go by
 * `docs/realtime-consistency.md`, rule 2: their pushes go through `pushTo`,
 * so that a read made before a push cannot undo it (an ended room does not
 * show LIVE again, someone who left is not back on stage, a hangout the
 * person was taken out of does not come back with who is inside). What only
 * has to be read again is `reread`.
 */

/** Rewrites a room wherever it is loaded, and in a read of it in flight when it lands. */
export function patchSpace(
  queryClient: QueryClient,
  spaceId: string,
  patch: (space: spaces.Space) => spaces.Space,
): void {
  pushTo<spaces.Space | null>(queryClient, spacesKeys.space(spaceId), patch);
  pushToAll<InfiniteData<SpacePage, string>>(queryClient, spacesKeys.lists, (list) => {
    if (!list.pages.some((page) => page.spaces.some((space) => space.id === spaceId))) return list;
    return {
      ...list,
      pages: list.pages.map((page) => ({
        ...page,
        spaces: page.spaces.map((space) => (space.id === spaceId ? patch(space) : space)),
      })),
    };
  });
}

/** Stores a room the server just returned (with the caller's `viewer`). */
export function rememberSpace(queryClient: QueryClient, space: spaces.Space | undefined): void {
  if (!space) return;
  // Not loaded yet: this is the first the cache holds of it.
  if (!queryClient.getQueryData(spacesKeys.space(space.id)))
    queryClient.setQueryData(spacesKeys.space(space.id), space);
  patchSpace(queryClient, space.id, (loaded) => ({
    ...space,
    friendsInside: loaded.friendsInside,
  }));
}

/** Takes someone who left out of the room's lists of people, and out of a page of them in flight. */
function withoutPerson(queryClient: QueryClient, spaceId: string, userId: string): void {
  pushToAll<InfiniteData<ParticipantPage, string>>(
    queryClient,
    spacesKeys.everyParticipants(spaceId),
    (list) => {
      if (!list.pages.some((page) => page.participants.some((entry) => entry.user?.id === userId)))
        return list;
      return {
        ...list,
        pages: list.pages.map((page) => ({
          ...page,
          participants: page.participants.filter((entry) => entry.user?.id !== userId),
        })),
      };
    },
  );
}

/** Rewrites a hangout wherever it is loaded, and in a read of it in flight when it lands. */
export function patchHangout(
  queryClient: QueryClient,
  hangoutId: string,
  patch: (hangout: spaces.Hangout) => spaces.Hangout,
): void {
  pushTo<spaces.Hangout | null>(queryClient, spacesKeys.hangout(hangoutId), patch);
  const each = (list: spaces.Hangout[]) =>
    list.map((hangout) => (hangout.id === hangoutId ? patch(hangout) : hangout));
  pushTo<HangoutBoard>(queryClient, spacesKeys.hangouts, (board) => ({
    hangouts: each(board.hangouts),
    invitations: each(board.invitations),
  }));
}

/** Stores a hangout the server just returned (with the caller's `viewer`). */
export function rememberHangout(queryClient: QueryClient, hangout: spaces.Hangout | undefined) {
  if (!hangout) return;
  const each = (list: spaces.Hangout[]) =>
    list.map((loaded) => (loaded.id === hangout.id ? hangout : loaded));
  queryClient.setQueryData(spacesKeys.hangout(hangout.id), hangout);
  queryClient.setQueryData<HangoutBoard>(
    spacesKeys.hangouts,
    (board) => board && { hangouts: each(board.hangouts), invitations: each(board.invitations) },
  );
}

/**
 * Takes a hangout out of everything that is loaded (closed, quit, removed,
 * declined), and out of what a read in flight brings when it lands.
 */
export function forgetHangout(queryClient: QueryClient, hangoutId: string): void {
  const others = (list: spaces.Hangout[]) => list.filter((hangout) => hangout.id !== hangoutId);
  pushTo<spaces.Hangout | null>(queryClient, spacesKeys.hangout(hangoutId), () => null);
  pushTo<HangoutBoard>(queryClient, spacesKeys.hangouts, (board) => ({
    hangouts: others(board.hangouts),
    invitations: others(board.invitations),
  }));
}

/** Lists of people waiting to be read again, by cache and room. */
const pendingLists = new WeakMap<QueryClient, Map<string, ReturnType<typeof setTimeout>>>();

/**
 * Asks for the lists of people inside again, a moment later and once for
 * everything that happened in between: a busy room announces many hands.
 */
function refreshPeople(queryClient: QueryClient, spaceId: string): void {
  let pending = pendingLists.get(queryClient);
  if (!pending) {
    pending = new Map();
    pendingLists.set(queryClient, pending);
  }
  if (pending.has(spaceId)) return;
  const timers = pending;
  timers.set(
    spaceId,
    setTimeout(() => {
      timers.delete(spaceId);
      void reread(queryClient, [spacesKeys.everyParticipants(spaceId)]);
    }, 1200),
  );
}

/**
 * Brings everything that is loaded in line with an event of the spaces
 * domain. Emoji bursts (`reaction`) are not stored anywhere: whoever draws
 * them handles them.
 */
export function applySpacesEvent(
  queryClient: QueryClient,
  event: spaces.SpacesEvent["event"],
  myId: string,
): void {
  switch (event.case) {
    case "spaceUpdated": {
      const pushed = event.value.space;
      if (!pushed) return;
      const loaded = queryClient.getQueryData<spaces.Space | null>(spacesKeys.space(pushed.id));
      const changedState = loaded ? loaded.state !== pushed.state : true;
      patchSpace(queryClient, pushed.id, (space) => mergeSpace(space, pushed, myId));
      if (changedState) {
        // It moved between "upcoming", "live" and "over": the lists it is on changed.
        void reread(queryClient, [spacesKeys.lists, spacesKeys.friendsInRooms]);
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
      withoutPerson(queryClient, spaceId, userId);
      refreshPeople(queryClient, spaceId);
      if (userId === myId && event.value.reason === spaces.SpaceLeaveReason.BANNED) {
        // The room no longer exists for a banned person.
        pushTo<spaces.Space | null>(queryClient, spacesKeys.space(spaceId), () => null);
        void reread(queryClient, [spacesKeys.lists]);
      }
      return;
    }
    case "counts": {
      const counts = event.value;
      const loaded = queryClient.getQueryData<spaces.Space | null>(
        spacesKeys.space(counts.spaceId),
      );
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
    case "stageInvitation": {
      const { spaceId, invited } = event.value;
      patchSpace(queryClient, spaceId, (space) =>
        space.viewer ? { ...space, viewer: { ...space.viewer, stageInvited: invited } } : space,
      );
      return;
    }
    case "spaceInvitation":
      void reread(queryClient, [
        spacesKeys.lists,
        ...(event.value.space ? [spacesKeys.space(event.value.space.id)] : []),
      ]);
      return;
    case "hangoutUpdated": {
      const pushed = event.value.hangout;
      if (!pushed) return;
      patchHangout(queryClient, pushed.id, (hangout) => mergeHangout(hangout, pushed, myId));
      // New to the caller, or moved from "invited" to "member": the board's shape changed. A board
      // still being read for the first time is read again: whether it has the hangout is not known.
      const board = queryClient.getQueryData<HangoutBoard>(spacesKeys.hangouts);
      const listedAsMember = board?.hangouts.some((hangout) => hangout.id === pushed.id);
      const listed =
        listedAsMember || board?.invitations.some((hangout) => hangout.id === pushed.id);
      const mine = pushed.members.find((member) => member.user?.id === myId);
      const isMember = mine?.state === spaces.HangoutMemberState.MEMBER;
      if (!listed || listedAsMember !== isMember) void reread(queryClient, [spacesKeys.hangouts]);
      return;
    }
    case "hangoutRemoved":
      forgetHangout(queryClient, event.value.hangoutId);
      void reread(queryClient, [spacesKeys.friendsInRooms]);
      return;
    case "hangoutPresence": {
      const presence = event.value;
      // When it was pushed, also when it is replayed on a read that lands later.
      const now = fromDate(new Date());
      patchHangout(queryClient, presence.hangoutId, (hangout) =>
        withPresence(hangout, presence, myId, now),
      );
      void reread(queryClient, [spacesKeys.friendsInRooms]);
      return;
    }
    case "hangoutInvitation":
      void reread(queryClient, [spacesKeys.hangouts]);
      return;
    default:
      return;
  }
}
