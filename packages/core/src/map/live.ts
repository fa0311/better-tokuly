import { geo, toMillis } from "@tokuly/api";

/**
 * Friends' live locations as an app holds them: the latest location per user
 * ID (as the server lets the viewer see it) and who has the app open. The
 * edits below are what the realtime connection's events do to them.
 */
export interface LiveLocations {
  friends: Record<string, geo.FriendLocation>;
  online: Record<string, boolean>;
}

/**
 * A snapshot of every visible friend: who is not in it leaves the map. The
 * answer can arrive after a newer pushed location for the same friend, so a
 * pin already held stays when it is newer than the snapshot's.
 */
export function withSnapshot(live: LiveLocations, locations: geo.FriendLocation[]): LiveLocations {
  const kept = locations.map((location) => {
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

/** A friend's new location (which also says whether they are online). */
export function withLocation(live: LiveLocations, location: geo.FriendLocation): LiveLocations {
  return {
    friends: { ...live.friends, [location.userId]: location },
    online: { ...live.online, [location.userId]: location.online },
  };
}

/** A friend who stopped sharing, or is no longer a friend, leaves the map. */
export function withoutFriend(
  friends: LiveLocations["friends"],
  userId: string,
): LiveLocations["friends"] {
  const { [userId]: _removed, ...rest } = friends;
  return rest;
}
