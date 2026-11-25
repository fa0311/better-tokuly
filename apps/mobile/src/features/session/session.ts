import { type Api, Code, ConnectError, identity, notify, type SessionState } from "@tokuly/api";
import { create } from "zustand";
import { api, apiWith, authClient, resetApi } from "#/lib/api";
import { prepareRealtime, resetRealtime } from "#/lib/realtime";
import { DEVICE_ID_KEY } from "#/platform/device";
import { forgetPreviousInstall } from "#/platform/install";
import {
  ACCOUNT_KEYS,
  activeTokenStore,
  addDeparted,
  departedTokenStore,
  departedTokens,
  forgetDeparted,
  forgetParked,
  parkActive,
  parkedTokenStore,
  parkedTokens,
  rotateParked,
} from "./vault";

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
 * one's accounts). Never leaves the session unknown, which would keep the
 * splash screen up for good: when the device's storage fails, the app starts
 * signed out.
 */
export async function startSession(): Promise<void> {
  try {
    await forgetPreviousInstall([...ACCOUNT_KEYS, DEVICE_ID_KEY]);
    await prepareRealtime();
    bindStore();
    if (await activeTokenStore.get()) await api().session.restore();
    else api().session.assumeSignedOut();
    void settleDeparted();
  } catch (error) {
    // Not knowing whose tokens storage holds (this install's or a previous one's), nobody is signed in by itself.
    console.warn("Could not resume the session; starting signed out.", error);
    if (!unbind) bindStore();
    api().session.assumeSignedOut();
  }
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
    await withParkedTokens(() => parkActive(current));
  }
  await startOver(response);
}

/**
 * Everything that reads or rotates a parked account's refresh token (a
 * switch, a removal, parking an account, work done as a parked session) runs
 * one after the other: a token sent twice — a switch and a refresh of the
 * same session at once — revokes that session (REFRESH_TOKEN_REUSE).
 */
let parkedQueue: Promise<unknown> = Promise.resolve();

function withParkedTokens<T>(work: () => Promise<T>): Promise<T> {
  const run = parkedQueue.then(work);
  parkedQueue = run.catch(() => {});
  return run;
}

/**
 * Runs `work` as a parked account's session without making it the active
 * one (what it still owes the server, such as removing a push registration
 * a switch could not remove). Resolves to `null` when the device no longer
 * holds that account.
 */
export function asParked<T>(
  sessionId: string,
  work: (account: Api) => Promise<T>,
): Promise<T | null> {
  return withParkedTokens(async () => {
    if (!(await parkedTokens())[sessionId]) return null;
    const account = apiWith(parkedTokenStore(sessionId));
    if (!(await account.session.restore()))
      throw new Error("the parked session could not be resumed");
    return work(account);
  });
}

export type SwitchResult = "switched" | "needsCode";

/**
 * Makes a parked account the active one. An account with two-factor
 * authentication may answer `needsCode`: ask for it and call again.
 */
export function switchAccount(sessionId: string, totpCode = ""): Promise<SwitchResult> {
  return withParkedTokens(() => switchTo(sessionId, totpCode));
}

async function switchTo(sessionId: string, totpCode: string): Promise<SwitchResult> {
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
  // The switch rotated the parked account's token: it is kept before anything
  // else may take time (leave hooks wait up to 3 s), or the app dying meanwhile
  // would leave the spent one behind and the next switch would get the session revoked.
  await rotateParked(sessionId, response.refreshToken);
  if (current) {
    await leave();
    await parkActive(current);
  }
  await forgetParked(sessionId);
  await startOver(response);
  return "switched";
}

/** Signs a parked account out of this device; the others stay. */
export function removeParkedAccount(sessionId: string): Promise<void> {
  return withParkedTokens(async () => {
    const token = (await parkedTokens())[sessionId];
    if (token) await authClient().removeDeviceAccount({ refreshToken: token });
    await forgetParked(sessionId);
  });
}

/**
 * Signs the active account out of this device. Parked accounts stay and can
 * be continued with. The app is signed out at once; the session's refresh
 * token is kept aside until the server is known to have ended it (offline,
 * the sign-out does not reach it): see `settleDeparted`.
 */
export async function signOut(allSessions = false): Promise<void> {
  await leave();
  const sessionId = activeSessionId();
  const token = await activeTokenStore.get();
  if (sessionId && token) await withParkedTokens(() => addDeparted(sessionId, token));
  await api().session.signOut(allSessions);
  await startOver(null);
  void settleDeparted();
}

/**
 * Finishes the sessions that left this device while the server may not know
 * (`departedTokens`): each is resumed, its push registrations are removed
 * (its pushes must not reach the phone in someone else's hands) and it is
 * signed out on the server. A session the server already ended refuses to
 * resume and is simply forgotten; one that cannot be reached yet waits for
 * the next start. Runs in the queue of the device's tokens.
 */
export function settleDeparted(): Promise<void> {
  return withParkedTokens(async () => {
    for (const sessionId of Object.keys(await departedTokens())) {
      try {
        const account = apiWith(departedTokenStore(sessionId));
        // Refused (already ended): its store forgets it. Unreachable: kept for next time.
        if (!(await account.session.restore())) continue;
        await account.service(notify.PushService).unregisterDevice({ deviceId: "" });
        await account.service(identity.AuthService).signOut({});
        await forgetDeparted(sessionId);
      } catch {
        // Offline again: the next start tries once more.
      }
    }
  });
}
