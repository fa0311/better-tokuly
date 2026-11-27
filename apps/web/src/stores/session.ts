import type { identity, SessionState } from "@tokuly/api";
import { create } from "zustand";
import { api } from "#/lib/api";
import { forgetSignedOutAccount } from "#/seams/sign-out";

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

/**
 * Set (readable, nothing secret in it) by the app's hand-off
 * (`/api/auth/handoff`) beside the refresh cookie: a fresh in-app web view
 * has no hint of its own, yet a session is waiting for it.
 */
const HANDOFF_HINT_COOKIE = "tokuly_session_hint";

/** Takes the hand-off's hint, if it is there (once: it is removed). */
function takeHandoffHint(): boolean {
  const present = document.cookie
    .split(";")
    .some((pair) => pair.trim().startsWith(`${HANDOFF_HINT_COOKIE}=`));
  // biome-ignore lint/suspicious/noDocumentCookie: a fixed value, and the Cookie Store API is not in every browser yet
  if (present) document.cookie = `${HANDOFF_HINT_COOKIE}=; Path=/; Max-Age=0; SameSite=Lax`;
  return present;
}

let started = false;

/**
 * The removal of what the last signed-out account left in this browser. It
 * loads every feature's list first, so it ends a moment after the session
 * does; `signOut` waits for it, or the next screen (or a closed tab) would
 * come before it.
 */
let forgetting: Promise<void> = Promise.resolve();

/**
 * Says that this browser has a session again, before a reload that must
 * resume it (another account on this device became the active one).
 */
export function rememberSession(): void {
  window.localStorage.setItem(SESSION_HINT_KEY, "1");
}

/** Tabs of this browser tell each other when the active account changed. */
const ACCOUNT_CHANNEL = "tokuly.account";
const ACCOUNT_CHANGED_KEY = "tokuly.accountChanged";
let announcing = false;

/**
 * Starts this tab over as whoever is active now. Nothing of the previous
 * account (cached lists, realtime subscriptions, an open screen) may go on
 * acting under the new one.
 */
function reloadAsActiveAccount(): void {
  window.localStorage.setItem(SESSION_HINT_KEY, "1");
  window.location.reload();
}

/**
 * Tells the other tabs that this browser's active account changed (a switch,
 * an added account, signing every account out), so they reload instead of
 * acting under an account their screen no longer shows.
 */
export function announceAccountChange(): void {
  // This tab moves on by itself; the message is for the others.
  announcing = true;
  try {
    const channel = new BroadcastChannel(ACCOUNT_CHANNEL);
    channel.postMessage("changed");
    channel.close();
  } catch {
    // No BroadcastChannel: the storage event below reaches the other tabs.
  }
  window.localStorage.setItem(ACCOUNT_CHANGED_KEY, String(Date.now()));
}

function listenForAccountChanges(): void {
  try {
    new BroadcastChannel(ACCOUNT_CHANNEL).onmessage = () => {
      if (!announcing) reloadAsActiveAccount();
    };
  } catch {
    window.addEventListener("storage", (event) => {
      if (event.key === ACCOUNT_CHANGED_KEY) reloadAsActiveAccount();
    });
  }
}

export function startSession(): void {
  if (started || typeof window === "undefined") return;
  started = true;
  const session = api().session;
  // A refresh that came back as another account: another tab switched.
  session.onAccountChange(() => reloadAsActiveAccount());
  listenForAccountChanges();
  session.subscribe((state) => {
    const previous = useSessionStore.getState().state;
    useSessionStore.setState({ state });
    if (state.status === "signedIn") window.localStorage.setItem(SESSION_HINT_KEY, "1");
    else if (state.status === "signedOut") window.localStorage.removeItem(SESSION_HINT_KEY);
    // Only an account that was signed in leaves: an offline start, signed out with its session
    // kept, keeps its drafts.
    if (previous.status === "signedIn" && state.status === "signedOut")
      forgetting = forgetSignedOutAccount().catch((error) =>
        console.error("sign-out: the cleanup could not run", error),
      );
  });
  const handedOff = takeHandoffHint();
  if (handedOff || window.localStorage.getItem(SESSION_HINT_KEY)) void session.restore();
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
 * Signs out, after giving the registered hooks their moment, and returns once
 * nothing of the account is left in this browser. Signing out always
 * completes: a hook that fails or hangs is left behind.
 */
export async function signOut(allSessions = false, allAccountsOnDevice = false): Promise<void> {
  await Promise.all([...beforeSignOut].map(runHook));
  await api().session.signOut(allSessions, allAccountsOnDevice);
  await forgetting;
  if (allAccountsOnDevice) announceAccountChange();
}
