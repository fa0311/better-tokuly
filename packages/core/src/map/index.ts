/**
 * Map logic every app shares: query keys, the rules of a friend's pin (live,
 * stale, moving, blurred, ghost mode), screen clustering and fanning,
 * polylines and distances, and the edits that keep live locations current.
 * The map view, camera and the device's own position stay in each app.
 */
export { mapKeys } from "./keys";
export { fromSnapshot, type LiveLocations, withLocation, withoutFriend } from "./live";
export {
  BLURRED_STALE_AFTER_MS,
  BUMP_RADIUS_M,
  BUZZ_RADIUS_M,
  byDistance,
  circleRing,
  clusterOnScreen,
  decodePolyline,
  distanceTo,
  fanOffset,
  ghostRuleFor,
  ghostSummary,
  isBlurred,
  isMoving,
  MOVING_SPEED_MPS,
  type PinState,
  pinState,
  type ScreenPoint,
  STALE_AFTER_MS,
  seeEachOther,
  stayMillis,
} from "./model";
