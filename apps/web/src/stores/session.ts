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

type BeforeSignOut = () => Promise<void> | void;
const beforeSignOut = new Set<BeforeSignOut>();

/**
 * Registers something to finish while the session still exists (telling the
 * server that this device stopped sharing its location, for example).
 * Returns the function that takes it out again.
 */
export function onBeforeSignOut(hook: BeforeSignOut): () => void {
  beforeSignOut.add(hook);
  return () => beforeSignOut.delete(hook);
}

/** How long sign-out waits for a hook. */
const HOOK_TIMEOUT_MS = 3000;

/**
 * Runs one hook, never longer than the time limit and never throwing —
 * whether it fails at once, later, or not at all.
 */
function runHook(hook: BeforeSignOut): Promise<void> {
  return new Promise<void>((resolve) => {
    const timer = setTimeout(resolve, HOOK_TIMEOUT_MS);
    const done = () => {
      clearTimeout(timer);
      resolve();
    };
    try {
      Promise.resolve(hook()).then(done, done);
    } catch {
      done();
    }
  });
}

/**
 * Signs out, after giving the registered hooks their moment. Signing out
 * always completes: a hook that fails or hangs is left behind.
 */
export async function signOut(allSessions = false): Promise<void> {
  await Promise.all([...beforeSignOut].map(runHook));
  await api().session.signOut(allSessions);
}
