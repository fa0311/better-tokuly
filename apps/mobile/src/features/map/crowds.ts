import type { geo } from "@tokuly/api";
import { clusterOnScreen } from "@tokuly/core/map";

/** MapLibre's world at zoom 0 is one tile of 512 points. */
const WORLD = 512;
/** Pins closer than this on screen are drawn as one stack (the web's distance). */
const CROWD_POINTS = 46;
/** Zoomed in this far, pins still in one stack are in the same place: no zoom will part them. */
export const CLOSEST_ZOOM = 17;

/** Friends whose pins would cover each other at the current zoom, drawn as one stack. */
export interface Crowd {
  /** The members' IDs, sorted: the same crowd keeps its key while the map moves. */
  key: string;
  lng: number;
  lat: number;
  members: geo.FriendLocation[];
}

/**
 * Where a position is on the flat map at `zoom`, in points. Only distances
 * between two of these are used, so the camera's centre and bearing do not
 * matter (and the slight tilt is ignored).
 */
function worldPoint(position: { lat: number; lng: number }, zoom: number) {
  const size = WORLD * 2 ** zoom;
  const sin = Math.sin((Math.max(-85, Math.min(85, position.lat)) * Math.PI) / 180);
  return {
    x: ((position.lng + 180) / 360) * size,
    y: (0.5 - Math.log((1 + sin) / (1 - sin)) / (4 * Math.PI)) * size,
  };
}

/**
 * The pins that overlap at `zoom`, grouped; each stack sits in the middle of
 * its members. `apart` (the friend whose card is open) is never stacked.
 */
export function crowdsAt(pins: geo.FriendLocation[], zoom: number, apart?: string | null): Crowd[] {
  const placed = pins.flatMap((location) =>
    location.position && location.userId !== apart
      ? [{ location, position: location.position }]
      : [],
  );
  const byId = new Map(placed.map((pin) => [pin.location.userId, pin]));
  const groups = clusterOnScreen(
    placed.map((pin) => ({ id: pin.location.userId, ...worldPoint(pin.position, zoom) })),
    CROWD_POINTS,
  );
  return groups.map((ids) => {
    const members = ids.flatMap((id) => byId.get(id) ?? []);
    return {
      key: ids.join(","),
      lng: members.reduce((sum, pin) => sum + pin.position.lng, 0) / members.length,
      lat: members.reduce((sum, pin) => sum + pin.position.lat, 0) / members.length,
      members: members.map((pin) => pin.location),
    };
  });
}

/** `[west, south, east, north]` around a crowd, for the camera. */
export function boundsOf(crowd: Crowd): [number, number, number, number] {
  const lngs = crowd.members.map((member) => member.position?.lng ?? crowd.lng);
  const lats = crowd.members.map((member) => member.position?.lat ?? crowd.lat);
  return [Math.min(...lngs), Math.min(...lats), Math.max(...lngs), Math.max(...lats)];
}
