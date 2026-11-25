import { buildMapStyle as buildStyle, type MapStyleId, TILE_ZOOMS } from "@tokuly/core/map";
import type { StyleSpecification } from "maplibre-gl";

export type { MapStyleId } from "@tokuly/core/map";

/** Tiles and map assets are served next to the app (the edge, or the dev server's proxy). */
function tileConfig() {
  const origin = typeof window === "undefined" ? "" : window.location.origin;
  const absolute = (url: string) => (url.startsWith("http") ? url : `${origin}${url}`);
  return {
    tilesUrl: absolute(import.meta.env.VITE_TILES_URL ?? "/tiles"),
    assetsUrl: absolute(import.meta.env.VITE_MAP_ASSETS_URL ?? "/map-assets"),
    ...TILE_ZOOMS,
  };
}

/** The MapLibre style of the web app's maps (`@tokuly/core/map` builds it). */
export function buildMapStyle(
  style: MapStyleId,
  dark: boolean,
  language: string,
): StyleSpecification {
  return buildStyle(style, dark, language, tileConfig()) as StyleSpecification;
}
