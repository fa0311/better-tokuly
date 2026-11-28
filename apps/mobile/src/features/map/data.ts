import { useQuery, useQueryClient } from "@tanstack/react-query";
import { geo, social } from "@tokuly/api";
import { mapKeys } from "@tokuly/core/map";
import { useEffect } from "react";
import { api } from "#/lib/api";
import { realtime } from "#/lib/realtime";
import { useLocations } from "./locations-store";

export const locationClient = () => api().service(geo.LocationService);
const friendClient = () => api().service(social.FriendService);

/** The caller's friends by user ID (every page of them). */
export function useFriendsById(): Record<string, social.Friend> {
  const { data } = useQuery({
    queryKey: mapKeys.friends,
    queryFn: async () => {
      const friends: social.Friend[] = [];
      let cursor = "";
      do {
        const page = await friendClient().listFriends({ page: { limit: 100, cursor } });
        friends.push(...page.friends);
        cursor = page.page?.nextCursor ?? "";
      } while (cursor);
      return friends;
    },
    staleTime: 60_000,
  });
  const byId: Record<string, social.Friend> = {};
  for (const friend of data ?? []) if (friend.user) byId[friend.user.id] = friend;
  return byId;
}

/** What a friend is called here: the nickname I gave them, else their display name. */
export function friendName(friend: social.Friend | undefined): string {
  return friend?.nickname || friend?.user?.displayName || friend?.user?.handle || "";
}

export const useGhostSettings = () =>
  useQuery({
    queryKey: mapKeys.ghost,
    queryFn: () => locationClient().getGhostSettings({}),
    staleTime: 60_000,
  });

/**
 * Keeps friends' pins in step with the server while someone is signed in:
 * a snapshot when the connection (re)establishes, then the pushed events.
 * Someone who stops being visible (unfriended, blocked, banned) leaves the
 * map at once; everything is dropped when the account changes.
 */
export function useMapSync(): void {
  const queryClient = useQueryClient();
  useEffect(() => {
    const client = realtime();
    let cancelled = false;
    const locations = useLocations.getState();

    const loadSnapshot = async () => {
      try {
        const response = await locationClient().listFriendLocations({});
        if (!cancelled) useLocations.getState().setSnapshot(response.locations);
      } catch {
        // The live stream still delivers updates; the next reconnect retries.
      }
    };
    const forget = (userId: string) => {
      useLocations.getState().remove(userId);
      void queryClient.invalidateQueries({ queryKey: mapKeys.friends });
    };

    let welcomed = false;
    const offFrame = client.onFrame((frame) => {
      if (frame.topic) return; // Topic-scoped frames belong to whoever subscribed.
      const body = frame.body;
      if (body.case === "welcome") {
        void loadSnapshot();
        if (welcomed) void queryClient.invalidateQueries({ queryKey: mapKeys.ghost });
        welcomed = true;
      } else if (body.case === "presence") {
        locations.setOnline(body.value.userId, body.value.online);
      } else if (body.case === "geo") {
        const event = body.value.event;
        if (event.case === "location") useLocations.getState().upsert(event.value);
        else if (event.case === "locationRemovedUserId") forget(event.value);
      } else if (body.case === "social") {
        const event = body.value.event;
        if (event.case === "friendRemovedUserId") forget(event.value);
        else if (event.case === "friendRequestAccepted")
          void queryClient.invalidateQueries({ queryKey: mapKeys.friends });
      }
    });
    // A new friend (whichever side accepted, from whichever screen): their pin may be there to see.
    let known: Set<string> | null = null;
    const offCache = queryClient.getQueryCache().subscribe((event) => {
      if (event.type !== "updated" || event.action.type !== "success") return;
      if (event.query.queryHash !== JSON.stringify(mapKeys.friends)) return;
      const ids = new Set((event.query.state.data as social.Friend[]).map((f) => f.user?.id ?? ""));
      const added = known !== null && [...ids].some((id) => !known?.has(id));
      known = ids;
      if (added) void loadSnapshot();
    });
    // Friends' pins are pushed only while the app is in front: back, look again.
    const offForeground = client.onForeground(() => void loadSnapshot());
    client.start();
    void loadSnapshot();
    return () => {
      cancelled = true;
      offFrame();
      offForeground();
      offCache();
      const store = useLocations.getState();
      store.setSnapshot([]);
      store.setOwn(null);
    };
  }, [queryClient]);
}
