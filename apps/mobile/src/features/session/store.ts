import type { identity, SessionState } from "@tokuly/api";
import { create } from "zustand";

interface SessionStore {
  state: SessionState;
  /**
   * Goes up whenever another account becomes the active one (or the active
   * one signs out). The signed-in tree is keyed by it: caches, realtime
   * subscriptions and the navigation stack of the previous account are gone,
   * the way a web page reloads.
   */
  generation: number;
}

/** React-facing mirror of the active account's session. */
export const useSessionStore = create<SessionStore>(() => ({
  state: { status: "unknown" },
  generation: 0,
}));

/** The signed-in account, or `null` while unknown / signed out. */
export function useMe(): identity.Me | null {
  return useSessionStore((s) => (s.state.status === "signedIn" ? s.state.me : null));
}
