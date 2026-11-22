/**
 * What the places feature shares with other features.
 *
 * Places:
 *
 *   <CheckInDialog open={open} onOpenChange={setOpen} target={{ placeId, name }} />   // no target = "here"
 *   <PlaceBubble place={place} />        // a place's emoji in its sticker bubble
 *   <GazetteerCredit />                  // the credit line required next to area names from the server
 */
export {
  currentPosition,
  DEFAULT_CENTER,
  FRIENDS_KEY,
  type LatLng,
  loadFriends,
  localDate,
  locationClient,
  placeClient,
  placeKeys,
  toLatLng,
} from "./api";
export { CheckInDialog, type CheckInTarget } from "./check-in-dialog";
export { GazetteerCredit } from "./credit";
export {
  forgetDeviceMemory,
  opaqueHash,
  readDeviceMemory,
  writeDeviceMemory,
} from "./device-memory";
export { labelMeta, placeEmoji } from "./labels";
export { PlaceBubble, PlaceRow } from "./place-row";
