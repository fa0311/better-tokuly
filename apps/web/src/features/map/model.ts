import { geo, toMillis } from "@tokuly/api";
import { distanceMeters } from "#/lib/format";

/** Positions older than this are shown as stale (dimmed, with their age). */
export const STALE_AFTER_MS = 10 * 60_000;
/**
 * A blurred pin's time is rounded down to ten-minute steps and only sent when
 * the step changes, so it needs a longer leash before it counts as stale.
 */
export const BLURRED_STALE_AFTER_MS = 30 * 60_000;
/** Faster than this counts as moving. */
export const MOVING_SPEED_MPS = 0.8;
/** Buzz only works this close together (the server's default radius). */
export const BUZZ_RADIUS_M = 200;
/** Two shakes match within this distance. */
export const BUMP_RADIUS_M = 60;

/**
 * How a pin is drawn. A frozen pin is never told apart from a phone that
 * stopped reporting: it simply goes stale.
 */
export type PinState = "live" | "blurred" | "stale" | "paused";

export function pinState(location: geo.FriendLocation, now: number): PinState {
  if (location.sharingPaused) return "paused";
  const age = now - toMillis(location.recordedAt);
  if (location.precision === geo.GhostMode.BLURRED)
    return age > BLURRED_STALE_AFTER_MS ? "stale" : "blurred";
  return age > STALE_AFTER_MS ? "stale" : "live";
}

export function isMoving(location: geo.FriendLocation, now: number): boolean {
  return pinState(location, now) === "live" && location.speedMps >= MOVING_SPEED_MPS;
}

/** How long someone has been on the spot, when that is worth showing (five minutes or more). */
export function stayMillis(location: geo.FriendLocation, now: number): number | null {
  const since = toMillis(location.stationarySince);
  if (!since || isMoving(location, now)) return null;
  const stay = now - since;
  return stay >= 5 * 60_000 ? stay : null;
}

/** The rule a friend is under right now: their override if it is still running, else the default. */
export function ghostRuleFor(
  settings: geo.GetGhostSettingsResponse | undefined,
  userId: string,
  now: number,
): geo.GhostMode {
  const running = (setting: geo.GhostSetting | undefined) => {
    if (!setting || setting.mode === geo.GhostMode.UNSPECIFIED) return undefined;
    const until = toMillis(setting.until);
    return until && until <= now ? undefined : setting.mode;
  };
  return (
    running(settings?.overrides[userId]) ?? running(settings?.everyone) ?? geo.GhostMode.PRECISE
  );
}

/** The strictest thing the settings do to anyone, for the indicator on the map. */
export function ghostSummary(
  settings: geo.GetGhostSettingsResponse | undefined,
  now: number,
): {
  mode: geo.GhostMode;
  scope: "everyone" | "some";
  hiddenCount: number;
  until: number | null;
} | null {
  if (!settings) return null;
  const everyone = ghostRuleFor(
    { ...settings, overrides: {} } as geo.GetGhostSettingsResponse,
    "",
    now,
  );
  if (everyone !== geo.GhostMode.PRECISE) {
    return {
      mode: everyone,
      scope: "everyone",
      hiddenCount: 0,
      until: toMillis(settings.everyone?.until) || null,
    };
  }
  const hidden = Object.entries(settings.overrides).filter(
    ([userId]) => ghostRuleFor(settings, userId, now) !== geo.GhostMode.PRECISE,
  );
  if (hidden.length === 0) return null;
  const frozen = hidden.some(([, setting]) => setting.mode === geo.GhostMode.FROZEN);
  const untils = hidden.map(([, setting]) => toMillis(setting.until));
  return {
    mode: frozen ? geo.GhostMode.FROZEN : geo.GhostMode.BLURRED,
    scope: "some",
    hiddenCount: hidden.length,
    // A shared end time is worth showing; mixed ones are not.
    until: untils.every((until) => until && until === untils[0]) ? (untils[0] ?? null) : null,
  };
}

/**
 * Whether the two see each other precisely and live, as far as this client
 * can know: their pin is precise and fresh, and my rule for them is precise.
 * Distances, travel times, trips, bumps and buzzes only exist for such pairs.
 */
export function seeEachOther(
  location: geo.FriendLocation | undefined,
  myRule: geo.GhostMode,
  now: number,
): boolean {
  return (
    Boolean(location?.position) &&
    pinState(location as geo.FriendLocation, now) === "live" &&
    myRule === geo.GhostMode.PRECISE
  );
}

export interface ScreenPoint {
  id: string;
  x: number;
  y: number;
}

/**
 * Groups pins whose screen positions are within `radius` pixels of each
 * other, transitively. Returns the groups with two or more members, each as
 * sorted ids, so the same crowd always gives the same key.
 */
export function clusterOnScreen(points: ScreenPoint[], radius: number): string[][] {
  const parent = points.map((_, index) => index);
  const find = (i: number): number => {
    while (parent[i] !== i) {
      parent[i] = parent[parent[i] as number] as number;
      i = parent[i] as number;
    }
    return i;
  };
  const r2 = radius * radius;
  for (let a = 0; a < points.length; a++) {
    for (let b = a + 1; b < points.length; b++) {
      const pa = points[a] as ScreenPoint;
      const pb = points[b] as ScreenPoint;
      if ((pa.x - pb.x) ** 2 + (pa.y - pb.y) ** 2 <= r2) parent[find(a)] = find(b);
    }
  }
  const buckets = new Map<number, string[]>();
  points.forEach((point, index) => {
    const root = find(index);
    const bucket = buckets.get(root);
    if (bucket) bucket.push(point.id);
    else buckets.set(root, [point.id]);
  });
  return [...buckets.values()].filter((ids) => ids.length >= 2).map((ids) => ids.sort());
}

/**
 * Where member `index` of `count` sits when a stack of pins fans out: two
 * side by side, more on a ring starting at the top.
 */
export function fanOffset(index: number, count: number): [number, number] {
  if (count === 2) return [index === 0 ? -44 : 44, 0];
  const radius = Math.max(58, (count * 62) / (2 * Math.PI));
  const angle = (index / count) * 2 * Math.PI - Math.PI / 2;
  return [Math.round(Math.cos(angle) * radius), Math.round(Math.sin(angle) * radius)];
}

/** A circle around a point as a GeoJSON ring (for accuracy and "somewhere around here" areas). */
export function circleRing(
  center: { lat: number; lng: number },
  radiusMeters: number,
  steps = 48,
): [number, number][] {
  const ring: [number, number][] = [];
  const latRadius = radiusMeters / 111_320;
  const lngRadius = latRadius / Math.max(0.01, Math.cos((center.lat * Math.PI) / 180));
  for (let i = 0; i <= steps; i++) {
    const angle = (i / steps) * 2 * Math.PI;
    ring.push([center.lng + Math.cos(angle) * lngRadius, center.lat + Math.sin(angle) * latRadius]);
  }
  return ring;
}

/** Decodes a Google encoded polyline (precision 5) into `[lng, lat]` pairs. */
export function decodePolyline(encoded: string): [number, number][] {
  const points: [number, number][] = [];
  let index = 0;
  let lat = 0;
  let lng = 0;
  const next = () => {
    let result = 0;
    let shift = 0;
    let byte: number;
    do {
      byte = encoded.charCodeAt(index++) - 63;
      result |= (byte & 0x1f) << shift;
      shift += 5;
    } while (byte >= 0x20 && index < encoded.length);
    return result & 1 ? ~(result >> 1) : result >> 1;
  };
  while (index < encoded.length) {
    lat += next();
    lng += next();
    points.push([lng / 1e5, lat / 1e5]);
  }
  return points;
}

/** Order for the friend list: nearest first, people without a known distance last. */
export function byDistance<T extends { distance: number | null }>(a: T, b: T): number {
  return (a.distance ?? Number.POSITIVE_INFINITY) - (b.distance ?? Number.POSITIVE_INFINITY);
}

/** Distance between me and a pin, only when it means something (a precise pin). */
export function distanceTo(
  own: { lat: number; lng: number } | null,
  location: geo.FriendLocation | undefined,
): number | null {
  if (!own || !location?.position || location.precision === geo.GhostMode.BLURRED) return null;
  return distanceMeters(own, location.position);
}
