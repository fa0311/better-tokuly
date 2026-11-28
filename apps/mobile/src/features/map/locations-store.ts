import type { geo } from "@tokuly/api";
import {
  askedForSnapshot,
  cleared,
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
  /** Notes that a snapshot is asked for; returns the ticket its answer is applied with. */
  askSnapshot(): number;
  setSnapshot(snapshot: geo.ListFriendLocationsResponse, ticket: number): void;
  /** `removalSequence`: set when the pin shows someone who just became visible. */
  upsert(location: geo.FriendLocation, removalSequence?: bigint): void;
  /** `removalSequence`: the number of the pushed removal, if it has one. */
  remove(userId: string, removalSequence?: bigint): void;
  setOnline(userId: string, online: boolean): void;
  /** Everything about friends and the own position, for the next account. */
  clear(): void;
  setOwn(position: OwnPosition | null): void;
}

/** Live state of the map (the rules of each edit are `@tokuly/core/map`). */
export const useLocations = create<LocationsStore>((set, get) => ({
  ...NO_LOCATIONS,
  own: null,
  askSnapshot: () => {
    set(askedForSnapshot);
    return get().asked;
  },
  setSnapshot: (snapshot, ticket) => set((state) => withSnapshot(state, snapshot, ticket)),
  upsert: (location, removalSequence) =>
    set((state) => withLocation(state, location, removalSequence)),
  remove: (userId, removalSequence) =>
    set((state) => withoutFriend(state, userId, removalSequence)),
  setOnline: (userId, online) =>
    set((state) => ({ online: { ...state.online, [userId]: online } })),
  clear: () => set((state) => ({ ...cleared(state), own: null })),
  setOwn: (own) => set({ own }),
}));
