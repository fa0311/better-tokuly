import { create, fromDate, geo } from "@tokuly/api";
import { useEffect } from "react";
import { toast } from "sonner";
import { create as createStore } from "zustand";
import { errorMessage } from "#/lib/errors";
import { distanceMeters } from "#/lib/format";
import i18n from "#/lib/i18n";
import { realtime } from "#/lib/realtime";
import { type OwnPosition, useLocations } from "#/stores/locations";
import { onBeforeSignOut, useMe } from "#/stores/session";
import { locationClient } from "./data";
import { MOVING_SPEED_MPS } from "./model";

/** Browsers on laptops report no speed: having moved this far since the last report counts as moving too. */
const MOVED_METERS = 15;
/** Other tabs hear about a stop through a storage event; give them this long before the server is told. */
const OTHER_TABS_MS = 300;

/**
 * What an account decided about sharing its position from this browser.
 * The decision belongs to the account, not to the browser: someone else
 * signing in here starts undecided, whatever the browser's own location
 * permission says, and nothing is sent for them until they say yes.
 */
export type SharingChoice = "on" | "off" | null;

const choiceKey = (userId: string) => `tokuly.map.sharing.${userId}`;

function readChoice(userId: string | null): SharingChoice {
  if (!userId) return null;
  try {
    const value = window.localStorage.getItem(choiceKey(userId));
    return value === "on" || value === "off" ? value : null;
  } catch {
    return null;
  }
}

function writeChoice(userId: string, choice: "on" | "off"): void {
  try {
    window.localStorage.setItem(choiceKey(userId), choice);
  } catch {
    // Private windows without storage: the choice lasts for this page.
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
  /** Sharing stopped here, but the server could not be told: friends may still see a live pin. */
  stopFailed: boolean;
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

async function tellServerStopped(): Promise<boolean> {
  try {
    await locationClient().stopSharing({});
    useSharing.setState({ stopFailed: false });
    return true;
  } catch (error) {
    // Never let someone believe they stopped when friends may still see them live.
    useSharing.setState({ stopFailed: true });
    toast.error(`${t("sharing.stopFailed.title")} ${errorMessage(error, { namespace: "map" })}`);
    return false;
  }
}

export const useSharing = createStore<SharingStore>((set, get) => ({
  userId: null,
  choice: null,
  permission: "unknown",
  asked: false,
  attempt: 0,
  stopFailed: false,
  setUser: (userId) => {
    if (userId === get().userId) return;
    set({ userId, choice: readChoice(userId), asked: false, stopFailed: false });
  },
  setPermission: (permission) => set({ permission }),
  start: () => {
    const { userId, choice } = get();
    if (!userId) return;
    writeChoice(userId, "on");
    useLocations.getState().setOwn(null);
    set((state) => ({ choice: "on", asked: true, attempt: state.attempt + 1, stopFailed: false }));
    if (choice !== "on") toast.success(t("sharing.turnedOn"));
  },
  stop: async () => {
    const { userId } = get();
    if (!userId) return;
    writeChoice(userId, "off");
    set({ choice: "off" });
    useLocations.getState().setOwn(null);
    // A sample another tab sends after the server heard "stopped" would start sharing again.
    await new Promise((resolve) => setTimeout(resolve, OTHER_TABS_MS));
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
  | "unsupported"
  | "unavailable";

export function useSharingState(): SharingState {
  const { permission, asked, choice } = useSharing();
  const own = useLocations((state) => state.own);
  const error = useLocations((state) => state.ownError);
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

/** A number the API can take: browsers give `NaN` for the heading of a device that stands still. */
const finite = (value: number | null) =>
  value !== null && Number.isFinite(value) ? value : undefined;

/**
 * Sends one fix. `stillSharing` is asked again after every wait: a stop (in
 * this tab or another) must not be followed by one more sample.
 */
async function report(position: OwnPosition, stillSharing: () => boolean): Promise<void> {
  const battery = await readBattery();
  if (!stillSharing()) return;
  const sample = create(geo.LocationSampleSchema, {
    position: { lat: position.lat, lng: position.lng },
    accuracyMeters: position.accuracyMeters,
    speedMps: finite(position.speedMps),
    headingDegrees: finite(position.headingDegrees),
    // The moment the device took the fix, not the moment it is sent.
    recordedAt: fromDate(new Date(position.recordedAt)),
    batteryPercent: battery ? Math.round(battery.level * 100) : undefined,
    charging: battery?.charging,
    foreground: document.visibilityState === "visible",
  });
  // Prefer the realtime connection; fall back to a plain RPC while it is down.
  if (realtime().sendLocation(sample)) return;
  try {
    await locationClient().reportLocations({ samples: [sample] });
  } catch {
    // Dropping one sample is fine: the next one supersedes it.
  }
}

function toOwnPosition(result: GeolocationPosition): OwnPosition {
  return {
    lat: result.coords.latitude,
    lng: result.coords.longitude,
    accuracyMeters: result.coords.accuracy,
    speedMps: finite(result.coords.speed) ?? null,
    headingDegrees: finite(result.coords.heading) ?? null,
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
    const onStorage = (event: StorageEvent) => {
      if (event.key !== choiceKey(myId)) return;
      const choice = readChoice(myId);
      useSharing.setState({ choice });
      if (choice !== "on") useLocations.getState().setOwn(null);
    };
    window.addEventListener("storage", onStorage);
    const offSignOut = onBeforeSignOut(async () => {
      if (useSharing.getState().choice !== "on" || !useLocations.getState().own) return;
      await locationClient()
        .stopSharing({})
        .catch(() => undefined);
    });
    return () => {
      window.removeEventListener("storage", onStorage);
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
  const active =
    userId !== null &&
    choice === "on" &&
    (permission === "granted" ||
      (asked && permission !== "unsupported" && permission !== "denied"));

  // `attempt` is a dependency on purpose: "try again" starts a stuck attempt over.
  useEffect(() => {
    if (!active || !userId) return;
    let live = true;
    // The other tab's stop is in storage before its event reaches this tab.
    const stillSharing = () => live && readChoice(userId) === "on";
    let lastReport: { at: number; position: OwnPosition } | null = null;
    /** The newest fix, until it has been sent. */
    let unsent: OwnPosition | null = null;
    let watchId: number | null = null;

    /** Sends the newest fix if it is time: at once for the first, then at the server's pace. */
    const flush = (force = false) => {
      if (!unsent || !stillSharing()) return;
      const position = unsent;
      const cadence = realtime().cadence;
      const now = Date.now();
      const moving =
        (position.speedMps ?? 0) >= MOVING_SPEED_MPS ||
        (lastReport !== null &&
          distanceMeters(lastReport.position, position) >=
            Math.max(MOVED_METERS, position.accuracyMeters));
      const interval = (moving ? cadence.movingSeconds : cadence.stationarySeconds) * 1000;
      if (!force && lastReport && now - lastReport.at < interval) return;
      unsent = null;
      // The same fix again (a cached reading) tells the server nothing new.
      if (lastReport && position.recordedAt <= lastReport.position.recordedAt) {
        lastReport.at = now;
        return;
      }
      lastReport = { at: now, position };
      void report(position, stillSharing);
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
