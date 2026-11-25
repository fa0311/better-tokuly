import { Code, ConnectError, type identity, type SessionState } from "@tokuly/api";
import { create } from "zustand";
import { api, authClient, resetApi } from "#/lib/api";
import { prepareRealtime, resetRealtime } from "#/lib/realtime";
import { DEVICE_ID_KEY } from "#/platform/device";
import { forgetPreviousInstall } from "#/platform/install";
import { ACCOUNT_KEYS, activeTokenStore, forgetParked, parkActive, parkedTokens } from "./vault";

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

let unbind: (() => void) | null = null;

function bindStore(): void {
  unbind?.();
  const session = api().session;
  unbind = session.subscribe((state) => useSessionStore.setState({ state }));
  useSessionStore.setState({ state: session.state });
}

/**
 * On app start: resumes the active account from the refresh token in secure
 * storage, unless this is a new install (its keychain may hold a previous
 * one's accounts).
 */
export async function startSession(): Promise<void> {
  await forgetPreviousInstall([...ACCOUNT_KEYS, DEVICE_ID_KEY]);
  await prepareRealtime();
  bindStore();
  if (await activeTokenStore.get()) await api().session.restore();
  else api().session.assumeSignedOut();
}

/** The signed-in account, or `null` while unknown / signed out. */
export function useMe(): identity.Me | null {
  return useSessionStore((s) => (s.state.status === "signedIn" ? s.state.me : null));
}

type BeforeLeave = () => Promise<void> | void;
const beforeLeave = new Set<BeforeLeave>();

/**
 * Registers something to finish while the active account's session still
 * works, before it signs out or another account takes over (telling the
 * server that this device stopped sharing its location). Returns the
 * function that takes it out again.
 */
export function onBeforeLeave(hook: BeforeLeave): () => void {
  beforeLeave.add(hook);
  return () => beforeLeave.delete(hook);
}

const HOOK_TIMEOUT_MS = 3000;

/** Every hook gets its moment; one that fails or hangs is left behind. */
function leave(): Promise<void> {
  const run = (hook: BeforeLeave) =>
    new Promise<void>((resolve) => {
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
  return Promise.all([...beforeLeave].map(run)).then(() => undefined);
}

/** Starts the app over as the account of `response` (or signed out): a new API, connection and cache. */
async function startOver(response: identity.AuthResponse | null): Promise<void> {
  resetRealtime();
  resetApi();
  bindStore();
  if (!response || !(await api().session.adopt(response))) api().session.assumeSignedOut();
  useSessionStore.setState((s) => ({ generation: s.generation + 1 }));
}

function activeSessionId(): string | null {
  const { state } = useSessionStore.getState();
  return state.status === "signedIn" ? state.sessionId : null;
}

/**
 * Enters the account a sign-in or sign-up answered with. Signed in already
 * (`add_account`), the account in use is parked beside it.
 */
export async function enterAccount(response: identity.AuthResponse): Promise<void> {
  const current = activeSessionId();
  if (current) {
    await leave();
    await parkActive(current);
  }
  await startOver(response);
}

export type SwitchResult = "switched" | "needsCode";

/**
 * Makes a parked account the active one. An account with two-factor
 * authentication may answer `needsCode`: ask for it and call again.
 */
export async function switchAccount(sessionId: string, totpCode = ""): Promise<SwitchResult> {
  const token = (await parkedTokens())[sessionId];
  if (!token) {
    await forgetParked(sessionId);
    throw new ConnectError("that account is not on this device", Code.NotFound, {
      "x-tokuly-error": "ACCOUNT_NOT_ON_DEVICE",
    });
  }
  const current = activeSessionId();
  // A wrong code, a cancel or a network failure leaves the account in use as it was:
  // it leaves (stops sharing, tells the server) only once the switch went through.
  const response = await authClient().switchAccount({ refreshToken: token, totpCode });
  if (response.mfaRequired) return "needsCode";
  if (current) {
    await leave();
    await parkActive(current);
  }
  await forgetParked(sessionId);
  await startOver(response);
  return "switched";
}

/** Signs a parked account out of this device; the others stay. */
export async function removeParkedAccount(sessionId: string): Promise<void> {
  const token = (await parkedTokens())[sessionId];
  if (token) await authClient().removeDeviceAccount({ refreshToken: token });
  await forgetParked(sessionId);
}

/** Signs the active account out of this device. Parked accounts stay and can be continued with. */
export async function signOut(allSessions = false): Promise<void> {
  await leave();
  await api().session.signOut(allSessions);
  await startOver(null);
}
