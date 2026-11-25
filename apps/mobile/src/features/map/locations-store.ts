import type { geo } from "@tokuly/api";
import { fromSnapshot, type OwnPosition, withLocation, withoutFriend } from "@tokuly/core/map";
import { create } from "zustand";

interface LocationsStore {
  /** Friends' latest locations by user ID, as the server lets this account see them. */
  friends: Record<string, geo.FriendLocation>;
  /** Who has the app open right now. */
  online: Record<string, boolean>;
  /** This device's own position, while it shares. */
  own: OwnPosition | null;
  setSnapshot(locations: geo.FriendLocation[]): void;
  upsert(location: geo.FriendLocation): void;
  remove(userId: string): void;
  setOnline(userId: string, online: boolean): void;
  setOwn(position: OwnPosition | null): void;
}

/** Live state of the map (the rules of each edit are `@tokuly/core/map`). */
export const useLocations = create<LocationsStore>((set) => ({
  friends: {},
  online: {},
  own: null,
  setSnapshot: (locations) => set(fromSnapshot(locations)),
  upsert: (location) => set((state) => withLocation(state, location)),
  remove: (userId) => set((state) => ({ friends: withoutFriend(state.friends, userId) })),
  setOnline: (userId, online) =>
    set((state) => ({ online: { ...state.online, [userId]: online } })),
  setOwn: (own) => set({ own }),
}));
