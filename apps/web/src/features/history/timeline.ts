import { geo, toMillis } from "@tokuly/api";
import { distanceMeters } from "#/lib/format";

/** A point of the day's route, flattened for drawing and replay. */
export interface TrackPoint {
  lat: number;
  lng: number;
  /** Milliseconds since the epoch. */
  at: number;
  activity: geo.Activity;
}

export const toTrack = (points: geo.RoutePoint[]): TrackPoint[] =>
  points.flatMap((point) =>
    point.position
      ? [
          {
            lat: point.position.lat,
            lng: point.position.lng,
            at: toMillis(point.recordedAt),
            activity: point.activity,
          },
        ]
      : [],
  );

/** Decodes a Google encoded polyline (precision 5) into `[lat, lng]` pairs. */
export function decodePolyline(encoded: string): { lat: number; lng: number }[] {
  const points: { lat: number; lng: number }[] = [];
  let [index, lat, lng] = [0, 0, 0];
  const next = (): number | null => {
    let [result, shift] = [0, 0];
    for (;;) {
      if (index >= encoded.length) return null;
      const byte = encoded.charCodeAt(index++) - 63;
      result |= (byte & 0x1f) << shift;
      shift += 5;
      if (byte < 0x20) break;
    }
    return result & 1 ? ~(result >> 1) : result >> 1;
  };
  for (;;) {
    const dLat = next();
    const dLng = next();
    if (dLat === null || dLng === null) break;
    lat += dLat;
    lng += dLng;
    points.push({ lat: lat / 1e5, lng: lng / 1e5 });
  }
  return points;
}

export interface VisitItem {
  kind: "visit";
  visit: geo.Visit;
}

export interface MoveItem {
  kind: "move";
  from: number;
  to: number;
  /** UNSPECIFIED when the way of travelling is not known. */
  activity: geo.Activity;
  distanceMeters: number;
}

export type TimelineItem = VisitItem | MoveItem;

/** Shorter hops than this are not worth a line of their own. */
const MIN_MOVE_MS = 60_000;
const MIN_MOVE_METERS = 50;

/** A way of travelling guessed from the average speed, when the device did not say. */
function activityForSpeed(metersPerSecond: number): geo.Activity {
  if (metersPerSecond < 2.2) return geo.Activity.WALKING;
  if (metersPerSecond < 4) return geo.Activity.RUNNING;
  if (metersPerSecond < 8) return geo.Activity.CYCLING;
  if (metersPerSecond < 30) return geo.Activity.DRIVING;
  if (metersPerSecond < 90) return geo.Activity.TRAIN;
  return geo.Activity.FLYING;
}

/** How a stretch of the route was travelled: the activity most of the moving time was spent in. */
function movement(
  track: TrackPoint[],
  from: number,
  to: number,
): { distance: number; activity: geo.Activity } | null {
  const stretch = track.filter((point) => point.at >= from && point.at <= to);
  if (stretch.length < 2) return null;
  let distance = 0;
  const time = new Map<geo.Activity, number>();
  for (let index = 1; index < stretch.length; index++) {
    const [a, b] = [stretch[index - 1]!, stretch[index]!];
    distance += distanceMeters(a, b);
    if (b.activity !== geo.Activity.UNSPECIFIED && b.activity !== geo.Activity.STILL)
      time.set(b.activity, (time.get(b.activity) ?? 0) + (b.at - a.at));
  }
  const said = [...time.entries()].sort((a, b) => b[1] - a[1])[0]?.[0];
  const seconds = Math.max(1, (stretch.at(-1)!.at - stretch[0]!.at) / 1000);
  return { distance, activity: said ?? activityForSpeed(distance / seconds) };
}

/**
 * The day as a story: each visit in order, and between two visits how the
 * person got from one to the other. With the route (`track`) the distance
 * and the way of travelling are known; without it only that they moved,
 * and roughly how far (the straight line).
 */
export function buildTimeline(visits: geo.Visit[], track: TrackPoint[]): TimelineItem[] {
  const ordered = [...visits].sort((a, b) => toMillis(a.arrivedAt) - toMillis(b.arrivedAt));
  const items: TimelineItem[] = [];
  ordered.forEach((visit, index) => {
    const previous = ordered[index - 1];
    if (previous) {
      const from =
        toMillis(previous.departedAt) ||
        toMillis(previous.arrivedAt) + Number(previous.durationSeconds) * 1000;
      const to = toMillis(visit.arrivedAt);
      const measured = movement(track, from, to);
      const straight =
        previous.center && visit.center ? distanceMeters(previous.center, visit.center) : 0;
      const distance = measured?.distance ?? straight;
      if (to > from && (to - from >= MIN_MOVE_MS || distance >= MIN_MOVE_METERS)) {
        items.push({
          kind: "move",
          from,
          to,
          activity: measured?.activity ?? geo.Activity.UNSPECIFIED,
          distanceMeters: distance,
        });
      }
    }
    items.push({ kind: "visit", visit });
  });
  return items;
}

/**
 * Where the replay is at a moment: the point reached and how many points of
 * the track lie behind it. `track` must be sorted by time.
 */
export function positionAt(
  track: TrackPoint[],
  at: number,
): { lat: number; lng: number; index: number } | null {
  if (track.length === 0) return null;
  if (at <= track[0]!.at) return { lat: track[0]!.lat, lng: track[0]!.lng, index: 0 };
  const last = track.at(-1)!;
  if (at >= last.at) return { lat: last.lat, lng: last.lng, index: track.length - 1 };
  let [low, high] = [0, track.length - 1];
  while (high - low > 1) {
    const middle = (low + high) >> 1;
    if (track[middle]!.at <= at) low = middle;
    else high = middle;
  }
  const [a, b] = [track[low]!, track[high]!];
  const share = b.at === a.at ? 0 : (at - a.at) / (b.at - a.at);
  return { lat: a.lat + (b.lat - a.lat) * share, lng: a.lng + (b.lng - a.lng) * share, index: low };
}
