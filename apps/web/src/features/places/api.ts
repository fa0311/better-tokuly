import { type common, geo, social } from "@tokuly/api";
import { accountTimeZone, zonedDate } from "#/lib/account-time";
import { api } from "#/lib/api";

export const placeClient = () => api().service(geo.PlaceService);
export const locationClient = () => api().service(geo.LocationService);

/** Query keys of the places area. Everything about one place hangs under `placeKeys.place(id)`. */
export const placeKeys = {
  all: ["geo", "places"] as const,
  mine: ["geo", "places", "mine"] as const,
  saved: ["geo", "places", "saved"] as const,
  suggestions: ["geo", "places", "suggestions"] as const,
  place: (id: string) => ["geo", "places", "place", id] as const,
  search: (query: string, near: LatLng | null) =>
    ["geo", "places", "search", query, near?.lat ?? null, near?.lng ?? null] as const,
  reverse: (position: LatLng, language: string) =>
    [
      "geo",
      "reverse",
      Number(position.lat.toFixed(4)),
      Number(position.lng.toFixed(4)),
      language,
    ] as const,
};

export interface LatLng {
  lat: number;
  lng: number;
}

/** Shibuya crossing: where maps open before the device says where it is. */
export const DEFAULT_CENTER: LatLng = { lat: 35.6595, lng: 139.7005 };

export const toLatLng = (point: common.LatLng | undefined): LatLng | null =>
  point ? { lat: point.lat, lng: point.lng } : null;

/** The device's position, asked once. Rejects when the browser cannot or may not say. */
export function currentPosition(): Promise<LatLng> {
  return new Promise((resolve, reject) => {
    if (!("geolocation" in navigator)) {
      reject(new Error("geolocation unavailable"));
      return;
    }
    navigator.geolocation.getCurrentPosition(
      (result) => resolve({ lat: result.coords.latitude, lng: result.coords.longitude }),
      (error) => reject(error),
      { enableHighAccuracy: true, maximumAge: 30_000, timeout: 15_000 },
    );
  });
}

/**
 * The caller's friends, all pages. Uses the cache entry the live map keeps
 * (`["social", "friends"]`), so the two never ask twice.
 */
export async function loadFriends(): Promise<social.Friend[]> {
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
}

export const FRIENDS_KEY = ["social", "friends"] as const;

/**
 * `YYYY-MM-DD` of a moment on the account's calendar (its time zone, wherever
 * the device is): the form the history API and `/history/<date>` take.
 */
export function localDate(date: Date): string {
  return zonedDate(date.getTime(), accountTimeZone());
}
