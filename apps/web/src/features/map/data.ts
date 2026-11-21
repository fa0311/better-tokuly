import { useQuery } from "@tanstack/react-query";
import { type common, geo, social } from "@tokuly/api";
import { useEffect } from "react";
import { api } from "#/lib/api";
import { realtime } from "#/lib/realtime";
import { useLocations } from "#/stores/locations";

/**
 * Keeps the locations store in sync with the server: one snapshot when the
 * connection (re)establishes, then live updates pushed over the realtime
 * connection.
 */
export function useFriendLocationsSync(): void {
  useEffect(() => {
    const client = realtime();
    const store = useLocations.getState();
    let cancelled = false;

    const loadSnapshot = async () => {
      try {
        const response = await api().service(geo.LocationService).listFriendLocations({});
        if (!cancelled) store.setSnapshot(response.locations);
      } catch {
        // The live stream still delivers updates; the next reconnect retries.
      }
    };

    const offFrame = client.onFrame((frame) => {
      if (frame.body.case === "geo") {
        const event = frame.body.value.event;
        if (event.case === "location") store.upsert(event.value);
        else if (event.case === "locationRemovedUserId") store.remove(event.value);
      } else if (frame.body.case === "presence") {
        store.setOnline(frame.body.value.userId, frame.body.value.online);
      } else if (frame.body.case === "welcome") {
        // Anything missed while disconnected is covered by a fresh snapshot.
        void loadSnapshot();
      }
    });
    client.start();
    void loadSnapshot();
    return () => {
      cancelled = true;
      offFrame();
    };
  }, []);
}

/** The caller's friends by user ID. */
export function useFriendsById(): Record<string, social.Friend> {
  const { data } = useQuery({
    queryKey: ["social", "friends"],
    queryFn: async () => {
      const friends: social.Friend[] = [];
      let cursor = "";
      do {
        const page = await api().service(social.FriendService).listFriends({ page: { limit: 100, cursor } });
        friends.push(...page.friends);
        cursor = page.page?.nextCursor ?? "";
      } while (cursor);
      return friends;
    },
    staleTime: 60_000,
  });
  const byId: Record<string, social.Friend> = {};
  for (const friend of data ?? []) {
    if (friend.user) byId[friend.user.id] = friend;
  }
  return byId;
}

export type FriendUser = common.UserSummary;
