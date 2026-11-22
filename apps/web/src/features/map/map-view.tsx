import type { common } from "@tokuly/api";
import type { GeoJSONSource, LayerSpecification } from "maplibre-gl";
import { Map as MapLibreMap, Marker, setWorkerUrl } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
// The map parses tiles in a module worker. Let the bundler emit that worker
// (with its imports) and tell the library where it ended up.
import mapWorkerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url";
import { createContext, type ReactNode, useContext, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { cn } from "#/lib/cn";
import { decodePolyline } from "./model";
import { buildMapStyle, type MapStyleId } from "./tiles";

setWorkerUrl(mapWorkerUrl);

const MapContext = createContext<MapLibreMap | null>(null);

/** Tokyo station: where a map opens when nothing better is known. */
export const DEFAULT_CENTER: [number, number] = [139.767125, 35.681236];

/** The live map instance. Only call inside `<MapView>` children. */
export function useMap(): MapLibreMap {
  const map = useContext(MapContext);
  if (!map) throw new Error("useMap must be used inside <MapView>");
  return map;
}

/** Follows the resolved theme on <html data-theme>. */
export function useIsDark(): boolean {
  const [dark, setDark] = useState(() => document.documentElement.dataset.theme === "dark");
  useEffect(() => {
    const root = document.documentElement;
    const observer = new MutationObserver(() => setDark(root.dataset.theme === "dark"));
    observer.observe(root, { attributes: true, attributeFilter: ["data-theme"] });
    return () => observer.disconnect();
  }, []);
  return dark;
}

/** A coordinate as the API gives it (`{ lat, lng }`). */
export interface MapPoint {
  lat: number;
  lng: number;
}

/** A simple marker for `<MapView markers>`. */
export interface MapMarker extends MapPoint {
  id: string;
  /** A face as the pin; without it the pin is `emoji` (default 📍) on a sticker. */
  user?: common.UserSummary;
  emoji?: string;
  /** Short text under the pin. */
  label?: string;
  /** Makes the pin a button. */
  onClick?(): void;
}

/** A line on the map: an encoded polyline (precision 5, as the API returns routes) or raw points. */
export interface MapRoute {
  polyline?: string;
  points?: MapPoint[];
  /** A CSS colour; default is the brand purple. */
  color?: string;
  dashed?: boolean;
}

export interface MapViewProps {
  /** Map colours. Default `"tokuly"`; all of them follow the app's light / dark theme. */
  styleId?: MapStyleId;
  /** Where the map opens: `[lng, lat]`. Default: Tokyo. */
  initialCenter?: [number, number];
  /** Default 11 (a city). */
  initialZoom?: number;
  /** Pins drawn for you. For anything richer render `<MapPin>` children. */
  markers?: MapMarker[];
  /** One or more lines (a route, a day's path). */
  route?: MapRoute | MapRoute[];
  /**
   * Keeps these points in view: the map moves to show all of them whenever
   * the list changes. `"content"` fits the markers and routes given above.
   */
  fitTo?: MapPoint[] | "content";
  /** `false` makes the map a picture: no panning, zooming or keyboard. Default `true`. */
  interactive?: boolean;
  /** Classes for the wrapper. Default: fill the positioned parent (`absolute inset-0`). */
  className?: string;
  /** Rendered once the style has loaded; can reach the map with `useMap()`. */
  children?: ReactNode;
}

/**
 * The map canvas. Children render once the style has loaded and can reach
 * the map with `useMap()` — markers, overlays and controls are all children.
 * Plain uses need no children: pass `markers`, `route` and `fitTo`.
 */
export function MapView({
  styleId = "tokuly",
  initialCenter = DEFAULT_CENTER,
  initialZoom = 11,
  markers,
  route,
  fitTo,
  interactive = true,
  className,
  children,
}: MapViewProps) {
  const container = useRef<HTMLDivElement>(null);
  const [map, setMap] = useState<MapLibreMap | null>(null);
  const { i18n } = useTranslation();
  const dark = useIsDark();
  const language = i18n.language.startsWith("ja") ? "ja" : "en";
  const appliedStyle = useRef<string>("");

  // Create the map once. Later style changes go through setStyle below.
  useEffect(() => {
    if (!container.current) return;
    const key = `${styleId}:${dark}:${language}`;
    appliedStyle.current = key;
    const instance = new MapLibreMap({
      container: container.current,
      style: buildMapStyle(styleId, dark, language),
      center: initialCenter,
      zoom: initialZoom,
      minZoom: 1.2,
      maxZoom: 19,
      maxPitch: 60,
      attributionControl: { compact: true },
      interactive,
      // Rotation with two fingers / right-drag stays on; the keyboard handler too.
      dragRotate: interactive,
    });
    instance.once("load", () => setMap(instance));
    return () => {
      setMap(null);
      instance.remove();
    };
  }, []);

  useEffect(() => {
    if (!map) return;
    const key = `${styleId}:${dark}:${language}`;
    if (appliedStyle.current === key) return;
    appliedStyle.current = key;
    map.setStyle(buildMapStyle(styleId, dark, language));
  }, [map, styleId, dark, language]);

  const routes = route ? (Array.isArray(route) ? route : [route]) : [];
  return (
    <div className={cn(className ?? "absolute inset-0")}>
      {/* Sized, not positioned: the map library sets `position: relative` on its container. */}
      <div ref={container} className="size-full" />
      {map && (
        <MapContext.Provider value={map}>
          {routes.map((line, index) => (
            <RouteLine key={index} id={`route-${index}`} {...line} />
          ))}
          {markers?.map((marker) => (
            <SimpleMarker key={marker.id} marker={marker} />
          ))}
          {fitTo && <FitTo points={fitTo === "content" ? contentPoints(markers, routes) : fitTo} />}
          {children}
        </MapContext.Provider>
      )}
    </div>
  );
}

function routePoints(route: MapRoute): [number, number][] {
  if (route.points) return route.points.map((point) => [point.lng, point.lat]);
  return route.polyline ? decodePolyline(route.polyline) : [];
}

function contentPoints(markers: MapMarker[] | undefined, routes: MapRoute[]): MapPoint[] {
  return [
    ...(markers ?? []),
    ...routes.flatMap((line) => routePoints(line).map(([lng, lat]) => ({ lat, lng }))),
  ];
}

/**
 * Moves the map so that every point is in view (one point: centres on it).
 * Respects the map's padding, so pins never end up under floating panels.
 */
export function fitMapToPoints(
  map: MapLibreMap,
  points: MapPoint[],
  options: { maxZoom?: number; padding?: number; animate?: boolean } = {},
): void {
  if (points.length === 0) return;
  const { maxZoom = 16, padding = 72, animate = true } = options;
  const lngs = points.map((point) => point.lng);
  const lats = points.map((point) => point.lat);
  const west = Math.min(...lngs);
  const east = Math.max(...lngs);
  const south = Math.min(...lats);
  const north = Math.max(...lats);
  if (west === east && south === north) {
    map.easeTo({
      center: [west, south],
      zoom: Math.max(map.getZoom(), Math.min(maxZoom, 15)),
      duration: animate ? 700 : 0,
      essential: true,
    });
    return;
  }
  map.fitBounds(
    [
      [west, south],
      [east, north],
    ],
    { padding, maxZoom, duration: animate ? 900 : 0, essential: true },
  );
}

function FitTo({ points }: { points: MapPoint[] }) {
  const map = useMap();
  const first = useRef(true);
  const signature = points
    .map((point) => `${point.lat.toFixed(5)},${point.lng.toFixed(5)}`)
    .join(";");
  useEffect(() => {
    fitMapToPoints(map, points, { animate: !first.current });
    first.current = false;
  }, [map, signature]);
  return null;
}

/**
 * Keeps a GeoJSON source and its layers on the map. Changing the theme
 * replaces the map's style, which drops everything added by hand, so the
 * source is put back whenever the style changes.
 */
/** GeoJSON as the map takes it. */
export type MapGeoJson = Exclude<Parameters<GeoJSONSource["setData"]>[0], string>;

export function useMapLayer(
  id: string,
  data: MapGeoJson,
  layers: Omit<LayerSpecification, "source">[],
): void {
  const map = useMap();
  const latest = useRef({ data, layers });
  latest.current = { data, layers };

  useEffect(() => {
    const ensure = () => {
      try {
        if (!map.getSource(id)) map.addSource(id, { type: "geojson", data: latest.current.data });
        for (const layer of latest.current.layers) {
          if (!map.getLayer(layer.id)) map.addLayer({ ...layer, source: id } as LayerSpecification);
        }
      } catch {
        // The style is still loading; `styledata` fires again when it is ready.
      }
    };
    ensure();
    map.on("styledata", ensure);
    return () => {
      map.off("styledata", ensure);
      // The map may already be gone when the whole view unmounts.
      try {
        for (const layer of latest.current.layers)
          if (map.getLayer(layer.id)) map.removeLayer(layer.id);
        if (map.getSource(id)) map.removeSource(id);
      } catch {
        // Nothing to clean up.
      }
    };
  }, [map, id]);

  useEffect(() => {
    (map.getSource(id) as GeoJSONSource | undefined)?.setData(data);
  }, [map, id, data]);
}

/** A line on the map. Inside `<MapView>` only. */
export function RouteLine({ id, color = "#7c5cff", dashed, ...route }: MapRoute & { id: string }) {
  const coordinates = routePoints(route);
  useMapLayer(
    `tokuly-${id}`,
    { type: "Feature", properties: {}, geometry: { type: "LineString", coordinates } },
    [
      {
        id: `tokuly-${id}-casing`,
        type: "line",
        layout: { "line-cap": "round", "line-join": "round" },
        paint: { "line-color": "#17122b", "line-width": 8, "line-opacity": dashed ? 0 : 0.9 },
      },
      {
        id: `tokuly-${id}-line`,
        type: "line",
        layout: { "line-cap": "round", "line-join": "round" },
        paint: {
          "line-color": color,
          "line-width": 5,
          ...(dashed ? { "line-dasharray": [1, 1.6] } : {}),
        },
      },
    ],
  );
  return null;
}

/** How long a pin takes to settle on a new position when nothing is known about the pace of updates. */
const GLIDE_MS = 900;
/** A pin never spends longer than this on one glide: after it the next sample is due. */
const MAX_GLIDE_MS = 5_000;
/** Farther than this (in degrees, roughly 5 km) a pin jumps instead of sweeping across the map. */
const JUMP_DEGREES = 0.05;

function animationsOff(): boolean {
  return (
    window.matchMedia("(prefers-reduced-motion: reduce)").matches ||
    document.documentElement.dataset.dopa === "0"
  );
}

export interface MapPinProps extends MapPoint {
  zIndex?: number;
  /** Shifts the pin on screen by `[x, y]` pixels (used when a crowd of pins fans out). */
  offset?: [number, number];
  /**
   * `"steady"` keeps moving at an even pace for about as long as the last
   * update took to arrive: right for someone who is travelling, so the pin
   * flows along instead of stopping and starting. Default: ease into place.
   */
  glide?: "settle" | "steady";
  /** Which point of the content sits on the coordinate. Default `"bottom"` (a pin with a tail). */
  anchor?: "bottom" | "center";
  children: ReactNode;
}

/**
 * Keeps a MapLibre marker at a coordinate, gliding there when it changes, and
 * renders React children into it.
 */
export function MapPin({
  lng,
  lat,
  zIndex,
  offset,
  glide = "settle",
  anchor = "bottom",
  children,
}: MapPinProps) {
  const map = useMap();
  const [element] = useState(() => document.createElement("div"));
  const marker = useRef<Marker | null>(null);
  const shown = useRef<[number, number] | null>(null);
  const lastChange = useRef(0);

  useEffect(() => {
    const instance = new Marker({ element, anchor });
    marker.current = instance;
    return () => {
      instance.remove();
      marker.current = null;
      shown.current = null;
    };
  }, [element, anchor]);

  useEffect(() => {
    const instance = marker.current;
    if (!instance) return;
    const from = shown.current;
    const now = performance.now();
    const sinceLast = now - lastChange.current;
    lastChange.current = now;
    if (!from) {
      // First position: appear in place.
      instance.setLngLat([lng, lat]).addTo(map);
      shown.current = [lng, lat];
      return;
    }
    if (animationsOff() || Math.abs(lng - from[0]) + Math.abs(lat - from[1]) > JUMP_DEGREES) {
      instance.setLngLat([lng, lat]);
      shown.current = [lng, lat];
      return;
    }
    const steady = glide === "steady";
    const duration = steady ? Math.min(MAX_GLIDE_MS, Math.max(GLIDE_MS, sinceLast)) : GLIDE_MS;
    let frame = 0;
    const step = (time: number) => {
      const t = Math.min(1, (time - now) / duration);
      const eased = steady ? t : 1 - (1 - t) ** 3;
      const position: [number, number] = [
        from[0] + (lng - from[0]) * eased,
        from[1] + (lat - from[1]) * eased,
      ];
      instance.setLngLat(position);
      shown.current = position;
      if (t < 1) frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);
    return () => cancelAnimationFrame(frame);
  }, [map, lng, lat, glide]);

  const [offsetX, offsetY] = offset ?? [0, 0];
  useEffect(() => {
    marker.current?.setOffset([offsetX, offsetY]);
  }, [offsetX, offsetY]);

  useEffect(() => {
    element.style.zIndex = String(zIndex ?? 1);
  }, [element, zIndex]);

  return createPortal(children, element);
}

function SimpleMarker({ marker }: { marker: MapMarker }) {
  const body = (
    <>
      {marker.user ? (
        <span className="sticker block rounded-[1.25rem] bg-surface p-[3px]">
          <Avatar user={marker.user} size="lg" className="block [&>span]:rounded-[1rem]" />
        </span>
      ) : (
        <span className="sticker grid size-11 place-items-center rounded-2xl bg-surface text-2xl">
          {marker.emoji ?? "📍"}
        </span>
      )}
      <span className="h-2.5 w-[3px] bg-ink" aria-hidden />
      {marker.label && (
        <span className="max-w-40 truncate rounded-full bg-ink px-2.5 py-0.5 text-[11px] font-black text-surface">
          {marker.label}
        </span>
      )}
    </>
  );
  return (
    <MapPin lng={marker.lng} lat={marker.lat}>
      {marker.onClick ? (
        <button
          type="button"
          onClick={marker.onClick}
          aria-label={marker.label ?? marker.user?.displayName}
          className="pressable flex flex-col items-center"
        >
          {body}
        </button>
      ) : (
        <div className="flex flex-col items-center">{body}</div>
      )}
    </MapPin>
  );
}
