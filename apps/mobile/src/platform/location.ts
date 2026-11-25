import type { OwnPosition } from "@tokuly/core/map";
import * as Battery from "expo-battery";
import * as Location from "expo-location";
import * as TaskManager from "expo-task-manager";

/** What the platform lets the app do with the device's position right now. */
export interface LocationAccess {
  /** Location services of the whole device. */
  servicesEnabled: boolean;
  foreground: "undetermined" | "granted" | "denied";
  /** "Always" (iOS) / "Allow all the time" (Android): fixes keep coming while the app is closed. */
  background: boolean;
  /** Only an approximate position (iOS reduced accuracy, Android coarse). */
  approximate: boolean;
}

function status(response: Location.LocationPermissionResponse): LocationAccess["foreground"] {
  if (response.granted) return "granted";
  return response.canAskAgain ? "undetermined" : "denied";
}

function access(
  servicesEnabled: boolean,
  foreground: Location.LocationPermissionResponse,
  background: Location.LocationPermissionResponse | null,
): LocationAccess {
  return {
    servicesEnabled,
    foreground: status(foreground),
    background: Boolean(background?.granted),
    approximate:
      foreground.ios?.accuracy === "reduced" || foreground.android?.accuracy === "coarse",
  };
}

/** Reads the permissions without asking. */
export async function readLocationAccess(): Promise<LocationAccess> {
  const [servicesEnabled, foreground] = await Promise.all([
    Location.hasServicesEnabledAsync(),
    Location.getForegroundPermissionsAsync(),
  ]);
  const background = foreground.granted ? await Location.getBackgroundPermissionsAsync() : null;
  return access(servicesEnabled, foreground, background);
}

/** Shows the system prompt for "while using the app". */
export async function askForegroundAccess(): Promise<LocationAccess> {
  await Location.requestForegroundPermissionsAsync();
  return readLocationAccess();
}

/** Shows the system prompt (or, on Android 11+, the settings page) for "always". */
export async function askBackgroundAccess(): Promise<LocationAccess> {
  await Location.requestBackgroundPermissionsAsync();
  return readLocationAccess();
}

export function toOwnPosition(location: Location.LocationObject): OwnPosition {
  return {
    lat: location.coords.latitude,
    lng: location.coords.longitude,
    accuracyMeters: location.coords.accuracy ?? 1000,
    speedMps: location.coords.speed,
    headingDegrees: location.coords.heading,
    recordedAt: location.timestamp,
  };
}

/** Fixes while the app is on screen. Returns the function that stops them. */
export async function watchForeground(
  precise: boolean,
  onFix: (position: OwnPosition) => void,
): Promise<() => void> {
  const subscription = await Location.watchPositionAsync(
    {
      accuracy: precise ? Location.Accuracy.High : Location.Accuracy.Balanced,
      distanceInterval: 10,
      timeInterval: 5_000,
    },
    (location) => onFix(toOwnPosition(location)),
  );
  return () => subscription.remove();
}

/** One fresh fix, or `null` when the device has none to give in time. */
export async function currentPosition(): Promise<OwnPosition | null> {
  try {
    return toOwnPosition(
      await Location.getCurrentPositionAsync({ accuracy: Location.Accuracy.Balanced }),
    );
  } catch {
    return null;
  }
}

/** Battery level (0–100) and charging, when the device tells. */
export async function readBattery(): Promise<{ batteryPercent?: number; charging?: boolean }> {
  try {
    const [level, state] = await Promise.all([
      Battery.getBatteryLevelAsync(),
      Battery.getBatteryStateAsync(),
    ]);
    return {
      batteryPercent: level >= 0 ? level * 100 : undefined,
      charging: state === Battery.BatteryState.CHARGING || state === Battery.BatteryState.FULL,
    };
  } catch {
    return {};
  }
}

const BACKGROUND_TASK = "tokuly.location.background";

/**
 * Defines what runs for fixes delivered while the app is in the background
 * (or not running: the OS starts it headless). Must run when the JS bundle
 * loads, before any screen.
 */
export function defineBackgroundLocationTask(
  onFixes: (positions: OwnPosition[]) => Promise<void>,
): void {
  TaskManager.defineTask<{ locations: Location.LocationObject[] }>(
    BACKGROUND_TASK,
    async ({ data, error }) => {
      if (error || !data?.locations?.length) return;
      await onFixes(data.locations.map(toOwnPosition));
    },
  );
}

export interface BackgroundNotice {
  title: string;
  body: string;
  color: string;
}

/**
 * Background updates, battery-aware: woken only after moving 50 m, delivered
 * in batches (deferred) and paused by the OS while the device stays put.
 * Android shows `notice` as the foreground service's notification.
 */
export async function startBackgroundUpdates(notice: BackgroundNotice): Promise<void> {
  if (await Location.hasStartedLocationUpdatesAsync(BACKGROUND_TASK)) return;
  await Location.startLocationUpdatesAsync(BACKGROUND_TASK, {
    accuracy: Location.Accuracy.Balanced,
    distanceInterval: 50,
    deferredUpdatesDistance: 100,
    deferredUpdatesInterval: 60_000,
    pausesUpdatesAutomatically: true,
    activityType: Location.ActivityType.Other,
    showsBackgroundLocationIndicator: false,
    foregroundService: {
      notificationTitle: notice.title,
      notificationBody: notice.body,
      notificationColor: notice.color,
      killServiceOnDestroy: false,
    },
  });
}

export async function stopBackgroundUpdates(): Promise<void> {
  try {
    if (await Location.hasStartedLocationUpdatesAsync(BACKGROUND_TASK))
      await Location.stopLocationUpdatesAsync(BACKGROUND_TASK);
  } catch {
    // Not running.
  }
}
