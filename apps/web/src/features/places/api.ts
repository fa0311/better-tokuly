import { geo, social } from "@tokuly/api";
import type { LatLng } from "@tokuly/core/places";
import { accountTimeZone, zonedDate } from "#/lib/account-time";
import { api } from "#/lib/api";

export { DEFAULT_CENTER, type LatLng, placeKeys, toLatLng } from "@tokuly/core/places";

export const placeClient = () => api().service(geo.PlaceService);
export const locationClient = () => api().service(geo.LocationService);

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
