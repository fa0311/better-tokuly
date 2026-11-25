import type { geo } from "@tokuly/api";
import { fromSnapshot, withLocation, withoutFriend } from "@tokuly/core/map";
import { create } from "zustand";

/** The device's own position as last read from the browser. */
export interface OwnPosition {
  lat: number;
  lng: number;
  accuracyMeters: number;
  speedMps: number | null;
  headingDegrees: number | null;
  recordedAt: number;
}

interface LocationsStore {
  /** Friends' latest locations by user ID, as the server lets us see them. */
  friends: Record<string, geo.FriendLocation>;
  /** Who has the app open right now. */
  online: Record<string, boolean>;
  own: OwnPosition | null;
  /** Why the own position is unavailable, if it is. */
  ownError: "denied" | "unavailable" | null;
  setSnapshot(locations: geo.FriendLocation[]): void;
  upsert(location: geo.FriendLocation): void;
  remove(userId: string): void;
  setOnline(userId: string, online: boolean): void;
  setOwn(position: OwnPosition | null, error?: LocationsStore["ownError"]): void;
}

export const useLocations = create<LocationsStore>((set) => ({
  friends: {},
  online: {},
  own: null,
  ownError: null,
  setSnapshot: (locations) => set(fromSnapshot(locations)),
  upsert: (location) => set((state) => withLocation(state, location)),
  remove: (userId) => set((state) => ({ friends: withoutFriend(state.friends, userId) })),
  setOnline: (userId, online) =>
    set((state) => ({ online: { ...state.online, [userId]: online } })),
  setOwn: (own, error = null) => set({ own, ownError: error }),
}));
