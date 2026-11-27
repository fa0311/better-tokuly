import { Code, ConnectError, identity } from "@tokuly/api";
import { api, apiWith, authClient, resetApi, settleApi } from "#/lib/api";
import { prepareRealtime, resetRealtime } from "#/lib/realtime";
import { DEVICE_ID_KEY } from "#/platform/device";
import { forgetPreviousInstall } from "#/platform/install";
import { FEATURE_SECURE_KEYS, runBeforeEnd, runBeforeLeave, runForget } from "#/seams/lifecycle";
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
  dropDepartedActive,
  forgetDeparted,
  forgetParked,
  parkActive,
  parkedTokens,
  rotateParked,
} from "./vault";

let unbind: (() => void) | null = null;

/** The store follows the active API's session; `nextGeneration`: it is another account's tree now. */
function bindStore(nextGeneration = false): void {
  unbind?.();
  const session = api().session;
  unbind = session.subscribe((state) => useSessionStore.setState({ state }));
  useSessionStore.setState((s) => ({
    state: session.state,
    generation: nextGeneration ? s.generation + 1 : s.generation,
  }));
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
    await dropDepartedActive();
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

/**
 * Starts the app over as the account of `response` (or signed out): a new API, connection and
 * cache. The screens change once, when the next account is known: its state and its generation
 * reach the store in one update, so the signed-in tree mounts once, as that account. A
 * signed-in tree on screen goes at once all the same: from here on `api()` speaks for the next
 * account, and nothing may pair it with the one that tree shows (`currentAccount()`).
 */
async function startOver(response: identity.AuthResponse | null): Promise<void> {
  resetRealtime();
  resetApi();
  runForget();
  unbind?.();
  unbind = null;
  const session = api().session;
  if (response && activeSessionId()) useSessionStore.setState({ state: { status: "unknown" } });
  if (!response || !(await session.adopt(response))) session.assumeSignedOut();
  bindStore(true);
}

/**
 * Parks the account in use. Its API is settled first: the token that is
 * parked is the one its session ended up with, not one a refresh is spending.
 */
function park(sessionId: string): Promise<void> {
  return settleApi(() => parkActive(sessionId));
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
    await withParkedTokens(() => park(current));
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
    await park(current);
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
 *
 * It takes its turn in the queue of the device's tokens, behind a switch
 * that may be on its way: the account it ends is the one in use when it was
 * asked. When a switch went through meanwhile, that account is parked by
 * then; it leaves the device all the same and the new one stays signed in.
 * Asked while nobody is signed in (an account is still being adopted), it
 * ends nobody: whoever comes next did not ask.
 */
export async function signOut(allSessions = false): Promise<{ everywherePending: boolean }> {
  const sessionId = activeSessionId();
  if (!sessionId) return { everywherePending: false };
  await withParkedTokens(async () => {
    if (activeSessionId() === sessionId) await signOutActive(sessionId, allSessions);
    else await departParked(sessionId, allSessions);
  });
  await settleDeparted();
  const pending = (await departedTokens())[sessionId];
  return { everywherePending: Boolean(allSessions && pending) };
}

/** The account in use leaves and the app starts over signed out. Runs in the queue of the device's tokens. */
async function signOutActive(sessionId: string, allSessions: boolean): Promise<void> {
  await leave();
  // The token set aside is the one the session ended up with (a refresh on its way lands first).
  await settleApi(async () => {
    const token = await activeTokenStore.get();
    if (token) await addDeparted(sessionId, { token, allSessions });
  });
  // Set aside, the account has left: the app is signed out even when the phone refuses to
  // empty the active store (the next start does not resume what is left there).
  try {
    await api().session.signOut(allSessions);
  } catch (error) {
    console.warn("Could not empty the active token store; signed out all the same.", error);
  }
  await startOver(null);
}

/**
 * A parked account leaves the device the way a signed-out one does: its
 * token goes aside as departed (before it is forgotten as parked, so it is
 * never lost), for `settleDeparted` to end it on the server.
 */
async function departParked(sessionId: string, allSessions: boolean): Promise<void> {
  const token = (await parkedTokens())[sessionId];
  if (token) await addDeparted(sessionId, { token, allSessions });
  await forgetParked(sessionId);
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
        // Refused (already ended): its store forgets it. Unreachable or rate-limited: kept for
        // next time (a restore would wait out the limit, holding up the device's tokens).
        if (!(await account.session.refresh())) continue;
        await runBeforeEnd(account);
        await account.service(identity.AuthService).signOut({ allSessions });
        await forgetDeparted(sessionId);
      } catch {
        // Offline again: the next start tries once more.
      }
    }
  });
}
