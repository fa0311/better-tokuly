import { fromDate, type geo } from "@tokuly/api";
import {
  finiteOrUndefined,
  type LastReport,
  locationSample,
  nextSequence,
  type OwnPosition,
  reportDecision,
} from "@tokuly/core/map";
import { useEffect } from "react";
import { toast } from "sonner";
import { create as createStore } from "zustand";
import { errorMessage } from "#/lib/errors";
import i18n from "#/lib/i18n";
import { realtime } from "#/lib/realtime";
import { useLocations } from "#/stores/locations";
import { onBeforeSignOut, useMe } from "#/stores/session";
import { locationClient } from "./data";

/** Other tabs hear about a stop through a storage event; give them this long before the server is told. */
const OTHER_TABS_MS = 300;
/**
 * The server takes nearby discovery and bumps only from fixes this good
 * (`tokuly_geo::claims`): an approximate location is not enough.
 */
export const PRECISE_ENOUGH_METERS = 100;

/** Why the server would refuse a claim, as far as the client can tell before asking. */
export type ClaimProblem = "LOCATION_NOT_VERIFIED" | "LOCATION_TOO_COARSE";

/** The refusal reasons nearby discovery and bumps can answer with. */
export function claimProblem(reason: string | undefined): ClaimProblem | null {
  return reason === "LOCATION_NOT_VERIFIED" || reason === "LOCATION_TOO_COARSE" ? reason : null;
}

/** A reading this recent counts as a fresh fix for nearby discovery and bumps. */
const FRESH_FIX_MS = 10_000;
/** The app's "someone is signed in here" marker (`stores/session.ts`): it goes when any tab signs out. */
const SIGNED_IN_KEY = "tokuly.signedIn";
/** Written by the tab that is signing out, a moment before the session goes: other tabs stop sending at once. */
const SESSION_ENDING_KEY = "tokuly.map.sessionEnding";
const SESSION_ENDING_MS = 15_000;

/**
 * What an account decided about sharing its position from this browser.
 * The decision belongs to the account, not to the browser: someone else
 * signing in here starts undecided, whatever the browser's own location
 * permission says, and nothing is sent for them until they say yes.
 */
export type SharingChoice = "on" | "off" | null;

const choiceKey = (userId: string) => `tokuly.map.sharing.${userId}`;
const stopOwedKey = (userId: string) => `tokuly.map.stopOwed.${userId}`;
/**
 * The account's running count in this browser (`nextSequence`), shared by its
 * tabs. Per account, as its choice is: the count says nothing about who else
 * signs in here.
 */
const sequenceKey = (userId: string) => `tokuly.map.sequence.${userId}`;

/** What could not be written to storage (private windows, full or blocked storage): kept for this page. */
const memory = new Map<string, string>();

function readStored(key: string): string | null {
  try {
    return window.localStorage.getItem(key) ?? memory.get(key) ?? null;
  } catch {
    return memory.get(key) ?? null;
  }
}

/** Returns whether the value will still be there on the next visit. */
function writeStored(key: string, value: string | null): boolean {
  try {
    if (value === null) window.localStorage.removeItem(key);
    else window.localStorage.setItem(key, value);
    memory.delete(key);
    return true;
  } catch {
    if (value === null) memory.delete(key);
    else memory.set(key, value);
    return false;
  }
}

/** The latest number of each account on this page: storage may be cleared under it. */
const lastSequence = new Map<string, number>();

/**
 * The next running number of `userId` in this browser, for a sample or a
 * stop. Taken right where the last check that sharing is on was made, with
 * no wait in between: a stop then gets a number above every sample sent
 * before it, from any tab, whatever the clock does.
 */
function takeSequence(userId: string): bigint {
  const stored = Number(readStored(sequenceKey(userId))) || 0;
  const latest = Math.max(lastSequence.get(userId) ?? 0, stored);
  const next = nextSequence(latest, Date.now());
  lastSequence.set(userId, next);
  writeStored(sequenceKey(userId), String(next));
  return BigInt(next);
}

function readChoice(userId: string | null): SharingChoice {
  if (!userId) return null;
  const value = readStored(choiceKey(userId));
  return value === "on" || value === "off" ? value : null;
}

/** Whether a session exists in this browser: false from the moment any tab starts signing out. */
function sessionAlive(): boolean {
  try {
    if (window.localStorage.getItem(SIGNED_IN_KEY) !== "1") return false;
    const ending = Number(window.localStorage.getItem(SESSION_ENDING_KEY) ?? 0);
    return !(ending > 0 && Date.now() - ending < SESSION_ENDING_MS);
  } catch {
    // Without storage there are no other tabs to hear from.
    return true;
  }
}

interface SharingStore {
  /** Whose choice this is. */
  userId: string | null;
  choice: SharingChoice;
  /** What the browser says about geolocation for this site. */
  permission: "unknown" | "prompt" | "granted" | "denied" | "unsupported";
  /** The person tapped "turn on" on this page, so the browser may show its prompt. */
  asked: boolean;
  /** Goes up with every "turn on" / "try again", so a stuck attempt starts over. */
  attempt: number;
  /**
   * Sharing stopped here, but the server has not been told yet: friends may
   * still see a live pin. Remembered per account and retried until it works.
   */
  stopFailed: boolean;
  /** The account signed out in another tab: this tab sends nothing more. */
  sessionEnded: boolean;
  setUser(userId: string | null): void;
  setPermission(permission: SharingStore["permission"]): void;
  /** The account says yes (or tries again): sharing starts, and the browser may ask. */
  start(): void;
  /** Stops sharing from every tab of this browser and tells the server. */
  stop(): Promise<void>;
  /** Tells the server again after a failed stop. */
  confirmStop(): Promise<void>;
}

const t = (key: string) => i18n.t(key, { ns: "map" });

function setStopOwed(userId: string | null, owed: boolean): void {
  if (userId) writeStored(stopOwedKey(userId), owed ? "1" : null);
  if (useSharing.getState().userId === userId) useSharing.setState({ stopFailed: owed });
}

/**
 * Tells the server that this account stopped sharing from here. A failure is
 * never dropped: it is remembered for the account, shown for as long as it
 * stands, and tried again (back online, next visit, the retry button).
 */
async function tellServerStopped(quiet = false): Promise<boolean> {
  const userId = useSharing.getState().userId;
  try {
    // The device's clock and count, as in its samples: what it sent before this moment never resumes sharing.
    await locationClient().stopSharing({
      stoppedAt: fromDate(new Date()),
      sequence: userId ? takeSequence(userId) : 0n,
    });
    setStopOwed(userId, false);
    return true;
  } catch (error) {
    setStopOwed(userId, true);
    if (!quiet)
      toast.error(`${t("sharing.stopFailed.title")} ${errorMessage(error, { namespace: "map" })}`);
    return false;
  }
}

/** Tries a stop the server still has not heard of — unless the account is sharing again by now. */
function retryOwedStop(): void {
  const { userId, stopFailed, choice, permission, sessionEnded } = useSharing.getState();
  // Until the browser has said what the permission is, it is not known whether this device shares.
  if (!userId || !stopFailed || sessionEnded || permission === "unknown") return;
  if (choice === "on" && permission === "granted") {
    // Sharing again: the server hears from this device, and a stop now would hide a live pin.
    setStopOwed(userId, false);
    return;
  }
  void tellServerStopped(true);
}

export const useSharing = createStore<SharingStore>((set, get) => ({
  userId: null,
  choice: null,
  permission: "unknown",
  asked: false,
  attempt: 0,
  stopFailed: false,
  sessionEnded: false,
  setUser: (userId) => {
    if (userId === get().userId) return;
    set({
      userId,
      choice: readChoice(userId),
      asked: false,
      stopFailed: userId !== null && readStored(stopOwedKey(userId)) === "1",
      sessionEnded: false,
    });
  },
  setPermission: (permission) => set({ permission }),
  start: () => {
    const { userId, choice } = get();
    if (!userId) return;
    const kept = writeStored(choiceKey(userId), "on");
    writeStored(stopOwedKey(userId), null);
    useLocations.getState().setOwn(null);
    set((state) => ({ choice: "on", asked: true, attempt: state.attempt + 1, stopFailed: false }));
    if (choice === "on") return;
    // Honest about what this browser can remember: without storage the question comes back.
    if (kept) toast.success(t("sharing.turnedOn"));
    else toast.success(t("sharing.turnedOnThisVisit"), { duration: 8000 });
  },
  stop: async () => {
    const { userId } = get();
    if (!userId) return;
    writeStored(choiceKey(userId), "off");
    set({ choice: "off" });
    useLocations.getState().setOwn(null);
    // A sample another tab sends after the server heard "stopped" would start sharing again.
    await new Promise((resolve) => setTimeout(resolve, OTHER_TABS_MS));
    // Resumed (here or in another tab) while waiting: a stop sent now would land on top of the new samples.
    if (get().userId !== userId || readChoice(userId) !== "off") return;
    if (await tellServerStopped()) toast.success(t("sharing.stoppedToast"));
  },
  confirmStop: async () => {
    if (await tellServerStopped()) toast.success(t("sharing.stoppedToast"));
  },
}));

/** Whether this device is sharing right now, and if not, why. */
export type SharingState =
  | "sharing"
  | "starting"
  | "needsConsent"
  | "needsPermission"
  | "blocked"
  | "stopped"
  | "sessionEnded"
  | "unsupported"
  | "unavailable";

export function useSharingState(): SharingState {
  const { permission, asked, choice, sessionEnded } = useSharing();
  const own = useLocations((state) => state.own);
  const error = useLocations((state) => state.ownError);
  if (sessionEnded) return "sessionEnded";
  if (permission === "unsupported") return "unsupported";
  if (choice === null) return "needsConsent";
  if (choice === "off") return "stopped";
  if (permission === "denied" || error === "denied") return "blocked";
  if (own) return "sharing";
  if (error === "unavailable") return "unavailable";
  if (permission !== "granted" && !asked) return "needsPermission";
  return "starting";
}

interface BatteryManager {
  level: number;
  charging: boolean;
}

/** Battery level, where the browser exposes it (Chromium). */
async function readBattery(): Promise<BatteryManager | null> {
  const getBattery = (navigator as Navigator & { getBattery?: () => Promise<BatteryManager> })
    .getBattery;
  if (!getBattery) return null;
  try {
    return await getBattery.call(navigator);
  } catch {
    return null;
  }
}

/**
 * Sends one fix. `stillSharing` is asked again after every wait: a stop (in
 * this tab or another) must not be followed by one more sample.
 */
async function report(
  position: OwnPosition,
  userId: string,
  stillSharing: () => boolean,
): Promise<void> {
  const sample = await sampleOf(position, userId, stillSharing);
  if (!sample) return;
  // Prefer the realtime connection; fall back to a plain RPC while it is down.
  if (realtime().sendLocation(sample)) return;
  try {
    await locationClient().reportLocations({ samples: [sample] });
  } catch {
    // Dropping one sample is fine: the next one supersedes it.
  }
}

/** Sends one fix over the plain RPC and waits until the server has taken it. */
async function reportNow(
  position: OwnPosition,
  userId: string,
  stillSharing: () => boolean,
): Promise<void> {
  const sample = await sampleOf(position, userId, stillSharing);
  if (!sample) return;
  try {
    await locationClient().reportLocations({ samples: [sample] });
  } catch {
    // The request that follows says so if the server has no fix.
  }
}

async function sampleOf(
  position: OwnPosition,
  userId: string,
  stillSharing: () => boolean,
): Promise<geo.LocationSample | null> {
  const battery = await readBattery();
  if (!stillSharing()) return null;
  return locationSample(position, {
    batteryPercent: battery ? battery.level * 100 : undefined,
    charging: battery?.charging,
    foreground: document.visibilityState === "visible",
    sequence: takeSequence(userId),
  });
}

/**
 * Reads a fresh fix and makes sure the server has it before the caller goes
 * on: nearby discovery and bumps are only accepted where the account's own
 * latest fix puts it. Only for an account sharing from here with the
 * browser's permission already given, so it never prompts; otherwise (or
 * when the browser has no fix to give) it returns the position already known.
 */
export async function reportFreshFix(): Promise<OwnPosition | null> {
  const { userId, choice, permission, sessionEnded } = useSharing.getState();
  const known = useLocations.getState().own;
  if (!userId || choice !== "on" || permission !== "granted" || sessionEnded) return known;
  const stillSharing = () => sessionAlive() && readChoice(userId) === "on";
  // A reading the watch delivered a moment ago is fresh enough; otherwise ask for a new one.
  let position = known && Date.now() - known.recordedAt <= FRESH_FIX_MS ? known : null;
  if (!position) {
    const result = await new Promise<GeolocationPosition | null>((resolve) =>
      navigator.geolocation.getCurrentPosition(resolve, () => resolve(null), {
        enableHighAccuracy: true,
        // A cached reading may be older than what the server already has: it would be dropped.
        maximumAge: 0,
        timeout: 5_000,
      }),
    );
    if (!result || !stillSharing()) return known;
    position = toOwnPosition(result);
    useLocations.getState().setOwn(position);
  }
  // Awaited over the plain RPC, so the server has processed it when this returns (a fix it
  // already has is simply skipped there).
  await reportNow(position, userId, stillSharing);
  return position;
}

function toOwnPosition(result: GeolocationPosition): OwnPosition {
  return {
    lat: result.coords.latitude,
    lng: result.coords.longitude,
    accuracyMeters: result.coords.accuracy,
    speedMps: finiteOrUndefined(result.coords.speed) ?? null,
    headingDegrees: finiteOrUndefined(result.coords.heading) ?? null,
    recordedAt: result.timestamp,
  };
}

/**
 * Follows the browser's permission for geolocation, including changes made
 * in its settings. Taking the permission away while sharing is a stop: the
 * server is told, so friends do not keep seeing a pin that looks live.
 */
function usePermissionWatch(): void {
  useEffect(() => {
    const { setPermission } = useSharing.getState();
    if (!("geolocation" in navigator)) {
      setPermission("unsupported");
      return;
    }
    if (!navigator.permissions?.query) {
      // No way to ask without prompting (older Safari): wait for the person to turn it on.
      setPermission("prompt");
      return;
    }
    let status: PermissionStatus | null = null;
    let cancelled = false;
    const update = () => {
      if (!status) return;
      const before = useSharing.getState();
      setPermission(status.state);
      if (before.permission === "granted" && status.state !== "granted" && before.choice === "on") {
        useLocations.getState().setOwn(null, status.state === "denied" ? "denied" : null);
        // If this fails it stays on screen (the banner) and is retried: see `tellServerStopped`.
        void tellServerStopped();
      }
    };
    navigator.permissions.query({ name: "geolocation" }).then(
      (result) => {
        if (cancelled) return;
        status = result;
        update();
        result.addEventListener("change", update);
      },
      () => setPermission("prompt"),
    );
    return () => {
      cancelled = true;
      status?.removeEventListener("change", update);
    };
  }, []);
}

/**
 * Keeps the store on the signed-in account's own choice, follows that choice
 * when another tab changes it, and tells the server "stopped" before a
 * sign-out takes the session away.
 */
function useAccountChoice(): void {
  const myId = useMe()?.user?.id ?? null;
  useEffect(() => {
    useSharing.getState().setUser(myId);
    if (!myId) return;
    const endHere = () => {
      useSharing.setState({ sessionEnded: true });
      useLocations.getState().setOwn(null);
    };
    const onStorage = (event: StorageEvent) => {
      if (event.key === choiceKey(myId)) {
        const choice = readChoice(myId);
        useSharing.setState({ choice });
        if (choice !== "on") useLocations.getState().setOwn(null);
      } else if (event.key === stopOwedKey(myId)) {
        useSharing.setState({ stopFailed: event.newValue === "1" });
      } else if (
        event.key === SIGNED_IN_KEY ||
        event.key === SESSION_ENDING_KEY ||
        event.key === null
      ) {
        // Another tab signed out (or storage was cleared): this account's session is over here too.
        if (!sessionAlive()) endHere();
      }
    };
    window.addEventListener("storage", onStorage);
    // A new sign-in: whatever an earlier sign-out announced is over.
    if (sessionAlive() === false && readStored(SIGNED_IN_KEY) === "1")
      writeStored(SESSION_ENDING_KEY, null);
    // A stop the server never heard of: try again now (once the permission is known), and
    // whenever the connection is back.
    retryOwedStop();
    const offPermission = useSharing.subscribe((state, before) => {
      if (before.permission === "unknown" && state.permission !== "unknown") retryOwedStop();
    });
    window.addEventListener("online", retryOwedStop);
    const offSignOut = onBeforeSignOut(async () => {
      if (useSharing.getState().choice !== "on") return;
      // First every tab stops sending (this one too), then the server is told: in that
      // order, or a late sample from another tab would start sharing again.
      writeStored(SESSION_ENDING_KEY, String(Date.now()));
      endHere();
      await new Promise((resolve) => setTimeout(resolve, OTHER_TABS_MS));
      await tellServerStopped(true);
    });
    return () => {
      window.removeEventListener("storage", onStorage);
      window.removeEventListener("online", retryOwedStop);
      offPermission();
      offSignOut();
      useSharing.getState().setUser(null);
    };
  }, [myId]);
}

/**
 * Shares this device's position with friends for as long as the signed-in
 * account has said yes in this browser and has the app on screen: reads the
 * position from the browser, keeps the store's own position current and
 * sends fixes over the realtime connection at the pace the server asks for —
 * every few seconds while moving, every few minutes while still.
 *
 * - Nothing is read or sent for an account that has not turned sharing on
 *   here (`useSharing().start()`, one tap on the map), and it never makes
 *   the browser ask for permission by itself.
 * - While the tab is hidden the position is not watched at all.
 * - A stop in any tab stops every tab.
 */
export function useOwnLocationReporting(): void {
  usePermissionWatch();
  useAccountChoice();
  const userId = useSharing((state) => state.userId);
  const choice = useSharing((state) => state.choice);
  const permission = useSharing((state) => state.permission);
  const asked = useSharing((state) => state.asked);
  const attempt = useSharing((state) => state.attempt);
  const sessionEnded = useSharing((state) => state.sessionEnded);
  const active =
    userId !== null &&
    !sessionEnded &&
    choice === "on" &&
    (permission === "granted" ||
      (asked && permission !== "unsupported" && permission !== "denied"));

  // `attempt` is a dependency on purpose: "try again" starts a stuck attempt over.
  useEffect(() => {
    if (!active || !userId) return;
    let live = true;
    // Another tab's stop or sign-out is in storage before its event reaches this tab.
    const stillSharing = () => live && sessionAlive() && readChoice(userId) === "on";
    let lastReport: LastReport | null = null;
    /** The newest fix, until it has been sent. */
    let unsent: OwnPosition | null = null;
    let watchId: number | null = null;

    /** Sends the newest fix if it is time: at once for the first, then at the server's pace. */
    const flush = (force = false) => {
      if (!unsent || !stillSharing()) return;
      const position = unsent;
      const now = Date.now();
      const decision = reportDecision(position, lastReport, realtime().cadence, now, force);
      if (decision === "wait") return;
      unsent = null;
      // The same fix again (a cached reading) tells the server nothing new.
      if (decision === "same" && lastReport) {
        lastReport.at = now;
        return;
      }
      lastReport = { at: now, position };
      void report(position, userId, stillSharing);
    };

    const accept = (result: GeolocationPosition, force = false) => {
      if (!stillSharing()) return;
      const position = toOwnPosition(result);
      useLocations.getState().setOwn(position);
      useSharing.getState().setPermission("granted");
      unsent = position;
      flush(force);
    };
    const fail = (error: GeolocationPositionError) => {
      if (!live) return;
      const denied = error.code === error.PERMISSION_DENIED;
      if (denied) useSharing.getState().setPermission("denied");
      useLocations.getState().setOwn(null, denied ? "denied" : "unavailable");
    };

    const watch = () => {
      if (watchId !== null || document.visibilityState !== "visible") return;
      // Fresh readings only: every fix carries the time it was taken, and that is what is sent.
      watchId = navigator.geolocation.watchPosition((result) => accept(result), fail, {
        enableHighAccuracy: true,
        maximumAge: 0,
        timeout: 20_000,
      });
    };
    const unwatch = () => {
      if (watchId !== null) navigator.geolocation.clearWatch(watchId);
      watchId = null;
    };

    const heartbeat = setInterval(() => {
      if (document.visibilityState !== "visible") return;
      // A fix that arrived too soon after the last report goes out when its turn comes.
      flush();
      // Standing still produces no new readings: take one when the stationary interval is up.
      if (!lastReport || unsent) return;
      if (Date.now() - lastReport.at < realtime().cadence.stationarySeconds * 1000) return;
      navigator.geolocation.getCurrentPosition(
        (result) => accept(result, true),
        () => undefined,
        {
          enableHighAccuracy: true,
          maximumAge: 0,
          timeout: 15_000,
        },
      );
    }, 1000);

    // A hidden tab does not watch the position; coming back starts again with a fresh fix.
    const onVisibility = () => (document.visibilityState === "visible" ? watch() : unwatch());
    document.addEventListener("visibilitychange", onVisibility);
    watch();

    return () => {
      live = false;
      unwatch();
      clearInterval(heartbeat);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [active, userId, attempt]);
}
