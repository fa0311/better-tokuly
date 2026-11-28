import { geo, toMillis } from "@tokuly/api";

/**
 * Friends' live locations as an app holds them: the latest location per user
 * ID (as the server lets the viewer see it), who has the app open, and who
 * was taken off the map by which removal. The edits below are what the
 * realtime connection's events do to them.
 */
export interface LiveLocations {
  friends: Record<string, geo.FriendLocation>;
  online: Record<string, boolean>;
  /** The number of the last removal of each user (`GeoEvent.removalSequence`). */
  removed: Record<string, bigint>;
}

export const NO_LOCATIONS: LiveLocations = { friends: {}, online: {}, removed: {} };

/**
 * A snapshot of every visible friend: who is not in it leaves the map. The
 * answer can arrive after newer pushes: a pin already held stays when it is
 * newer than the snapshot's, and someone removed after the snapshot was read
 * (a removal numbered above the snapshot's count) does not come back.
 */
export function withSnapshot(
  live: LiveLocations,
  snapshot: Pick<geo.ListFriendLocationsResponse, "locations" | "removalSequence">,
): LiveLocations {
  const kept = snapshot.locations
    .filter((location) => (live.removed[location.userId] ?? 0n) <= snapshot.removalSequence)
    .map((location) => {
      const held = live.friends[location.userId];
      return held && supersedes(held, location)
        ? { location: held, online: live.online[location.userId] ?? held.online }
        : { location, online: location.online };
    });
  return {
    friends: Object.fromEntries(kept.map(({ location }) => [location.userId, location])),
    online: Object.fromEntries(
      kept.filter(({ online }) => online).map(({ location }) => [location.userId, true]),
    ),
    removed: live.removed,
  };
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
 * after any removal of them, so they are back on the map.
 */
export function withLocation(live: LiveLocations, location: geo.FriendLocation): LiveLocations {
  const { [location.userId]: _back, ...removed } = live.removed;
  return {
    friends: { ...live.friends, [location.userId]: location },
    online: { ...live.online, [location.userId]: location.online },
    removed,
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
  const last = live.removed[userId] ?? 0n;
  return {
    ...live,
    friends,
    removed: removalSequence > last ? { ...live.removed, [userId]: removalSequence } : live.removed,
  };
}
