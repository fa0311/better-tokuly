import { Code, ConnectError, identity } from "@tokuly/api";
import { api, apiWith, authClient, resetApi } from "#/lib/api";
import { prepareRealtime, resetRealtime } from "#/lib/realtime";
import { DEVICE_ID_KEY } from "#/platform/device";
import { forgetPreviousInstall } from "#/platform/install";
import { FEATURE_SECURE_KEYS, runBeforeEnd, runBeforeLeave } from "#/seams/lifecycle";
import { currentAccount } from "./account";
import { withParkedTokens } from "./parked";
import { useSessionStore } from "./store";
import {
  ACCOUNT_KEYS,
  activeTokenStore,
  addDeparted,
  departedTokenStore,
  departedTokens,
  departPreviousInstall,
  forgetDeparted,
  forgetParked,
  parkActive,
  parkedTokens,
  rotateParked,
} from "./vault";

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
 * one's accounts: they are ended on the server, best effort, and forgotten). Never leaves the session unknown, which would keep the
 * splash screen up for good: when the device's storage fails, the app starts
 * signed out.
 */
export async function startSession(): Promise<void> {
  try {
    await forgetPreviousInstall(
      [...ACCOUNT_KEYS, DEVICE_ID_KEY, ...FEATURE_SECURE_KEYS],
      departPreviousInstall,
    );
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

/** The account in use leaves (`#/seams/lifecycle`); nobody signed in, nobody leaves. */
async function leave(): Promise<void> {
  const account = currentAccount();
  if (account) await runBeforeLeave(account);
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
 * Signs the active account out of this device (and, with `allSessions`, out
 * of every device). Parked accounts stay and can be continued with. The app
 * is signed out at once; the session's refresh token is kept aside until the
 * server is known to have ended it (offline, the sign-out does not reach it):
 * see `settleDeparted`. `everywherePending`: every session was to end, and
 * the server has not done it yet — the person is told it will be.
 */
export async function signOut(allSessions = false): Promise<{ everywherePending: boolean }> {
  await leave();
  const sessionId = activeSessionId();
  const token = await activeTokenStore.get();
  if (sessionId && token)
    await withParkedTokens(() => addDeparted(sessionId, { token, allSessions }));
  await api().session.signOut(allSessions);
  await startOver(null);
  await settleDeparted();
  const pending = sessionId ? (await departedTokens())[sessionId] : undefined;
  return { everywherePending: Boolean(allSessions && pending) };
}

/**
 * Finishes the sessions that left this device while the server may not know
 * (`departedTokens`): each is resumed, the features take back what it left
 * on the server (`beforeEnd` of `#/seams/lifecycle`: its push registrations,
 * so its pushes do not reach the phone in someone else's hands) and it is
 * signed out on the server. A session the server already ended refuses to
 * resume and is simply forgotten; one that cannot be reached yet waits for
 * the next start. Runs in the queue of the device's tokens.
 */
export function settleDeparted(): Promise<void> {
  return withParkedTokens(async () => {
    for (const [sessionId, { allSessions }] of Object.entries(await departedTokens())) {
      try {
        const account = apiWith(departedTokenStore(sessionId));
        // Refused (already ended): its store forgets it. Unreachable: kept for next time.
        if (!(await account.session.restore())) continue;
        await runBeforeEnd(account);
        await account.service(identity.AuthService).signOut({ allSessions });
        await forgetDeparted(sessionId);
      } catch {
        // Offline again: the next start tries once more.
      }
    }
  });
}
