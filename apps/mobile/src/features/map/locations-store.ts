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

interface LocationsStore extends LiveLocations {
  /** This device's own position, while it shares. */
  own: OwnPosition | null;
  setSnapshot(snapshot: geo.ListFriendLocationsResponse): void;
  upsert(location: geo.FriendLocation): void;
  /** `removalSequence`: the number of the pushed removal, if it has one. */
  remove(userId: string, removalSequence?: bigint): void;
  setOnline(userId: string, online: boolean): void;
  /** Everything about friends and the own position, for the next account. */
  clear(): void;
  setOwn(position: OwnPosition | null): void;
}

/** Live state of the map (the rules of each edit are `@tokuly/core/map`). */
export const useLocations = create<LocationsStore>((set) => ({
  ...NO_LOCATIONS,
  own: null,
  setSnapshot: (snapshot) => set((state) => withSnapshot(state, snapshot)),
  upsert: (location) => set((state) => withLocation(state, location)),
  remove: (userId, removalSequence) =>
    set((state) => withoutFriend(state, userId, removalSequence)),
  setOnline: (userId, online) =>
    set((state) => ({ online: { ...state.online, [userId]: online } })),
  clear: () => set({ ...NO_LOCATIONS, own: null }),
  setOwn: (own) => set({ own }),
}));
