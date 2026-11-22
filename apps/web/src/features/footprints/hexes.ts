import {
  cellsToMultiPolygon,
  cellToBoundary,
  cellToParent,
  getResolution,
  isValidCell,
} from "h3-js";
import type { GeoFeature, GeoFeatures, GeoRing } from "#/features/map";

/** The whole map as one ring: what the fog covers before anything is uncovered. */
const WORLD_RING: [number, number][] = [
  [-180, -85],
  [180, -85],
  [180, 85],
  [-180, 85],
  [-180, -85],
];

type MultiPolygon = { type: "MultiPolygon"; coordinates: GeoRing[][] };

export interface FogShapes {
  /** Everything not yet visited: the world with the uncovered area cut out. */
  fog: GeoFeature<MultiPolygon>;
  /** The uncovered area itself, for its glowing edge. */
  uncovered: GeoFeature<MultiPolygon>;
}

/**
 * Turns uncovered cells into the fog to draw. Neighbouring cells are merged
 * into one outline first, so twenty thousand hexagons become a handful of
 * shapes and the map stays fast at every zoom.
 */
export function fogShapes(cells: string[]): FogShapes {
  const merged = cellsToMultiPolygon(
    cells.filter((cell) => isValidCell(cell)),
    true,
  ) as [number, number][][][];
  const fog: [number, number][][][] = [[WORLD_RING, ...merged.map((polygon) => polygon[0]!)]];
  // A hole in the uncovered area (a block walked around) is fog again.
  for (const polygon of merged) for (const hole of polygon.slice(1)) fog.push([hole]);
  return {
    fog: { type: "Feature", properties: {}, geometry: { type: "MultiPolygon", coordinates: fog } },
    uncovered: {
      type: "Feature",
      properties: {},
      geometry: { type: "MultiPolygon", coordinates: merged },
    },
  };
}

/**
 * Cells as separate hexagons (for the freshly uncovered ones, which are
 * highlighted). Fine cells are lifted to `resolution` when the map shows
 * coarser ones.
 */
export function cellPolygons(
  cells: string[],
  resolution?: number,
): GeoFeatures<{ type: "Polygon"; coordinates: GeoRing[] }> {
  const shown = new Set<string>();
  for (const cell of cells) {
    if (!isValidCell(cell)) continue;
    shown.add(
      resolution !== undefined && getResolution(cell) > resolution
        ? cellToParent(cell, resolution)
        : cell,
    );
  }
  return {
    type: "FeatureCollection",
    features: [...shown].map((cell) => ({
      type: "Feature",
      properties: {},
      geometry: {
        type: "Polygon",
        coordinates: [cellToBoundary(cell, true) as [number, number][]],
      },
    })),
  };
}

/**
 * Cells laid out as flat hexagons inside a `width` × `height` box (for the
 * recap card, as SVG or on a canvas): the shape of what was uncovered,
 * centred and scaled to fit, north up.
 */
export function cellArt(
  cells: string[],
  width: number,
  height: number,
  padding = 0,
): [number, number][][] {
  const rings = cells
    .filter((cell) => isValidCell(cell))
    .map((cell) => cellToBoundary(cell, true) as [number, number][]);
  if (rings.length === 0) return [];
  const midLat = rings.reduce((sum, ring) => sum + ring[0]![1], 0) / rings.length;
  const squeeze = Math.cos((midLat * Math.PI) / 180);
  const flat = rings.map((ring) =>
    ring.map(([lng, lat]) => [lng * squeeze, -lat] as [number, number]),
  );
  let [minX, minY, maxX, maxY] = [
    Number.POSITIVE_INFINITY,
    Number.POSITIVE_INFINITY,
    Number.NEGATIVE_INFINITY,
    Number.NEGATIVE_INFINITY,
  ];
  for (const ring of flat) {
    for (const [x, y] of ring) {
      minX = Math.min(minX, x);
      maxX = Math.max(maxX, x);
      minY = Math.min(minY, y);
      maxY = Math.max(maxY, y);
    }
  }
  const scale = Math.min(
    (width - padding * 2) / Math.max(maxX - minX, 1e-9),
    (height - padding * 2) / Math.max(maxY - minY, 1e-9),
  );
  const offsetX = (width - (maxX - minX) * scale) / 2;
  const offsetY = (height - (maxY - minY) * scale) / 2;
  return flat.map((ring) =>
    ring.map(
      ([x, y]) => [offsetX + (x - minX) * scale, offsetY + (y - minY) * scale] as [number, number],
    ),
  );
}
