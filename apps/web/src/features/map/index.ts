/**
 * What the map feature shares with other features.
 *
 * `<MapView>` is a map that fills its positioned parent. For most screens
 * the props are enough:
 *
 *   <div className="relative h-64">
 *     <MapView
 *       markers={[{ id: place.id, lat, lng, emoji: "☕", label: place.name, onClick: () => openPlace(place.id) }]}
 *       route={{ polyline: day.pathPolyline }}        // or { points: [{ lat, lng }, …] }, or several routes
 *       fitTo="content"                               // keep markers and routes in view; or pass points
 *     />
 *   </div>
 *
 * | prop            | default         | meaning |
 * |-----------------|-----------------|---------|
 * | `styleId`       | `"tokuly"`      | map colours (`"tokuly" | "light" | "dark" | "mono"`); all follow the app theme |
 * | `initialCenter` | Tokyo           | `[lng, lat]` the map opens at |
 * | `initialZoom`   | `11`            | zoom the map opens at |
 * | `markers`       | none            | `MapMarker[]`: `{ id, lat, lng, user? | emoji?, label?, onClick? }` |
 * | `route`         | none            | `MapRoute | MapRoute[]`: `{ polyline? | points?, color?, dashed? }` |
 * | `fitTo`         | none            | `MapPoint[] | "content"`: moves the map to show them whenever they change |
 * | `interactive`   | `true`          | `false` = no panning, zooming or keyboard |
 * | `className`     | `absolute inset-0` | classes of the wrapper |
 * | `children`      | none            | rendered once the map is ready; may use the pieces below |
 *
 * Inside `<MapView>` children can use `useMap()` (the MapLibre map),
 * `<MapPin lat lng>` (any React content as a pin that glides when its
 * position changes), `<RouteLine id polyline|points>`,
 * `useMapLayer(id, geojson, layers)` (a GeoJSON source that survives theme
 * changes) and `fitMapToPoints(map, points, { maxZoom?, padding?, animate? })`.
 * `decodePolyline(encoded)` turns the API's encoded polylines into
 * `[lng, lat]` pairs.
 *
 * A map of your own in a box, with the pieces most screens need:
 *
 *   <MapFrame center={{ lat, lng }} zoom={14} className="h-64 rounded-card">
 *     <GeoLayer id="area" data={geojson} layers={[{ id: "area-fill", type: "fill", paint: { … } }]} />
 *     <Pin position={{ lat, lng }} onDragEnd={(position) => …}>…any React content…</Pin>   // a pin that can be dragged
 *     <FitTo points={points} fitKey={date} />
 *     <MapClick onClick={(position) => …} />
 *   </MapFrame>
 *
 * and `circlePolygon(center, meters)`, `zoomForRadius(meters)`,
 * `boundsOf(points)`, `cameraMs(ms)` (camera moves that follow ドパガキ度).
 *
 * `<StaticMapPreview lat lng>` is a picture of the map around a point with a
 * pin on it — for place cards, history entries, location messages and place
 * tags. It is an image, cheap enough for long lists:
 *
 *   <StaticMapPreview lat={visit.center.lat} lng={visit.center.lng} label={visit.name} className="h-32 rounded-2xl" />
 *
 * | prop        | default            | meaning |
 * |-------------|--------------------|---------|
 * | `lat`, `lng`| required           | the point in the middle |
 * | `zoom`      | `15`               | how close |
 * | `pin`       | a 📍 sticker       | what sits on the point; `null` for nothing |
 * | `label`     | the coordinates    | read aloud instead of the picture |
 * | `className` | full width, 16:9   | size and corners of the box |
 *
 * A link to `/map?friend=<user id>` opens the live map on that friend.
 */

export {
  boundsOf,
  cameraMs,
  circlePolygon,
  FitTo,
  type GeoData,
  type GeoFeature,
  type GeoFeatures,
  type GeoGeometry,
  GeoLayer,
  type GeoLayerStyle,
  type GeoRing,
  MapClick,
  MapFrame,
  Pin,
  zoomForRadius,
} from "./kit";
export {
  DEFAULT_CENTER,
  fitMapToPoints,
  type MapMarker,
  MapPin,
  type MapPinProps,
  type MapPoint,
  type MapRoute,
  MapView,
  type MapViewProps,
  RouteLine,
  useIsDark,
  useMap,
  useMapLayer,
} from "./map-view";
export { decodePolyline } from "./model";
export { StaticMapPreview, type StaticMapPreviewProps } from "./static-map-preview";
export type { MapStyleId } from "./tiles";
