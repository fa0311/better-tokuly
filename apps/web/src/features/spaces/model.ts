import { type common, spaces, toMillis } from "@tokuly/api";

/**
 * Rooms and hangouts as plain values: who is on stage, what an event changes.
 * Nothing here touches the network or React, so it is what the tests pin down.
 */

const Role = spaces.SpaceRole;

/** Host, co-hosts and speakers: the people who may talk. */
export function onStage(role: spaces.SpaceRole): boolean {
  return role >= Role.SPEAKER;
}

/** The host and co-hosts. */
export function isModerator(role: spaces.SpaceRole): boolean {
  return role >= Role.COHOST;
}

export function nameOf(user: common.UserSummary | undefined): string {
  return user?.displayName || user?.handle || "";
}

/** Tags as people type them ("#部活, ゲーム  雑談") → what the API takes. */
export function parseTags(input: string): string[] {
  const tags = input
    .split(/[\s,、，]+/)
    .map((tag) => tag.replace(/^[#＃]+/, "").trim())
    .filter(Boolean);
  return [...new Set(tags)];
}

/** Host first, then co-hosts, then speakers in the order they came. */
export function sortStage(stage: spaces.SpaceParticipant[]): spaces.SpaceParticipant[] {
  return [...stage].sort((a, b) => b.role - a.role || toMillis(a.joinedAt) - toMillis(b.joinedAt));
}

/** The room after someone came on stage, changed role or stepped down. */
export function withParticipant(
  space: spaces.Space,
  participant: spaces.SpaceParticipant,
  myId: string,
): spaces.Space {
  const userId = participant.user?.id ?? "";
  const others = space.stage.filter((entry) => entry.user?.id !== userId);
  const stage = onStage(participant.role) ? sortStage([...others, participant]) : others;
  const host = participant.role === Role.HOST && participant.user ? participant.user : space.host;
  const viewer =
    space.viewer && userId === myId
      ? {
          ...space.viewer,
          inside: true,
          role: participant.role,
          handRaised: Boolean(participant.handRaisedAt),
          // Pushes never carry the invitation; coming on stage answers it.
          stageInvited: onStage(participant.role) ? false : space.viewer.stageInvited,
        }
      : space.viewer;
  return { ...space, stage, host, viewer };
}

/** The room after someone left it. */
export function withoutParticipant(
  space: spaces.Space,
  userId: string,
  myId: string,
): spaces.Space {
  const viewer =
    space.viewer && userId === myId
      ? { ...space.viewer, inside: false, handRaised: false, stageInvited: false }
      : space.viewer;
  return { ...space, stage: space.stage.filter((entry) => entry.user?.id !== userId), viewer };
}

export function withCounts(space: spaces.Space, counts: spaces.SpaceCounts): spaces.Space {
  return {
    ...space,
    listenerCount: counts.listenerCount,
    speakerCount: counts.speakerCount,
    handCount: counts.handCount,
    peakListeners: counts.peakListeners,
  };
}

/**
 * A pushed room over the one that is loaded. Pushes carry no `viewer` and no
 * friends, so the caller's own relation is kept and brought in line with the
 * stage (a handed-over room changes the caller's role).
 */
export function mergeSpace(
  loaded: spaces.Space | undefined,
  pushed: spaces.Space,
  myId: string,
): spaces.Space {
  if (!loaded) return pushed;
  const mine = pushed.stage.find((entry) => entry.user?.id === myId);
  const live = pushed.state === spaces.SpaceState.LIVE;
  const viewer = loaded.viewer && {
    ...loaded.viewer,
    inside: live && loaded.viewer.inside,
    role: mine
      ? mine.role
      : loaded.viewer.inside && onStage(loaded.viewer.role) && live
        ? Role.LISTENER
        : loaded.viewer.role,
  };
  return { ...pushed, viewer, friendsInside: loaded.friendsInside };
}

/** A pushed hangout over the loaded one: the caller's relation follows the member list. */
export function mergeHangout(
  loaded: spaces.Hangout | undefined,
  pushed: spaces.Hangout,
  myId: string,
): spaces.Hangout {
  if (!loaded?.viewer) return pushed;
  const mine = pushed.members.find((member) => member.user?.id === myId);
  const owner = pushed.owner?.id === myId;
  return {
    ...pushed,
    viewer: {
      ...loaded.viewer,
      state: mine?.state ?? loaded.viewer.state,
      owner,
      inside: mine?.inside ?? false,
      canInvite: owner || pushed.membersCanInvite,
    },
  };
}

/** A hangout after someone entered or left its voice room. */
export function withPresence(
  hangout: spaces.Hangout,
  presence: spaces.HangoutPresence,
  myId: string,
  now: spaces.HangoutMember["insideSince"],
): spaces.Hangout {
  const userId = presence.user?.id ?? "";
  const members = hangout.members.map((member) =>
    member.user?.id === userId
      ? { ...member, inside: presence.inside, insideSince: presence.inside ? now : undefined }
      : member,
  );
  // Those inside lead, as the API orders them; the rest keeps its order.
  members.sort((a, b) => Number(b.inside) - Number(a.inside));
  const viewer =
    hangout.viewer && userId === myId
      ? { ...hangout.viewer, inside: presence.inside }
      : hangout.viewer;
  return { ...hangout, members, insideCount: presence.insideCount, viewer };
}

/** The members who are in the voice room right now. */
export function insideOf(hangout: spaces.Hangout): spaces.HangoutMember[] {
  return hangout.members.filter((member) => member.inside);
}

/** How long a room ran, in whole minutes (at least one). */
export function minutesOf(space: spaces.Space): number {
  const started = toMillis(space.startedAt);
  const ended = toMillis(space.endedAt);
  if (!started || !ended) return 0;
  return Math.max(1, Math.round((ended - started) / 60_000));
}
