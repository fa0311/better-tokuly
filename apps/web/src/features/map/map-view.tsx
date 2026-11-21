import { Map as MapLibreMap, setWorkerUrl } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
// The map parses tiles in a module worker. Let the bundler emit that worker
// (with its imports) and tell the library where it ended up.
import mapWorkerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url";
import { createContext, type ReactNode, useContext, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { buildMapStyle, type MapStyleId } from "./tiles";

setWorkerUrl(mapWorkerUrl);

const MapContext = createContext<MapLibreMap | null>(null);

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

export interface MapViewProps {
  styleId: MapStyleId;
  /** Where the map opens: `[lng, lat]`. */
  initialCenter: [number, number];
  initialZoom: number;
  children?: ReactNode;
}

/**
 * The map canvas. Children render once the style has loaded and can reach
 * the map with `useMap()` — markers, overlays and controls are all children.
 */
export function MapView({ styleId, initialCenter, initialZoom, children }: MapViewProps) {
  const container = useRef<HTMLDivElement>(null);
  const [map, setMap] = useState<MapLibreMap | null>(null);
  const { i18n } = useTranslation();
  const dark = useIsDark();
  const language = i18n.language.startsWith("ja") ? "ja" : "en";
  const appliedStyle = useRef<string>("");

  // Create the map once. Later style changes go through setStyle below.
  // biome-ignore lint/correctness/useExhaustiveDependencies: the map must not be recreated when its initial view or style inputs change.
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
      // Rotation with two fingers / right-drag stays on; the keyboard handler too.
      dragRotate: true,
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

  return (
    <div className="absolute inset-0">
      {/* Sized, not positioned: the map library sets `position: relative` on its container. */}
      <div ref={container} className="size-full" />
      {map && <MapContext.Provider value={map}>{children}</MapContext.Provider>}
    </div>
  );
}
