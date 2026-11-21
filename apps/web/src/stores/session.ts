import type { identity, SessionState } from "@tokuly/api";
import { create } from "zustand";
import { api } from "#/lib/api";

interface SessionStore {
  state: SessionState;
}

/**
 * React-facing mirror of the API session. `startSession` wires it up once on
 * the client and restores the previous sign-in from the refresh cookie.
 */
export const useSessionStore = create<SessionStore>(() => ({ state: { status: "unknown" } }));

/**
 * Remembers that this browser has (or had) a session. The refresh token
 * itself is in an HttpOnly cookie we cannot see, so without this hint every
 * signed-out visit would cost a failing refresh request.
 */
const SESSION_HINT_KEY = "tokuly.signedIn";

let started = false;

export function startSession(): void {
  if (started || typeof window === "undefined") return;
  started = true;
  const session = api().session;
  session.subscribe((state) => {
    useSessionStore.setState({ state });
    if (state.status === "signedIn") window.localStorage.setItem(SESSION_HINT_KEY, "1");
    else if (state.status === "signedOut") window.localStorage.removeItem(SESSION_HINT_KEY);
  });
  if (window.localStorage.getItem(SESSION_HINT_KEY)) void session.restore();
  else session.assumeSignedOut();
}

/** The signed-in account, or `null` while unknown / signed out. */
export function useMe(): identity.Me | null {
  return useSessionStore((s) => (s.state.status === "signedIn" ? s.state.me : null));
}

export function useSessionStatus(): SessionState["status"] {
  return useSessionStore((s) => s.state.status);
}
