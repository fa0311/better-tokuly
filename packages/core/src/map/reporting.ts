import { create, fromDate, geo } from "@tokuly/api";
import { distanceMeters } from "../format";
import { MOVING_SPEED_MPS } from "./model";

/** The device's own position as last read from the platform's location service. */
export interface OwnPosition {
  lat: number;
  lng: number;
  accuracyMeters: number;
  speedMps: number | null;
  headingDegrees: number | null;
  /** When the device took the fix (epoch milliseconds). */
  recordedAt: number;
}

/** How often the server wants a fix: often while moving, rarely while still (the realtime Welcome). */
export interface ReportCadence {
  movingSeconds: number;
  stationarySeconds: number;
}

/** The last fix this device sent, and when. */
export interface LastReport {
  at: number;
  position: OwnPosition;
}

/**
 * Devices that report no speed (laptops, some phones in power saving): having
 * moved this far since the last report counts as moving too.
 */
export const MOVED_METERS = 15;

/** Whether a fix says the device is on the move, compared with the last report. */
export function isMovingFix(position: OwnPosition, last: LastReport | null): boolean {
  if ((position.speedMps ?? 0) >= MOVING_SPEED_MPS) return true;
  if (!last) return false;
  return distanceMeters(last.position, position) >= Math.max(MOVED_METERS, position.accuracyMeters);
}

/**
 * What to do with the newest fix:
 * - `send`: it is time (the first fix, a forced one, or the interval is up);
 * - `wait`: too soon after the last report, keep it until its turn comes;
 * - `same`: it is the fix already sent (a cached reading), it tells the server nothing.
 */
export type ReportDecision = "send" | "wait" | "same";

export function reportDecision(
  position: OwnPosition,
  last: LastReport | null,
  cadence: ReportCadence,
  now: number,
  force = false,
): ReportDecision {
  if (!last) return "send";
  const seconds = isMovingFix(position, last) ? cadence.movingSeconds : cadence.stationarySeconds;
  if (!force && now - last.at < seconds * 1000) return "wait";
  return position.recordedAt <= last.position.recordedAt ? "same" : "send";
}

/** A number the API can take: platforms give `NaN` (or -1) for the heading of a device that stands still. */
export function finiteOrUndefined(value: number | null | undefined): number | undefined {
  return value !== null && value !== undefined && Number.isFinite(value) && value >= 0
    ? value
    : undefined;
}

/** What the device says about itself next to the fix. */
export interface SampleContext {
  /** 0–100, when the platform tells. */
  batteryPercent?: number;
  charging?: boolean;
  /** The app is on screen. */
  foreground: boolean;
}

/** The sample the geo API takes for one fix: the moment the device took it, not the moment it is sent. */
export function locationSample(position: OwnPosition, context: SampleContext): geo.LocationSample {
  return create(geo.LocationSampleSchema, {
    position: { lat: position.lat, lng: position.lng },
    accuracyMeters: position.accuracyMeters,
    speedMps: finiteOrUndefined(position.speedMps),
    headingDegrees: finiteOrUndefined(position.headingDegrees),
    recordedAt: fromDate(new Date(position.recordedAt)),
    batteryPercent:
      context.batteryPercent === undefined ? undefined : Math.round(context.batteryPercent),
    charging: context.charging,
    foreground: context.foreground,
  });
}
