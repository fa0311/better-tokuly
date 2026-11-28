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

export type { OwnPosition } from "@tokuly/core/map";

interface LocationsStore extends LiveLocations {
  own: OwnPosition | null;
  /** Why the own position is unavailable, if it is. */
  ownError: "denied" | "unavailable" | null;
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
  setOwn(position: OwnPosition | null, error?: LocationsStore["ownError"]): void;
}

export const useLocations = create<LocationsStore>((set, get) => ({
  ...NO_LOCATIONS,
  own: null,
  ownError: null,
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
  clear: () => set((state) => ({ ...cleared(state), own: null, ownError: null })),
  setOwn: (own, error = null) => set({ own, ownError: error }),
}));
