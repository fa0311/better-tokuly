import { create, fromDate, geo } from "@tokuly/api";
import { useEffect } from "react";
import { api } from "#/lib/api";
import { realtime } from "#/lib/realtime";
import { type OwnPosition, useLocations } from "#/stores/locations";

/** Below this speed the device counts as standing still. */
const MOVING_SPEED_MPS = 0.8;

interface BatteryManager {
  level: number;
  charging: boolean;
}

/** Battery level, where the browser exposes it (Chromium). */
async function readBattery(): Promise<BatteryManager | null> {
  const getBattery = (navigator as Navigator & { getBattery?: () => Promise<BatteryManager> }).getBattery;
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
    await api().service(geo.LocationService).reportLocations({ samples: [sample] });
  } catch {
    // Dropping one sample is fine: the next one supersedes it.
  }
}

/**
 * Watches the device position while the map is open, keeps the store's own
 * position current and shares it with friends at the cadence the server asks
 * for (often while moving, rarely while still).
 */
export function useOwnLocation(enabled: boolean): void {
  useEffect(() => {
    if (!enabled) return;
    if (!("geolocation" in navigator)) {
      useLocations.getState().setOwn(null, "unavailable");
      return;
    }
    let lastReportAt = 0;
    const watchId = navigator.geolocation.watchPosition(
      (result) => {
        const position: OwnPosition = {
          lat: result.coords.latitude,
          lng: result.coords.longitude,
          accuracyMeters: result.coords.accuracy,
          speedMps: result.coords.speed,
          headingDegrees: result.coords.heading,
          recordedAt: result.timestamp,
        };
        useLocations.getState().setOwn(position);
        const cadence = realtime().cadence;
        const moving = (position.speedMps ?? 0) >= MOVING_SPEED_MPS;
        const intervalMs = (moving ? cadence.movingSeconds : cadence.stationarySeconds) * 1000;
        if (Date.now() - lastReportAt >= intervalMs) {
          lastReportAt = Date.now();
          void report(position);
        }
      },
      (error) => {
        useLocations.getState().setOwn(null, error.code === error.PERMISSION_DENIED ? "denied" : "unavailable");
      },
      { enableHighAccuracy: true, maximumAge: 5_000, timeout: 20_000 },
    );
    return () => navigator.geolocation.clearWatch(watchId);
  }, [enabled]);
}
