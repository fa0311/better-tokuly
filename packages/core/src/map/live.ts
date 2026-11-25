import type { geo } from "@tokuly/api";

/**
 * Friends' live locations as an app holds them: the latest location per user
 * ID (as the server lets the viewer see it) and who has the app open. The
 * edits below are what the realtime connection's events do to them.
 */
export interface LiveLocations {
  friends: Record<string, geo.FriendLocation>;
  online: Record<string, boolean>;
}

/** The whole picture from a snapshot of every visible friend. */
export function fromSnapshot(locations: geo.FriendLocation[]): LiveLocations {
  return {
    friends: Object.fromEntries(locations.map((location) => [location.userId, location])),
    online: Object.fromEntries(
      locations.filter((location) => location.online).map((location) => [location.userId, true]),
    ),
  };
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
