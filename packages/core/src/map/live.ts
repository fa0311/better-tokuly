import { geo, toMillis } from "@tokuly/api";

/**
 * Friends' live locations as an app holds them: the latest location per user
 * ID (as the server lets the viewer see it), who has the app open, and what
 * orders them against a snapshot that answers late. The edits below are what
 * the realtime connection's events do to them.
 *
 * The server numbers every push that changes who is on the viewer's map — a
 * removal, the pin of someone who just became visible — and a snapshot says
 * how many of them it was read after (`removalSequence` on both).
 */
export interface LiveLocations {
  friends: Record<string, geo.FriendLocation>;
  online: Record<string, boolean>;
  /**
   * Who a numbered removal took off the map: the highest number, and the last
   * snapshot asked for before the removal arrived (see `asked`).
   */
  removed: Record<string, Mark>;
  /** Who a numbered pin showed while a snapshot was on its way, marked the same way. */
  appeared: Record<string, Mark>;
  /** Snapshots asked for so far; each answer is applied with its ticket. */
  asked: number;
  /** The ticket of the snapshot applied last: older answers are dropped. */
  applied: number;
}

/**
 * A numbered push held against snapshots that may answer late. A snapshot
 * asked for after the push arrived was read after it, so it already says
 * what the push did, and the mark goes when that snapshot is applied.
 */
export interface Mark {
  sequence: bigint;
  after: number;
}

export const NO_LOCATIONS: LiveLocations = {
  friends: {},
  online: {},
  removed: {},
  appeared: {},
  asked: 0,
  applied: 0,
};

/** A snapshot is asked for: its ticket is the new `asked`. */
export function askedForSnapshot(live: LiveLocations): LiveLocations {
  return { ...live, asked: live.asked + 1 };
}

/**
 * A snapshot of every visible friend, asked for with `ticket`: who is not in
 * it leaves the map. The answer can arrive after newer pushes, and after the
 * answer to a snapshot asked for later, which is newer and stands. Otherwise
 * a pin already held stays when it is newer than the snapshot's; someone
 * removed after the snapshot was read does not come back; and someone who
 * appeared after it was read stays. Marks from before the snapshot was asked
 * for are in it, so they are dropped, even when the server's count started
 * over since.
 */
export function withSnapshot(
  live: LiveLocations,
  snapshot: Pick<geo.ListFriendLocationsResponse, "locations" | "removalSequence">,
  ticket: number,
): LiveLocations {
  if (ticket < live.applied) return live;
  const listed = new Set(snapshot.locations.map((location) => location.userId));
  const since = snapshot.removalSequence;
  const removed = pending(live.removed, ticket);
  const appeared = pending(live.appeared, ticket);
  const after = (marks: Record<string, Mark>, userId: string) =>
    (marks[userId]?.sequence ?? 0n) > since;
  const held = (location: geo.FriendLocation) => ({
    location,
    online: live.online[location.userId] ?? location.online,
  });
  const kept = [
    ...snapshot.locations
      .filter((location) => !after(removed, location.userId))
      .map((location) => {
        const pin = live.friends[location.userId];
        return pin && supersedes(pin, location) ? held(pin) : { location, online: location.online };
      }),
    ...Object.values(live.friends)
      .filter((pin) => !listed.has(pin.userId) && after(appeared, pin.userId))
      .map(held),
  ];
  return {
    friends: Object.fromEntries(kept.map(({ location }) => [location.userId, location])),
    online: Object.fromEntries(
      kept.filter(({ online }) => online).map(({ location }) => [location.userId, true]),
    ),
    removed,
    appeared,
    asked: live.asked,
    applied: ticket,
  };
}

/** The marks a snapshot asked for with `ticket` may not say yet. */
function pending(marks: Record<string, Mark>, ticket: number): Record<string, Mark> {
  return Object.fromEntries(Object.entries(marks).filter(([, mark]) => mark.after >= ticket));
}

/**
 * Whether a held pin is the newer one. A blurred pin's time is rounded down,
 * so times only compare at the same precision; across precisions the blurred
 * pin stands, until the next push says otherwise.
 */
function supersedes(held: geo.FriendLocation, snapshot: geo.FriendLocation): boolean {
  if (held.precision !== snapshot.precision) return held.precision === geo.GhostMode.BLURRED;
  return toMillis(held.recordedAt) > toMillis(snapshot.recordedAt);
}

/**
 * A friend's new location (which also says whether they are online). Pushed
 * after any removal of them, so they are back on the map. `removalSequence`
 * is set when the pin shows someone who just became visible (a new friend).
 */
export function withLocation(
  live: LiveLocations,
  location: geo.FriendLocation,
  removalSequence = 0n,
): LiveLocations {
  const { [location.userId]: _back, ...removed } = live.removed;
  return {
    ...live,
    friends: { ...live.friends, [location.userId]: location },
    online: { ...live.online, [location.userId]: location.online },
    removed,
    appeared:
      removalSequence > 0n
        ? { ...live.appeared, [location.userId]: { sequence: removalSequence, after: live.asked } }
        : live.appeared,
  };
}

/**
 * A friend who stopped sharing, or is no longer a friend, leaves the map.
 * `removalSequence` is the removal's number when the server pushed one; a
 * removal without it (0) cannot be ordered against a snapshot.
 */
export function withoutFriend(
  live: LiveLocations,
  userId: string,
  removalSequence = 0n,
): LiveLocations {
  const { [userId]: _removed, ...friends } = live.friends;
  const { [userId]: _gone, ...appeared } = live.appeared;
  const last = live.removed[userId]?.sequence ?? 0n;
  const sequence = removalSequence > last ? removalSequence : last;
  return {
    ...live,
    friends,
    removed:
      removalSequence > 0n
        ? { ...live.removed, [userId]: { sequence, after: live.asked } }
        : live.removed,
    appeared,
  };
}

/**
 * Everything forgotten (the account changed), and the answers to snapshots
 * asked for before are dropped when they arrive.
 */
export function cleared(live: LiveLocations): LiveLocations {
  return { ...NO_LOCATIONS, asked: live.asked + 1, applied: live.asked + 1 };
}
