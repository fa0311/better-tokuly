import { type QueryClient, useQuery, useQueryClient } from "@tanstack/react-query";
import { type common, geo, social, toMillis } from "@tokuly/api";
import { mapKeys } from "@tokuly/core/map";
import { rereadOnWelcome } from "@tokuly/core/query";
import { useEffect } from "react";
import { toast } from "sonner";
import { api } from "#/lib/api";
import i18n from "#/lib/i18n";
import { realtime } from "#/lib/realtime";
import { useLocations } from "#/stores/locations";
import { useMe } from "#/stores/session";
import { serverNow, setServerTime } from "./clock";
import { emitEffect } from "./effects";

export { useNow } from "./clock";

export const locationClient = () => api().service(geo.LocationService);
export const tripClient = () => api().service(geo.TripService);
export const safetyClient = () => api().service(geo.SafetyService);
export const placeClient = () => api().service(geo.PlaceService);
export const interactionClient = () => api().service(social.InteractionService);

// The map's keys live in `@tokuly/core/map`; the folder reads them from here.
export { mapKeys };

export type FriendUser = common.UserSummary;

const t = (key: string, options?: Record<string, unknown>) =>
  i18n.t(key, { ns: "map", ...options });

/** The caller's friends by user ID. */
export function useFriendsById(): Record<string, social.Friend> {
  const { data } = useQuery({
    queryKey: mapKeys.friends,
    queryFn: async () => {
      const friends: social.Friend[] = [];
      let cursor = "";
      do {
        const page = await api()
          .service(social.FriendService)
          .listFriends({ page: { limit: 100, cursor } });
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

/** What a friend is called here: the nickname I gave them, else their display name. */
export function friendName(
  friend: social.Friend | undefined,
  user: common.UserSummary | undefined,
): string {
  return friend?.nickname || user?.displayName || user?.handle || "";
}

export const useGhostSettings = () =>
  useQuery({
    queryKey: mapKeys.ghost,
    queryFn: () => locationClient().getGhostSettings({}),
    staleTime: 60_000,
  });

export const useGatherings = () =>
  useQuery({
    queryKey: mapKeys.gatherings,
    queryFn: async () => (await locationClient().listGatherings({})).gatherings,
    staleTime: 60_000,
  });

export const useActiveTrips = () =>
  useQuery({
    queryKey: mapKeys.trips,
    queryFn: async () => (await tripClient().listActiveTrips({})).trips,
    staleTime: 60_000,
  });

export const useActiveSos = () =>
  useQuery({
    queryKey: mapKeys.sos,
    queryFn: () => safetyClient().getActiveSos({}),
    staleTime: 60_000,
  });

export const usePlaceAlerts = () =>
  useQuery({
    queryKey: mapKeys.alerts,
    queryFn: () => safetyClient().listPlaceAlerts({}),
    staleTime: 60_000,
  });

export const useLiveShares = () =>
  useQuery({
    queryKey: mapKeys.shares,
    queryFn: async () => (await locationClient().listLiveShares({})).shares,
    staleTime: 30_000,
  });

export const useMapViews = () =>
  useQuery({
    queryKey: mapKeys.mapViews,
    queryFn: () => interactionClient().getMapViews({}),
    staleTime: 60_000,
  });

function upsertById<T extends { id: string }>(list: T[] | undefined, item: T): T[] {
  const rest = (list ?? []).filter((entry) => entry.id !== item.id);
  return [...rest, item];
}

function applyGeoEvent(
  queryClient: QueryClient,
  { event, removalSequence }: geo.GeoEvent,
  myId: string | undefined,
): void {
  const locations = useLocations.getState();
  switch (event.case) {
    case "location":
      locations.upsert(event.value, removalSequence);
      break;
    case "gatheringUpdated":
      queryClient.setQueryData<geo.Gathering[]>(mapKeys.gatherings, (list) =>
        upsertById(list, event.value),
      );
      break;
    case "gatheringEndedId":
      queryClient.setQueryData<geo.Gathering[]>(mapKeys.gatherings, (list) =>
        (list ?? []).filter((gathering) => gathering.id !== event.value),
      );
      break;
    case "tripUpdated": {
      const trip = event.value;
      const active = trip.status === geo.TripStatus.ACTIVE;
      queryClient.setQueryData<geo.Trip[]>(mapKeys.trips, (list) =>
        active ? upsertById(list, trip) : (list ?? []).filter((entry) => entry.id !== trip.id),
      );
      if (trip.status === geo.TripStatus.ARRIVED) {
        const mine = trip.user?.id === myId;
        toast.success(
          mine
            ? t("trips.arrivedMine", { place: tripDestinationName(trip) })
            : t("trips.arrivedFriend", { name: trip.user?.displayName }),
        );
        emitEffect({ kind: "celebrate", at: trip.destination?.resolved });
      }
      break;
    }
    case "placeAlertUpdated":
      void queryClient.invalidateQueries({ queryKey: mapKeys.alerts });
      break;
    case "placeAlertFired": {
      const alert = event.value.alert;
      void queryClient.invalidateQueries({ queryKey: mapKeys.alerts });
      if (alert) {
        const place = alertPlaceName(alert);
        toast(
          alert.trigger === geo.AlertTrigger.LEAVE
            ? t("safety.firedLeft", { name: alert.subject?.displayName, place })
            : t("safety.firedArrived", { name: alert.subject?.displayName, place }),
          { icon: "📍" },
        );
      }
      break;
    }
    case "sosUpdated": {
      const sos = event.value;
      void queryClient.invalidateQueries({ queryKey: mapKeys.sos });
      if (sos.user?.id !== myId) {
        if (sos.resolvedAt)
          toast(t("sos.friendResolved", { name: sos.user?.displayName }), { icon: "💚" });
        else
          toast.error(t("sos.friendTriggered", { name: sos.user?.displayName }), {
            duration: 15_000,
          });
      }
      break;
    }
    case "visit": {
      const visit = event.value;
      const name = friendNameById(queryClient, visit.userId);
      const place = visit.placeLabel
        ? t(`place.${PLACE_LABEL_KEYS[visit.placeLabel] ?? "custom"}`)
        : visit.placeName;
      if (name && place)
        toast(
          visit.arrived ? t("visit.arrived", { name, place }) : t("visit.left", { name, place }),
          { icon: visit.arrived ? "🏁" : "👋" },
        );
      break;
    }
    default:
      break;
  }
}

export const PLACE_LABEL_KEYS: Record<number, "home" | "work" | "school"> = {
  [geo.PlaceLabel.HOME]: "home",
  [geo.PlaceLabel.WORK]: "work",
  [geo.PlaceLabel.SCHOOL]: "school",
};

function friendNameById(queryClient: QueryClient, userId: string): string {
  const friend = queryClient
    .getQueryData<social.Friend[]>(mapKeys.friends)
    ?.find((entry) => entry.user?.id === userId);
  return friendName(friend, friend?.user);
}

/** The place an alert watches, in words. */
export function alertPlaceName(alert: geo.PlaceAlert): string {
  const label = PLACE_LABEL_KEYS[alert.subjectPlaceLabel];
  return label ? t(`place.${label}`) : alert.placeName || t("safety.thisSpot");
}

/** Where a trip is going, in words. */
export function tripDestinationName(trip: geo.Trip): string {
  return trip.destination?.name || t("trips.destination");
}

function applySocialEvent(
  queryClient: QueryClient,
  event: social.SocialEvent["event"],
  myId: string | undefined,
): void {
  switch (event.case) {
    case "mapReaction":
      emitEffect({ kind: "reaction", reaction: event.value, direction: "incoming" });
      break;
    case "knock":
      if (event.value.from)
        emitEffect({
          kind: "knock",
          from: event.value.from,
          count: Math.max(1, event.value.count),
        });
      break;
    case "buzz":
      if (event.value.from) emitEffect({ kind: "buzz", from: event.value.from });
      break;
    case "bump": {
      const other = event.value.participants.find((participant) => participant.id !== myId);
      if (other)
        emitEffect({
          kind: "bump",
          id: event.value.id,
          with: other,
          pairCount: event.value.pairCount,
          placeName: event.value.placeName,
        });
      void queryClient.invalidateQueries({ queryKey: ["social", "bumps"] });
      break;
    }
    case "mapView":
      queryClient.setQueryData<social.GetMapViewsResponse>(mapKeys.mapViews, (current) =>
        current ? { ...current, viewerCount: event.value.viewerCount24h } : current,
      );
      void queryClient.invalidateQueries({ queryKey: mapKeys.mapViews });
      break;
    case "friendRequestAccepted":
      void queryClient.invalidateQueries({ queryKey: mapKeys.friends });
      break;
    default:
      break;
  }
}

/**
 * Keeps everything the map shows in step with the server: one snapshot when
 * the connection (re)establishes, then the events pushed over the realtime
 * connection — pins, gatherings, trips, alerts, SOS, and the things friends
 * throw at each other.
 */
export function useMapSync(): void {
  const queryClient = useQueryClient();
  const myId = useMe()?.user?.id;
  useEffect(() => {
    const client = realtime();
    let cancelled = false;

    const loadSnapshot = async () => {
      const ticket = useLocations.getState().askSnapshot();
      try {
        const response = await locationClient().listFriendLocations({});
        if (cancelled) return;
        setServerTime(toMillis(response.serverTime));
        useLocations.getState().setSnapshot(response, ticket);
      } catch {
        // The live stream still delivers updates; the next reconnect retries.
      }
    };

    const offFrame = client.onFrame((frame) => {
      if (frame.topic) return; // Topic-scoped frames (a public share) belong to whoever subscribed.
      if (frame.body.case === "geo") applyGeoEvent(queryClient, frame.body.value, myId);
      else if (frame.body.case === "social")
        applySocialEvent(queryClient, frame.body.value.event, myId);
      else if (frame.body.case === "presence")
        useLocations.getState().setOnline(frame.body.value.userId, frame.body.value.online);
      // Anything missed while disconnected is covered by a fresh look.
      else if (frame.body.case === "welcome") void loadSnapshot();
    });
    // Pushed before the connection was up, or while it was down: an SOS must not wait for a refetch.
    const offWelcome = rereadOnWelcome(queryClient, client, [
      mapKeys.ghost,
      mapKeys.gatherings,
      mapKeys.trips,
      mapKeys.sos,
      mapKeys.alerts,
    ]);
    // Friends' pins are pushed only while the app is in front: back, look again.
    const offForeground = client.onForeground(() => void loadSnapshot());
    client.start();
    void loadSnapshot();
    return () => {
      cancelled = true;
      offFrame();
      offWelcome();
      offForeground();
    };
  }, [queryClient, myId]);
}

/**
 * Runs on every page while someone is signed in, so that a position never
 * stays in memory longer than its reason for being there:
 * - someone who stops being visible to me (unfriended, blocked, banned)
 *   leaves the store at once, whatever page is open;
 * - the store is emptied when the account signs out or another one signs in,
 *   so the next account never sees the previous one's friends.
 * It also keeps the server clock in step from the connection's welcome frame.
 */
export function useFriendPositionsHousekeeping(): void {
  const queryClient = useQueryClient();
  const myId = useMe()?.user?.id;
  useEffect(() => {
    const client = realtime();
    const forget = (userId: string, removalSequence?: bigint) => {
      useLocations.getState().remove(userId, removalSequence);
      void queryClient.invalidateQueries({ queryKey: mapKeys.friends });
    };
    const offFrame = client.onFrame((frame) => {
      if (frame.topic) return;
      if (frame.body.case === "welcome") setServerTime(toMillis(frame.body.value.serverTime));
      else if (frame.body.case === "geo" && frame.body.value.event.case === "locationRemovedUserId")
        forget(frame.body.value.event.value, frame.body.value.removalSequence);
      else if (
        frame.body.case === "social" &&
        frame.body.value.event.case === "friendRemovedUserId"
      )
        forget(frame.body.value.event.value);
    });
    client.start();
    return () => {
      offFrame();
      useLocations.getState().clear();
    };
  }, [queryClient, myId]);
}

/**
 * Asks the server for the ghost settings again the moment a timed rule ends
 * (by the server's clock), so the indicator and the pickers never show a
 * rule that is already over — or drop one early because this device's clock
 * runs ahead.
 */
export function useGhostExpiryRefetch(): void {
  const queryClient = useQueryClient();
  const { data } = useGhostSettings();
  const ends = [data?.everyone, ...Object.values(data?.overrides ?? {})]
    .map((setting) => toMillis(setting?.until))
    .filter((until) => until > serverNow());
  const next = ends.length ? Math.min(...ends) : 0;
  useEffect(() => {
    if (!next) return;
    const timer = setTimeout(
      () => void queryClient.invalidateQueries({ queryKey: mapKeys.ghost }),
      Math.min(Math.max(0, next - serverNow()) + 500, 2 ** 31 - 1),
    );
    return () => clearTimeout(timer);
  }, [next, queryClient]);
}
