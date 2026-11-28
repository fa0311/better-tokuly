import type { geo } from "@tokuly/api";
import {
  type LiveLocations,
  NO_LOCATIONS,
  type OwnPosition,
  withLocation,
  withoutFriend,
  withSnapshot,
} from "@tokuly/core/map";
import { create } from "zustand";

export type { OwnPosition } from "@tokuly/core/map";

interface LocationsStore extends LiveLocations {
  own: OwnPosition | null;
  /** Why the own position is unavailable, if it is. */
  ownError: "denied" | "unavailable" | null;
  setSnapshot(snapshot: geo.ListFriendLocationsResponse): void;
  upsert(location: geo.FriendLocation): void;
  /** `removalSequence`: the number of the pushed removal, if it has one. */
  remove(userId: string, removalSequence?: bigint): void;
  setOnline(userId: string, online: boolean): void;
  /** Everything about friends and the own position, for the next account. */
  clear(): void;
  setOwn(position: OwnPosition | null, error?: LocationsStore["ownError"]): void;
}

export const useLocations = create<LocationsStore>((set) => ({
  ...NO_LOCATIONS,
  own: null,
  ownError: null,
  setSnapshot: (snapshot) => set((state) => withSnapshot(state, snapshot)),
  upsert: (location) => set((state) => withLocation(state, location)),
  remove: (userId, removalSequence) =>
    set((state) => withoutFriend(state, userId, removalSequence)),
  setOnline: (userId, online) =>
    set((state) => ({ online: { ...state.online, [userId]: online } })),
  clear: () => set({ ...NO_LOCATIONS, own: null, ownError: null }),
  setOwn: (own, error = null) => set({ own, ownError: error }),
}));
