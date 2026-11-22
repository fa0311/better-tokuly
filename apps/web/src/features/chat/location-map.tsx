import { MapPin } from "lucide-react";
import { useEffect, useRef } from "react";
import { MapView, useMap } from "#/features/map";

/**
 * A small map with a pin in its middle, drawn by the app's map
 * (`features/map`). With `onMove` the pin stays put while the map is dragged
 * under it and every stop reports the new centre (choosing a place); without
 * it the map simply shows the place.
 *
 * Loaded on demand (`lazy(() => import("./location-map"))`): the map library
 * is big, and chat only needs it when a place is opened or shared.
 */
export default function LocationMap({
  lat,
  lng,
  onMove,
}: {
  lat: number;
  lng: number;
  onMove?(position: { lat: number; lng: number }): void;
}) {
  return (
    <div className="relative size-full" data-testid="location-map">
      <MapView initialCenter={[lng, lat]} initialZoom={15}>
        <Centre lat={lat} lng={lng} onMove={onMove} />
      </MapView>
      <span className="pointer-events-none absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-full text-pink drop-shadow-[0_2px_0_var(--shadow)]">
        <MapPin className="size-10 fill-current stroke-[#17122b] stroke-[1.5]" />
      </span>
    </div>
  );
}

/** Keeps the map's centre and the chosen position in step, both ways. */
function Centre({
  lat,
  lng,
  onMove,
}: {
  lat: number;
  lng: number;
  onMove?(position: { lat: number; lng: number }): void;
}) {
  const map = useMap();
  const report = useRef(onMove);
  useEffect(() => {
    report.current = onMove;
  }, [onMove]);

  useEffect(() => {
    const moved = () => {
      const centre = map.getCenter();
      report.current?.({ lat: centre.lat, lng: centre.lng });
    };
    map.on("moveend", moved);
    return () => {
      map.off("moveend", moved);
    };
  }, [map]);

  // A position set from outside (the device's own, once it is known) recentres the map.
  useEffect(() => {
    const centre = map.getCenter();
    if (Math.abs(centre.lat - lat) > 1e-6 || Math.abs(centre.lng - lng) > 1e-6)
      map.jumpTo({ center: [lng, lat] });
  }, [map, lat, lng]);

  return null;
}
