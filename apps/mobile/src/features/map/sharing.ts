import { type Api, fromDate, geo } from "@tokuly/api";
import {
  type LastReport,
  locationSample,
  type OwnPosition,
  reportDecision,
} from "@tokuly/core/map";
import { useEffect } from "react";
import { AppState } from "react-native";
import { create } from "zustand";
import type { Account } from "#/features/session/account";
import { useMe } from "#/features/session/store";
import { api } from "#/lib/api";
import i18n from "#/lib/i18n";
import { realtime } from "#/lib/realtime";
import {
  currentPosition,
  type LocationAccess,
  readBattery,
  readLocationAccess,
  askBackgroundAccess as requestAlways,
  askForegroundAccess as requestWhileUsing,
  startBackgroundUpdates,
  stopBackgroundUpdates,
  watchForeground,
} from "#/platform/location";
import { readSecure, writeSecure } from "#/platform/secure-storage";
import { useLocations } from "./locations-store";

/**
 * What an account decided about sharing its position from this device. The
 * decision belongs to the account, not to the device: another account
 * signing in here starts undecided, whatever the OS permission says, and
 * nothing is sent for it until it says yes (the same rule as the web).
 */
export type SharingChoice = "on" | "off" | null;

export const choiceKey = (userId: string) => `tokuly.map.sharing.${userId}`;
const stopOwedKey = (userId: string) => `tokuly.map.stopOwed.${userId}`;
/** The account the background task sends for; gone the moment that account leaves. */
export const BACKGROUND_OWNER_KEY = "tokuly.map.backgroundOwner";

interface SharingStore {
  /** Whose choice this is; `loaded` once it was read from storage. */
  userId: string | null;
  loaded: boolean;
  choice: SharingChoice;
  access: LocationAccess | null;
  /**
   * Sharing stopped here, but the server has not confirmed it yet (the stop
   * is on its way, or it failed): friends may still see a live pin.
   * Remembered per account and retried until it works.
   */
  stopOwed: boolean;
  /** The account is leaving (signing out, another account taking over): nothing more goes out for it. */
  leaving: boolean;
}

export const useSharing = create<SharingStore>(() => ({
  userId: null,
  loaded: false,
  choice: null,
  access: null,
  stopOwed: false,
  leaving: false,
}));

/**
 * Whether `userId` still shares from here through `account`: the same API
 * (no switch happened since), the same account in the store, sharing on,
 * not leaving. Asked again after every wait, the way the web's
 * `stillSharing` is: a stop or a switch must not be followed by one more fix.
 */
function stillSharing(userId: string, account: Api): boolean {
  const state = useSharing.getState();
  return (
    api() === account &&
    state.userId === userId &&
    state.loaded &&
    state.choice === "on" &&
    !state.leaving
  );
}

/** When `userId` stopped sharing from here while the server has not heard of it yet (ms), or `null`. */
async function stopOwedSince(userId: string): Promise<number | null> {
  const since = Number(await readSecure(stopOwedKey(userId)));
  return since > 0 ? since : null;
}

async function setStopOwed(userId: string, since: number | null): Promise<void> {
  await writeSecure(stopOwedKey(userId), since === null ? null : String(since));
  if (useSharing.getState().userId === userId) useSharing.setState({ stopOwed: since !== null });
}

/**
 * Writes down that `userId` stopped sharing from here just now and the
 * server is owed the news — before anything is sent, so an app killed
 * mid-request still owes it at the next start. A stop already owed keeps
 * its time: nothing was shared from here since.
 */
async function oweStop(userId: string): Promise<void> {
  if ((await stopOwedSince(userId)) === null) await setStopOwed(userId, Date.now());
}

/**
 * Tells the server the stop `userId` owes it, with the time sharing
 * stopped (the device's clock, as in its samples: what was sent before that
 * moment never resumes sharing, what another device sent after it is not
 * undone by a late retry). Tried again until it goes through: the next
 * start, the app coming back to the screen, the retry button. Nothing owed
 * (sharing was turned on again meanwhile), nothing is sent. `account` is
 * the API of `userId`, taken before the caller's first wait: by the time
 * this runs the active one may be the next account's, and its sharing must
 * not be stopped in this one's place.
 */
async function tellOwedStop(userId: string, account: Api): Promise<boolean> {
  const since = await stopOwedSince(userId);
  if (since === null) return true;
  try {
    await account
      .service(geo.LocationService)
      .stopSharing({ stoppedAt: fromDate(new Date(since)) });
  } catch {
    return false;
  }
  // Unless sharing stopped once more while this one was on its way: that stop is still owed.
  if ((await stopOwedSince(userId)) === since) await setStopOwed(userId, null);
  return true;
}

/** Sharing stopped from here just now: owed first, then told. */
async function tellServerStopped(userId: string, account: Api): Promise<boolean> {
  await oweStop(userId);
  return tellOwedStop(userId, account);
}

/** Whether this device shares right now: the account said yes, the OS allows it, and it is not leaving. */
function sharesNow({ choice, access, leaving }: SharingStore): boolean {
  return choice === "on" && access?.foreground === "granted" && access.servicesEnabled && !leaving;
}

async function setChoice(userId: string, choice: SharingChoice): Promise<void> {
  // In memory first: a fix already on its way checks the store, not the storage.
  if (useSharing.getState().userId === userId) useSharing.setState({ choice });
  await writeSecure(choiceKey(userId), choice);
}

export async function refreshAccess(): Promise<void> {
  useSharing.setState({ access: await readLocationAccess() });
}

/** The account says yes: the OS asks for "while using the app" first, if it has not yet. */
export async function turnOn(): Promise<void> {
  const { userId } = useSharing.getState();
  if (!userId) return;
  const access = await requestWhileUsing();
  useSharing.setState({ access });
  if (access.foreground !== "granted") return;
  // Sharing again: a stop the server never heard of is void, and one on its way is not told again.
  await setStopOwed(userId, null);
  await setChoice(userId, "on");
}

/** The second step: "always", so friends keep getting fixes while the app is closed. */
export async function askAlways(): Promise<void> {
  useSharing.setState({ access: await requestAlways() });
}

/** Stops sharing from this device and tells the server, so no pin stays looking live. */
export async function stopSharing(): Promise<void> {
  const { userId } = useSharing.getState();
  if (!userId) return;
  const account = api();
  await setChoice(userId, "off");
  await oweStop(userId);
  await stopBackground();
  useLocations.getState().setOwn(null);
  await tellOwedStop(userId, account);
}

/** Tries the owed stop again (the retry button, the app coming back to the screen). */
export async function confirmStop(): Promise<void> {
  const { userId } = useSharing.getState();
  if (userId) await tellOwedStop(userId, api());
}

async function stopBackground(): Promise<void> {
  await writeSecure(BACKGROUND_OWNER_KEY, null);
  await stopBackgroundUpdates();
}

/** Whether this device is sharing right now, and if not, why. */
export type SharingState =
  | "loading"
  | "needsConsent"
  | "stopped"
  | "servicesOff"
  | "blocked"
  | "starting"
  | "sharing";

export function useSharingState(): SharingState {
  const { loaded, choice, access } = useSharing();
  const own = useLocations((state) => state.own);
  if (!loaded || !access) return "loading";
  if (choice === null) return "needsConsent";
  if (choice === "off") return "stopped";
  if (!access.servicesEnabled) return "servicesOff";
  // Never asked on this install (a reinstall keeps the account's yes, not the OS permission).
  if (access.foreground === "undetermined") return "needsConsent";
  if (access.foreground !== "granted") return "blocked";
  return own ? "sharing" : "starting";
}

/**
 * Sends one fix for `userId`: over the realtime connection when it is up,
 * the plain RPC otherwise, both those of the account active when it started.
 * Nothing goes out once that account stopped, left or was switched away from.
 */
export async function sendFix(position: OwnPosition, userId: string): Promise<void> {
  const account = api();
  const connection = realtime();
  if (!stillSharing(userId, account)) return;
  const battery = await readBattery();
  if (!stillSharing(userId, account)) return;
  const sample = locationSample(position, { ...battery, foreground: true });
  if (connection.sendLocation(sample)) return;
  try {
    await account.service(geo.LocationService).reportLocations({ samples: [sample] });
  } catch {
    // Dropping one sample is fine: the next one supersedes it.
  }
}

/**
 * Before an account leaves this device's screen (sign-out, a switch): nothing
 * more is sent for it from the foreground or the background, then the
 * server hears that it stopped — through the account's own API: the leave
 * may give up waiting (`LEAVE_TIMEOUT_MS`) while the background task is
 * still stopping, and the next account is the active one after that.
 */
export async function leaveSharing({ userId, api: account }: Account): Promise<void> {
  const state = useSharing.getState();
  const wasSharing = state.userId === userId && state.choice === "on";
  useSharing.setState({ leaving: true });
  useLocations.getState().setOwn(null);
  if (!wasSharing) return;
  await oweStop(userId);
  await stopBackground();
  await tellOwedStop(userId, account);
}

/** Every few seconds the newest fix goes out if its turn came; a still device gets a fresh one. */
const HEARTBEAT_MS = 5_000;

/**
 * While the app is on screen and the account shares: watches the position
 * and sends fixes at the pace the server asks for (often while moving,
 * rarely while still). Off screen, the background task takes over when
 * "always" is allowed.
 */
function useForegroundReporting(userId: string | null, active: boolean, precise: boolean): void {
  useEffect(() => {
    if (!active || !userId) return;
    const account = api();
    let live = true;
    const sharing = () => live && stillSharing(userId, account);
    let last: LastReport | null = null;
    let unsent: OwnPosition | null = null;
    let unwatch: (() => void) | null = null;

    const flush = (force = false) => {
      if (!sharing() || !unsent) return;
      const now = Date.now();
      const decision = reportDecision(unsent, last, realtime().cadence, now, force);
      if (decision === "wait") return;
      const position = unsent;
      unsent = null;
      if (decision === "same") {
        if (last) last.at = now;
        return;
      }
      last = { at: now, position };
      void sendFix(position, userId);
    };
    const accept = (position: OwnPosition, force = false) => {
      if (!sharing()) return;
      useLocations.getState().setOwn(position);
      unsent = position;
      flush(force);
    };

    const watch = async () => {
      if (unwatch || AppState.currentState !== "active") return;
      try {
        const stop = await watchForeground(precise, (position) => accept(position));
        if (live) unwatch = stop;
        else stop();
      } catch {
        void refreshAccess();
      }
    };
    const unwatchNow = () => {
      unwatch?.();
      unwatch = null;
    };

    const heartbeat = setInterval(() => {
      if (AppState.currentState !== "active") return;
      flush();
      if (!last || unsent) return;
      if (Date.now() - last.at < realtime().cadence.stationarySeconds * 1000) return;
      void currentPosition().then((position) => position && accept(position, true));
    }, HEARTBEAT_MS);

    const appState = AppState.addEventListener("change", (state) =>
      state === "active" ? void watch() : unwatchNow(),
    );
    void watch();
    return () => {
      live = false;
      unwatchNow();
      clearInterval(heartbeat);
      appState.remove();
    };
  }, [userId, active, precise]);
}

/**
 * The background task runs for the account that shares with "always" allowed;
 * it stops otherwise. `userId` is null until that account's choice is known:
 * nothing is started or stopped on a guess (no notification flashing by).
 */
function useBackgroundTask(userId: string | null, active: boolean): void {
  useEffect(() => {
    if (!userId) return;
    if (!active) {
      void stopBackground();
      return;
    }
    void (async () => {
      await writeSecure(BACKGROUND_OWNER_KEY, userId);
      try {
        await startBackgroundUpdates({
          title: i18n.t("mobile:sharing.notification.title"),
          body: i18n.t("mobile:sharing.notification.body"),
          color: "#7c5cff",
        });
      } catch {
        // Not allowed after all (changed in settings meanwhile): the next look at the access says so.
        void refreshAccess();
      }
    })();
  }, [userId, active]);
}

/**
 * Runs while someone is signed in: keeps the store on the signed-in
 * account's own choice and the OS permission, shares while both say yes,
 * and stops when either is taken back. Before the account leaves (sign-out,
 * another account), `leaveSharing` stops sending and tells the server
 * (`lifecycle.ts`).
 */
export function useOwnLocationSharing(): void {
  const myId = useMe()?.user?.id ?? null;
  const choice = useSharing((state) => state.choice);
  const access = useSharing((state) => state.access);

  useEffect(() => {
    useSharing.setState({
      userId: myId,
      loaded: false,
      choice: null,
      stopOwed: false,
      leaving: false,
    });
    if (!myId) return;
    const account = api();
    let current = true;
    void (async () => {
      const [stored, owed] = await Promise.all([readSecure(choiceKey(myId)), stopOwedSince(myId)]);
      // Read too late: the account is leaving, and its yes must not come back into the store.
      if (!current || useSharing.getState().leaving) return;
      useSharing.setState({
        loaded: true,
        choice: stored === "on" || stored === "off" ? stored : null,
        stopOwed: owed !== null,
      });
      await refreshAccess();
      // A stop the server never heard of: try again, unless this device shares again by now.
      const state = useSharing.getState();
      if (state.stopOwed && state.choice !== "on" && current && api() === account)
        void tellOwedStop(myId, account);
    })();
    // Back from the settings app: the permission may have changed.
    const appState = AppState.addEventListener("change", (state) => {
      if (state !== "active") return;
      void refreshAccess();
      const sharing = useSharing.getState();
      if (sharing.stopOwed && !sharesNow(sharing)) void confirmStop();
    });
    return () => {
      current = false;
      appState.remove();
    };
  }, [myId]);

  // The permission taken away while sharing is a stop: the server is told.
  const granted = access !== null && access.foreground === "granted" && access.servicesEnabled;
  const revoked = access !== null && !granted;
  useEffect(() => {
    if (!myId || choice !== "on" || !revoked) return;
    useLocations.getState().setOwn(null);
    void stopBackground();
    void tellServerStopped(myId, api());
  }, [myId, choice, revoked]);

  // Only once this account's own choice was read: right after a switch the store still holds
  // the previous account's yes for a render, and nothing may start on it for the new one.
  const decided = useSharing((state) => (state.loaded ? state.userId : null)) === myId;
  const leaving = useSharing((state) => state.leaving);
  const active = decided && Boolean(myId) && choice === "on" && granted && !leaving;
  // Sharing again (the account left while sharing and is back, the permission returned): a stop
  // still owed from before is void — told now, it would take the live pin away.
  useEffect(() => {
    if (myId && active && useSharing.getState().stopOwed) void setStopOwed(myId, null);
  }, [myId, active]);
  useForegroundReporting(myId, active, access?.approximate === false);
  // Nor before the OS permission was read: a start or a stop on a guess flashes the notification.
  useBackgroundTask(
    decided && access !== null ? myId : null,
    active && access?.background === true,
  );
}
