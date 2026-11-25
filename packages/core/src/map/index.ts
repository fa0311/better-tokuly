/**
 * Map logic every app shares: query keys, the rules of a friend's pin (live,
 * stale, moving, blurred, ghost mode), screen clustering and fanning,
 * polylines and distances, the edits that keep live locations current, the
 * rules of reporting the device's own fixes and the map style. The map view,
 * the camera and reading the device's position stay in each app.
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
export {
  finiteOrUndefined,
  isMovingFix,
  type LastReport,
  locationSample,
  MOVED_METERS,
  type OwnPosition,
  type ReportCadence,
  type ReportDecision,
  reportDecision,
  type SampleContext,
} from "./reporting";
export { buildMapStyle, type MapStyleId, TILE_ZOOMS, type TileConfig } from "./style";
