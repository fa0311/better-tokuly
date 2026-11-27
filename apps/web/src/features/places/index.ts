/**
 * What the places feature shares with other features (its logic is
 * `@tokuly/core/places`).
 *
 * Places:
 *
 *   <CheckInDialog open={open} onOpenChange={setOpen} target={{ placeId, name }} />   // no target = "here"
 *   <PlaceBubble place={place} />        // a place's emoji in its sticker bubble
 *   <GazetteerCredit />                  // the credit line required next to area names from the server
 */

export { readDeviceMemory, writeDeviceMemory } from "#/lib/device-memory";
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
export { PlaceBubble, PlaceRow } from "./place-row";
