/**
 * What the places feature shares with other features.
 *
 * Drawing on the Tokuly map (everything goes inside `<MapFrame>`, or any
 * `<MapView>`):
 *
 *   <MapFrame center={{ lat, lng }} zoom={14} className="h-64 rounded-card">
 *     <GeoLayer id="route" data={geojson} layers={[{ id: "route-line", type: "line", paint: { … } }]} />
 *     <Pin position={{ lat, lng }} onDragEnd={(position) => …}>…any React content…</Pin>
 *     <FitTo points={points} fitKey={date} />
 *     <MapClick onClick={(position) => …} />
 *   </MapFrame>
 *
 * Places:
 *
 *   <CheckInDialog open={open} onOpenChange={setOpen} target={{ placeId, name }} />   // no target = "here"
 *   <PlaceBubble place={place} />        // a place's emoji in its sticker bubble
 *   <GazetteerCredit />                  // the credit line required next to area names from the server
 */
export { currentPosition, DEFAULT_CENTER, deviceTimeZone, FRIENDS_KEY, type LatLng, loadFriends, localDate, locationClient, placeClient, placeKeys, toLatLng } from "./api";
export { CheckInDialog, type CheckInTarget } from "./check-in-dialog";
export { GazetteerCredit } from "./credit";
export { labelMeta, placeEmoji } from "./labels";
export { boundsOf, cameraMs, circlePolygon, FitTo, type GeoData, type GeoFeature, type GeoFeatures, type GeoGeometry, GeoLayer, type GeoLayerStyle, type GeoRing, MapClick, MapFrame, Pin, useIsDark, useMap, zoomForRadius } from "./map-kit";
export { PlaceBubble, PlaceRow } from "./place-row";
