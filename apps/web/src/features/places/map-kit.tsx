import type { GeoJSONSource, LayerSpecification, LngLatBoundsLike, MapMouseEvent } from "maplibre-gl";
import { Marker } from "maplibre-gl";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { MapView, useMap } from "#/features/map/map-view";
import { dopamineFactor } from "#/lib/appearance";
import { cn } from "#/lib/cn";
import type { LatLng } from "./api";

export { useIsDark, useMap } from "#/features/map/map-view";

/**
 * A box with the Tokuly map in it. `<MapView>` fills its nearest positioned
 * ancestor, so this gives it one with a size; children are map children
 * (layers, pins, controls) and may call `useMap()`.
 */
export function MapFrame({ center, zoom = 14, className, children }: { center: LatLng; zoom?: number; className?: string; children?: ReactNode }) {
  return (
    <div className={cn("relative isolate overflow-hidden bg-surface-2", className)}>
      <MapView styleId="tokuly" initialCenter={[center.lng, center.lat]} initialZoom={zoom}>
        {children}
      </MapView>
    </div>
  );
}

/** A ring of `[lng, lat]` positions. */
export type GeoRing = [number, number][];

/** The few GeoJSON shapes the app draws, spelled out (the full GeoJSON typings are not a dependency). */
export type GeoGeometry =
  | { type: "Polygon"; coordinates: GeoRing[] }
  | { type: "MultiPolygon"; coordinates: GeoRing[][] }
  | { type: "LineString"; coordinates: GeoRing };

export interface GeoFeature<G extends GeoGeometry = GeoGeometry> {
  type: "Feature";
  properties: Record<string, unknown>;
  geometry: G;
}

export interface GeoFeatures<G extends GeoGeometry = GeoGeometry> {
  type: "FeatureCollection";
  features: GeoFeature<G>[];
}

/** What a `<GeoLayer>` draws. */
export type GeoData = GeoFeature | GeoFeatures;

type SourceData = Parameters<GeoJSONSource["setData"]>[0];

/** One style layer of a `<GeoLayer>`: a MapLibre layer without its source, which the component fills in. */
export interface GeoLayerStyle {
  id: string;
  type: "fill" | "line" | "circle" | "symbol";
  paint?: Record<string, unknown>;
  layout?: Record<string, unknown>;
  filter?: unknown[];
}

/**
 * Draws GeoJSON on the map with the given style layers (ids must be unique
 * on the map). The basemap style is replaced when the theme or language
 * changes, which drops everything added to it, so the source and layers are
 * put back whenever they are missing.
 */
export function GeoLayer({ id, data, layers }: { id: string; data: GeoData; layers: GeoLayerStyle[] }) {
  const map = useMap();
  const latest = useRef({ data, layers });
  latest.current = { data, layers };

  useEffect(() => {
    const ensure = () => {
      try {
        if (!map.getSource(id)) map.addSource(id, { type: "geojson", data: latest.current.data as SourceData });
        for (const layer of latest.current.layers) {
          if (!map.getLayer(layer.id)) map.addLayer({ ...layer, source: id } as LayerSpecification);
        }
      } catch {
        // The new style is still loading; `styledata` fires again when it is in.
      }
    };
    ensure();
    map.on("styledata", ensure);
    map.on("style.load", ensure);
    return () => {
      map.off("styledata", ensure);
      map.off("style.load", ensure);
      // The map may already be gone when the page unmounts.
      try {
        for (const layer of latest.current.layers) if (map.getLayer(layer.id)) map.removeLayer(layer.id);
        if (map.getSource(id)) map.removeSource(id);
      } catch {
        // Removed together with the map.
      }
    };
  }, [map, id]);

  useEffect(() => {
    (map.getSource(id) as GeoJSONSource | undefined)?.setData(data as SourceData);
  }, [map, id, data]);

  return null;
}

/**
 * React content pinned to a coordinate. With `onDragEnd` the pin can be
 * picked up and dropped somewhere else.
 */
export function Pin({
  position,
  anchor = "bottom",
  zIndex = 1,
  onDragEnd,
  children,
}: {
  position: LatLng;
  anchor?: "bottom" | "center";
  zIndex?: number;
  onDragEnd?(position: LatLng): void;
  children: ReactNode;
}) {
  const map = useMap();
  const [element] = useState(() => document.createElement("div"));
  const marker = useRef<Marker | null>(null);
  const dropped = useRef(onDragEnd);
  dropped.current = onDragEnd;
  const draggable = Boolean(onDragEnd);

  useEffect(() => {
    const instance = new Marker({ element, anchor, draggable });
    instance.on("dragend", () => {
      const at = instance.getLngLat();
      dropped.current?.({ lat: at.lat, lng: at.lng });
    });
    marker.current = instance;
    return () => {
      instance.remove();
      marker.current = null;
    };
  }, [element, anchor, draggable]);

  useEffect(() => {
    marker.current?.setLngLat([position.lng, position.lat]).addTo(map);
  }, [map, position.lat, position.lng]);

  useEffect(() => {
    element.style.zIndex = String(zIndex);
  }, [element, zIndex]);

  return createPortal(children, element);
}

/** Calls `onClick` with the coordinate of a tap on the map itself (not on a pin). */
export function MapClick({ onClick }: { onClick(position: LatLng): void }) {
  const map = useMap();
  const handler = useRef(onClick);
  handler.current = onClick;
  useEffect(() => {
    const listener = (event: MapMouseEvent) => {
      // A tap on a pin (or the end of dragging one) is not a tap on the map.
      const target = event.originalEvent.target;
      if (target instanceof Element && target.closest(".maplibregl-marker")) return;
      handler.current({ lat: event.lngLat.lat, lng: event.lngLat.lng });
    };
    map.on("click", listener);
    return () => {
      map.off("click", listener);
    };
  }, [map]);
  return null;
}

/** How long camera moves take: follows the ドパガキ度, instant with motion off. */
export function cameraMs(ms: number): number {
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return 0;
  return ms * dopamineFactor();
}

/** The smallest box around some points, as the map wants it; `null` without points. */
export function boundsOf(points: LatLng[]): LngLatBoundsLike | null {
  if (points.length === 0) return null;
  let [south, west, north, east] = [90, 180, -90, -180];
  for (const point of points) {
    south = Math.min(south, point.lat);
    north = Math.max(north, point.lat);
    west = Math.min(west, point.lng);
    east = Math.max(east, point.lng);
  }
  return [
    [west, south],
    [east, north],
  ];
}

/**
 * Moves the camera so that all `points` are in view whenever `fitKey`
 * changes (not on every render: the person may have moved the map since).
 */
export function FitTo({ points, fitKey, padding = 56, maxZoom = 16 }: { points: LatLng[]; fitKey: string; padding?: number; maxZoom?: number }) {
  const map = useMap();
  const latest = useRef(points);
  latest.current = points;
  useEffect(() => {
    const bounds = boundsOf(latest.current);
    if (bounds) map.fitBounds(bounds, { padding, maxZoom, duration: cameraMs(700) });
  }, [map, fitKey, padding, maxZoom]);
  return null;
}

/** A circle of `radiusMeters` around a point as a polygon ring (the map has no metric circles). */
export function circlePolygon(center: LatLng, radiusMeters: number, steps = 64): GeoFeature<{ type: "Polygon"; coordinates: GeoRing[] }> {
  const latRadius = radiusMeters / 111_320;
  const lngRadius = radiusMeters / (111_320 * Math.max(0.01, Math.cos((center.lat * Math.PI) / 180)));
  const ring: [number, number][] = [];
  for (let step = 0; step <= steps; step++) {
    const angle = (step / steps) * Math.PI * 2;
    ring.push([center.lng + Math.cos(angle) * lngRadius, center.lat + Math.sin(angle) * latRadius]);
  }
  return { type: "Feature", properties: {}, geometry: { type: "Polygon", coordinates: [ring] } };
}

/** The zoom at which a circle of `radiusMeters` comfortably fills a small map. */
export function zoomForRadius(radiusMeters: number): number {
  return Math.max(11, Math.min(17, 16.5 - Math.log2(Math.max(25, radiusMeters) / 100)));
}
