import { type Flavor, layers, namedFlavor } from "@protomaps/basemaps";
import type { StyleSpecification } from "maplibre-gl";

/**
 * Where map data comes from. Everything is self-hostable: vector tiles are
 * served from PMTiles archives by the `tiles` service, and glyphs/sprites are
 * static assets next to them.
 *
 * Two archives make up the basemap: `world` (whole planet, low zoom) and
 * `region` (the area the deployment cares about, street level).
 */
export interface TileConfig {
  /** Base URL of the tile service, e.g. `/tiles`. */
  tilesUrl: string;
  /** Base URL of basemap fonts and sprites. */
  assetsUrl: string;
  /** Highest zoom contained in the `world` archive. */
  worldMaxZoom: number;
  /** Highest zoom contained in the `region` archive. */
  regionMaxZoom: number;
}

export const DEFAULT_TILE_CONFIG: TileConfig = {
  tilesUrl: import.meta.env.VITE_TILES_URL ?? "/tiles",
  assetsUrl: import.meta.env.VITE_MAP_ASSETS_URL ?? "/map-assets",
  worldMaxZoom: 6,
  regionMaxZoom: 15,
};

export type MapStyleId = "tokuly" | "light" | "dark" | "mono";

/** Tokuly's own map colours: soft lavender land, mint parks, sky water. */
const TOKULY_LIGHT: Partial<Flavor> = {
  background: "#ece8fb",
  earth: "#f3f0ff",
  park_a: "#c9f0d0",
  park_b: "#b7ebc4",
  wood_a: "#cdeed0",
  wood_b: "#bfe8c6",
  scrub_a: "#d9f2d6",
  scrub_b: "#cdeccb",
  water: "#a9dcff",
  beach: "#fff0c2",
  sand: "#f7ecd0",
  buildings: "#e2dcf5",
  pedestrian: "#ebe6fa",
  school: "#ece0ff",
  hospital: "#ffe0ea",
  industrial: "#e6e3f0",
  minor_a: "#ffffff",
  minor_b: "#ffffff",
  minor_service: "#f8f6ff",
  link: "#ffffff",
  major: "#ffffff",
  highway: "#ffe9a8",
  highway_casing_late: "#f2d27a",
  railway: "#c7bfe0",
  boundaries: "#b3a7dc",
  city_label: "#4b4270",
  city_label_halo: "#f3f0ff",
  subplace_label: "#7c749a",
  subplace_label_halo: "#f3f0ff",
  state_label: "#a69fc2",
  country_label: "#8a82ac",
  ocean_label: "#5c9fd0",
  roads_label_major: "#8a84a6",
  roads_label_minor: "#a39dbd",
};

const TOKULY_DARK: Partial<Flavor> = {
  background: "#14111f",
  earth: "#1b1730",
  park_a: "#1e3a34",
  park_b: "#1b352f",
  wood_a: "#1d3832",
  wood_b: "#1a332e",
  water: "#17304d",
  buildings: "#262040",
  pedestrian: "#211c38",
  minor_a: "#2c2647",
  minor_b: "#2c2647",
  minor_service: "#262040",
  link: "#332c52",
  major: "#3a3260",
  highway: "#4a3f7a",
  railway: "#3a3360",
  boundaries: "#5d5390",
  city_label: "#d9d3f5",
  city_label_halo: "#14111f",
  subplace_label: "#a9a1cc",
  subplace_label_halo: "#14111f",
  ocean_label: "#5f8fc4",
};

function flavorFor(style: MapStyleId, dark: boolean): Flavor {
  switch (style) {
    case "light":
      return namedFlavor("light");
    case "dark":
      return namedFlavor("dark");
    case "mono":
      return namedFlavor(dark ? "black" : "grayscale");
    default:
      return dark
        ? { ...namedFlavor("dark"), ...TOKULY_DARK }
        : { ...namedFlavor("light"), ...TOKULY_LIGHT };
  }
}

/**
 * Builds the MapLibre style. The low-zoom planet draws up to the zoom where
 * the regional archive takes over; outside the region the map simply shows
 * land colour at street level.
 */
export function buildMapStyle(
  style: MapStyleId,
  dark: boolean,
  language: string,
  config = DEFAULT_TILE_CONFIG,
): StyleSpecification {
  const flavor = flavorFor(style, dark);
  const origin = typeof window === "undefined" ? "" : window.location.origin;
  const absolute = (url: string) => (url.startsWith("http") ? url : `${origin}${url}`);
  const handover = config.worldMaxZoom + 1;

  const worldLayers = layers("world", flavor, { lang: language }).map((layer) => ({
    ...layer,
    id: `world-${layer.id}`,
    // The background belongs to the planet layer set and must always draw.
    ...(layer.type === "background" ? {} : { maxzoom: handover }),
  }));
  const regionLayers = layers("region", flavor, { lang: language })
    .filter((layer) => layer.type !== "background")
    .map((layer) => ({
      ...layer,
      id: `region-${layer.id}`,
      minzoom: Math.max(handover, "minzoom" in layer ? (layer.minzoom ?? 0) : 0),
    }));

  return {
    version: 8,
    glyphs: `${absolute(config.assetsUrl)}/fonts/{fontstack}/{range}.pbf`,
    sprite: `${absolute(config.assetsUrl)}/sprites/v4/${dark ? "dark" : "light"}`,
    sources: {
      world: {
        type: "vector",
        tiles: [`${absolute(config.tilesUrl)}/world/{z}/{x}/{y}.mvt`],
        maxzoom: config.worldMaxZoom,
        attribution:
          '<a href="https://protomaps.com">Protomaps</a> © <a href="https://openstreetmap.org">OpenStreetMap</a>',
      },
      region: {
        type: "vector",
        tiles: [`${absolute(config.tilesUrl)}/region/{z}/{x}/{y}.mvt`],
        maxzoom: config.regionMaxZoom,
      },
    },
    // biome-ignore lint/suspicious/noExplicitAny: the generated layer list is a valid layer array; the spread loses the union type.
    layers: [...worldLayers, ...regionLayers] as any,
    sky: {
      "sky-color": dark ? "#0d0b14" : "#cfe9ff",
      "horizon-color": dark ? "#241d42" : "#f3f0ff",
      "fog-color": dark ? "#14111f" : "#ffffff",
      "sky-horizon-blend": 0.6,
      "horizon-fog-blend": 0.6,
      "fog-ground-blend": 0.4,
      "atmosphere-blend": ["interpolate", ["linear"], ["zoom"], 0, 1, 5, 1, 7, 0],
    },
    projection: {
      type: ["interpolate", ["linear"], ["zoom"], 4, "vertical-perspective", 6, "mercator"],
    },
  };
}
