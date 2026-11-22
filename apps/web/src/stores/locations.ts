import type { geo } from "@tokuly/api";
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
  setSnapshot: (locations) =>
    set({
      friends: Object.fromEntries(locations.map((l) => [l.userId, l])),
      online: Object.fromEntries(locations.filter((l) => l.online).map((l) => [l.userId, true])),
    }),
  upsert: (location) =>
    set((state) => ({
      friends: { ...state.friends, [location.userId]: location },
      online: { ...state.online, [location.userId]: location.online },
    })),
  remove: (userId) =>
    set((state) => {
      const { [userId]: _removed, ...friends } = state.friends;
      return { friends };
    }),
  setOnline: (userId, online) =>
    set((state) => ({ online: { ...state.online, [userId]: online } })),
  setOwn: (own, error = null) => set({ own, ownError: error }),
}));
