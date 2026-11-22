import { create, fromDate, geo } from "@tokuly/api";
import { useEffect } from "react";
import { create as createStore } from "zustand";
import { distanceMeters } from "#/lib/format";
import { realtime } from "#/lib/realtime";
import { type OwnPosition, useLocations } from "#/stores/locations";
import { locationClient } from "./data";
import { MOVING_SPEED_MPS } from "./model";

const STOPPED_KEY = "tokuly.map.sharingStopped";
/** Browsers on laptops report no speed: having moved this far since the last report counts as moving too. */
const MOVED_METERS = 15;

/**
 * Where location sharing stands on this device.
 * - `permission`: what the browser says about geolocation for this site.
 * - `asked`: the person tapped "turn on" here, so the browser may show its prompt.
 * - `stopped`: the person chose to stop sharing from this device (remembered).
 */
interface SharingStore {
  permission: "unknown" | "prompt" | "granted" | "denied" | "unsupported";
  asked: boolean;
  stopped: boolean;
  setPermission(permission: SharingStore["permission"]): void;
  /** Starts sharing: lets the browser ask when it has not been asked yet. */
  start(): void;
  /** Stops sharing from this device and tells friends right away. */
  stop(): void;
}

function readStopped(): boolean {
  try {
    return typeof window !== "undefined" && window.localStorage.getItem(STOPPED_KEY) === "1";
  } catch {
    return false;
  }
}

export const useSharing = createStore<SharingStore>((set) => ({
  permission: "unknown",
  asked: false,
  stopped: readStopped(),
  setPermission: (permission) => set({ permission }),
  start: () => {
    window.localStorage.removeItem(STOPPED_KEY);
    set({ asked: true, stopped: false });
  },
  stop: () => {
    window.localStorage.setItem(STOPPED_KEY, "1");
    set({ stopped: true });
    useLocations.getState().setOwn(null);
    // Friends see the pin go stale at once instead of waiting for it to age.
    void locationClient()
      .stopSharing({})
      .catch(() => undefined);
  },
}));

/** Whether this device is sharing right now, and if not, why. */
export type SharingState =
  | "sharing"
  | "starting"
  | "needsPermission"
  | "blocked"
  | "stopped"
  | "unsupported"
  | "unavailable";

export function useSharingState(): SharingState {
  const { permission, asked, stopped } = useSharing();
  const own = useLocations((state) => state.own);
  const error = useLocations((state) => state.ownError);
  if (permission === "unsupported") return "unsupported";
  if (stopped) return "stopped";
  if (permission === "denied" || error === "denied") return "blocked";
  if (own) return "sharing";
  if (error === "unavailable") return "unavailable";
  if (permission === "prompt" && !asked) return "needsPermission";
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

async function report(position: OwnPosition): Promise<void> {
  const battery = await readBattery();
  const sample = create(geo.LocationSampleSchema, {
    position: { lat: position.lat, lng: position.lng },
    accuracyMeters: position.accuracyMeters,
    speedMps: position.speedMps ?? undefined,
    headingDegrees: position.headingDegrees ?? undefined,
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
    speedMps: result.coords.speed,
    headingDegrees: result.coords.heading,
    // Some browsers stamp a cached fix with its original time: never report the past twice.
    recordedAt: result.timestamp,
  };
}

/** Follows the browser's permission for geolocation, including changes made in its settings. */
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
    const update = () => status && setPermission(status.state);
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
 * Shares this device's position with friends for as long as someone is
 * signed in and has the app open: reads it from the browser, keeps the
 * store's own position current and sends samples over the realtime
 * connection at the pace the server asks for — every few seconds while
 * moving, every few minutes while still.
 *
 * It never makes the browser ask for permission by itself: that happens when
 * the person taps "turn on" on the map (`useSharing().start()`). While the
 * tab is hidden it does not ask the device for positions of its own accord;
 * whatever the browser still delivers is passed on, marked as background.
 */
export function useOwnLocationReporting(): void {
  usePermissionWatch();
  const permission = useSharing((state) => state.permission);
  const asked = useSharing((state) => state.asked);
  const stopped = useSharing((state) => state.stopped);
  const active =
    !stopped &&
    (permission === "granted" ||
      (asked && permission !== "unsupported" && permission !== "denied"));

  useEffect(() => {
    if (!active) return;
    const startedAt = Date.now();
    let lastReport: { at: number; position: OwnPosition } | null = null;
    let lastTimestamp = 0;
    /** The newest fix, until it has been sent. */
    let unsent: OwnPosition | null = null;

    /** Sends the newest fix if it is time: at once for the first, then at the server's pace. */
    const flush = (force = false) => {
      if (!unsent) return;
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
      // A sample must be newer than the one before it (the server skips duplicates), and a
      // cached fix from before sharing (re)started still says where the device is now.
      const recordedAt = Math.min(
        now,
        Math.max(position.recordedAt, lastTimestamp + 1000, startedAt),
      );
      if (recordedAt <= lastTimestamp) return;
      lastTimestamp = recordedAt;
      lastReport = { at: now, position };
      unsent = null;
      void report({ ...position, recordedAt });
    };

    const accept = (result: GeolocationPosition, force = false) => {
      const position = toOwnPosition(result);
      useLocations.getState().setOwn(position);
      useSharing.getState().setPermission("granted");
      unsent = position;
      flush(force);
    };
    const fail = (error: GeolocationPositionError) => {
      const denied = error.code === error.PERMISSION_DENIED;
      if (denied) useSharing.getState().setPermission("denied");
      useLocations.getState().setOwn(null, denied ? "denied" : "unavailable");
    };
    const refresh = (maximumAge: number) =>
      navigator.geolocation.getCurrentPosition(
        (result) =>
          accept({ coords: result.coords, timestamp: Date.now() } as GeolocationPosition, true),
        () => undefined,
        { enableHighAccuracy: true, maximumAge, timeout: 15_000 },
      );

    const watchId = navigator.geolocation.watchPosition((result) => accept(result), fail, {
      enableHighAccuracy: true,
      maximumAge: 5_000,
      timeout: 20_000,
    });

    const heartbeat = setInterval(() => {
      // A fix that arrived too soon after the last report goes out when its turn comes.
      flush();
      // Standing still produces no new readings: look again when the stationary interval is up.
      if (document.visibilityState !== "visible" || !lastReport || unsent) return;
      if (Date.now() - lastReport.at >= realtime().cadence.stationarySeconds * 1000) refresh(0);
    }, 1000);

    // Coming back to the tab: friends get a fresh pin at once.
    const onVisible = () => {
      if (document.visibilityState === "visible" && lastReport) refresh(10_000);
    };
    document.addEventListener("visibilitychange", onVisible);

    return () => {
      navigator.geolocation.clearWatch(watchId);
      clearInterval(heartbeat);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [active]);
}
