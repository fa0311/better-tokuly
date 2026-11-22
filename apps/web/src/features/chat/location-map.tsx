import { layers, namedFlavor } from "@protomaps/basemaps";
import { Map as MapLibreMap, type StyleSpecification, setWorkerUrl } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
// The map parses tiles in a module worker, emitted by the bundler (same as the main map).
import mapWorkerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url";
import { MapPin } from "lucide-react";
import { useEffect, useRef } from "react";

setWorkerUrl(mapWorkerUrl);

const TILES_URL: string = import.meta.env.VITE_TILES_URL ?? "/tiles";
const ASSETS_URL: string = import.meta.env.VITE_MAP_ASSETS_URL ?? "/map-assets";
/** Highest zoom in the planet-wide archive; the regional one takes over above it. */
const WORLD_MAX_ZOOM = 6;
const REGION_MAX_ZOOM = 15;

/** A plain street map in the app's two themes, from the deployment's own tiles. */
function mapStyle(dark: boolean, language: string): StyleSpecification {
  const absolute = (url: string) => (url.startsWith("http") ? url : `${window.location.origin}${url}`);
  const flavor = dark
    ? { ...namedFlavor("dark"), background: "#14111f", earth: "#1b1730", water: "#17304d", buildings: "#262040" }
    : { ...namedFlavor("light"), background: "#ece8fb", earth: "#f3f0ff", water: "#a9dcff", park_a: "#c9f0d0", park_b: "#b7ebc4", buildings: "#e2dcf5" };
  const handover = WORLD_MAX_ZOOM + 1;
  const world = layers("world", flavor, { lang: language }).map((layer) => ({ ...layer, id: `world-${layer.id}`, ...(layer.type === "background" ? {} : { maxzoom: handover }) }));
  const region = layers("region", flavor, { lang: language })
    .filter((layer) => layer.type !== "background")
    .map((layer) => ({ ...layer, id: `region-${layer.id}`, minzoom: Math.max(handover, "minzoom" in layer ? (layer.minzoom ?? 0) : 0) }));
  return {
    version: 8,
    glyphs: `${absolute(ASSETS_URL)}/fonts/{fontstack}/{range}.pbf`,
    sprite: `${absolute(ASSETS_URL)}/sprites/v4/${dark ? "dark" : "light"}`,
    sources: {
      world: { type: "vector", tiles: [`${absolute(TILES_URL)}/world/{z}/{x}/{y}.mvt`], maxzoom: WORLD_MAX_ZOOM, attribution: '<a href="https://protomaps.com">Protomaps</a> © <a href="https://openstreetmap.org">OpenStreetMap</a>' },
      region: { type: "vector", tiles: [`${absolute(TILES_URL)}/region/{z}/{x}/{y}.mvt`], maxzoom: REGION_MAX_ZOOM },
    },
    // biome-ignore lint/suspicious/noExplicitAny: the generated layers are valid; spreading them loses the union type.
    layers: [...world, ...region] as any,
  };
}

/**
 * A small map with a pin in its middle. With `onMove` the pin stays put while
 * the map is dragged under it and every stop reports the new centre (choosing
 * a place); without it the map simply shows the place.
 *
 * Loaded on demand (`lazy(() => import("./location-map"))`): the map library
 * is big, and chat only needs it when a place is opened or shared.
 */
export default function LocationMap({ lat, lng, onMove, language }: { lat: number; lng: number; onMove?(position: { lat: number; lng: number }): void; language: string }) {
  const container = useRef<HTMLDivElement>(null);
  const map = useRef<MapLibreMap | null>(null);
  const report = useRef(onMove);
  useEffect(() => {
    report.current = onMove;
  }, [onMove]);

  // Created once; a new position moves the existing map below.
  useEffect(() => {
    if (!container.current) return;
    const instance = new MapLibreMap({
      container: container.current,
      style: mapStyle(document.documentElement.dataset.theme === "dark", language.startsWith("ja") ? "ja" : "en"),
      center: [lng, lat],
      zoom: 15,
      minZoom: 2,
      maxZoom: 19,
      attributionControl: { compact: true },
      dragRotate: false,
      pitchWithRotate: false,
    });
    instance.on("moveend", () => {
      const centre = instance.getCenter();
      report.current?.({ lat: centre.lat, lng: centre.lng });
    });
    map.current = instance;
    return () => {
      map.current = null;
      instance.remove();
    };
  }, []);

  // A position set from outside (the device's own, once it is known) recentres the map.
  useEffect(() => {
    const instance = map.current;
    if (!instance) return;
    const centre = instance.getCenter();
    if (Math.abs(centre.lat - lat) > 1e-6 || Math.abs(centre.lng - lng) > 1e-6) instance.jumpTo({ center: [lng, lat] });
  }, [lat, lng]);

  return (
    <div className="relative size-full" data-testid="location-map">
      <div ref={container} className="size-full" />
      <span className="pointer-events-none absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-full text-pink drop-shadow-[0_2px_0_var(--shadow)]">
        <MapPin className="size-10 fill-current stroke-[#17122b] stroke-[1.5]" />
      </span>
    </div>
  );
}
