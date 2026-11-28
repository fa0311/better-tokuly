import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { type ReactNode, useEffect, useState } from "react";
import { GeoLayer, MapFrame, useIsDark, useMap } from "#/features/map";
import type { LatLng } from "#/features/places";
import { useTranslation } from "#/lib/use-translation";
import { footprintClient, footprintKeys } from "./api";
import { cellPolygons, fogShapes } from "./hexes";

/**
 * Colours of the fog on the map canvas. The map draws with its own style
 * language, which cannot read the app's CSS tokens, so they are repeated
 * here: ink for the fog, lime for the edge of what has been uncovered.
 */
const FOG = {
  light: { fog: "#3a2f6b", fogOpacity: 0.5, edge: "#7c5cff", fresh: "#c6f24e" },
  dark: { fog: "#05040a", fogOpacity: 0.62, edge: "#c6f24e", fresh: "#c6f24e" },
} as const;

interface View {
  /** south, west, north, east of the area to ask for (wider than the screen). */
  box: [number, number, number, number];
  zoom: number;
}

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

/**
 * The area to load for what the map shows: the screen plus a margin, snapped
 * to a grid the size of the screen, so that panning a little asks for
 * nothing new.
 */
function viewOf(map: ReturnType<typeof useMap>): View {
  const bounds = map.getBounds();
  const zoom = Math.floor(map.getZoom());
  const step = 360 / 2 ** Math.max(0, zoom);
  const snapDown = (value: number) => Math.floor(value / step - 0.5) * step;
  const snapUp = (value: number) => Math.ceil(value / step + 0.5) * step;
  const west = bounds.getWest();
  const east = bounds.getEast();
  const wide = east - west >= 360;
  return {
    box: [
      clamp(snapDown(bounds.getSouth()), -85, 85),
      wide ? -180 : clamp(snapDown(west), -180, 180),
      clamp(snapUp(bounds.getNorth()), -85, 85),
      wide ? 180 : clamp(snapUp(east), -180, 180),
    ],
    zoom,
  };
}

/** The fog over everything not yet visited, lifted where `userId` (empty = me) has been. */
function Fog({ userId, fresh }: { userId: string; fresh: string[] }) {
  const { t } = useTranslation("footprints");
  const map = useMap();
  const dark = useIsDark();
  const [view, setView] = useState(() => viewOf(map));
  useEffect(() => {
    const update = () => setView(viewOf(map));
    map.on("moveend", update);
    return () => {
      map.off("moveend", update);
    };
  }, [map]);

  const { data } = useQuery({
    queryKey: footprintKeys.cells(userId, view.box, view.zoom),
    queryFn: () =>
      footprintClient().getCells({
        userId,
        bounds: {
          southWest: { lat: view.box[0], lng: view.box[1] },
          northEast: { lat: view.box[2], lng: view.box[3] },
        },
        zoom: view.zoom,
      }),
    placeholderData: keepPreviousData,
    staleTime: 60_000,
    // Someone else's map may simply not be shown to us: no retries, no error.
    retry: false,
  });

  const palette = dark ? FOG.dark : FOG.light;
  const shapes = fogShapes(data?.cells ?? []);
  const freshShapes = cellPolygons(fresh, data?.resolution);

  return (
    <>
      <GeoLayer
        id="footprint-fog"
        data={shapes.fog}
        layers={[
          {
            id: "footprint-fog-fill",
            type: "fill",
            paint: {
              "fill-color": palette.fog,
              "fill-opacity": palette.fogOpacity,
              "fill-antialias": true,
            },
          },
        ]}
      />
      <GeoLayer
        id="footprint-uncovered"
        data={shapes.uncovered}
        layers={[
          {
            id: "footprint-edge-glow",
            type: "line",
            paint: {
              "line-color": palette.edge,
              "line-width": 7,
              "line-blur": 6,
              "line-opacity": 0.55,
            },
          },
          {
            id: "footprint-edge",
            type: "line",
            paint: { "line-color": palette.edge, "line-width": 1.5, "line-opacity": 0.9 },
          },
        ]}
      />
      <GeoLayer
        id="footprint-fresh"
        data={freshShapes}
        layers={[
          {
            id: "footprint-fresh-fill",
            type: "fill",
            paint: { "fill-color": palette.fresh, "fill-opacity": 0.45 },
          },
          {
            id: "footprint-fresh-edge",
            type: "line",
            paint: { "line-color": palette.fresh, "line-width": 2 },
          },
        ]}
      />
      {data && (
        <div className="pointer-events-none absolute bottom-3 left-3 z-10 flex max-w-[calc(100%-1.5rem)] flex-col items-start gap-1.5">
          {data.truncated && (
            <p className="glass rounded-full px-3 py-1 text-xs font-bold text-ink-soft">
              {t("map.truncated")}
            </p>
          )}
          <p
            className="glass sticker-sm rounded-full px-3 py-1 text-xs font-black"
            data-testid="footprint-cells"
            data-cells={data.cells.length}
          >
            {data.cells.length > 0
              ? t("map.cellsHere", { count: data.cells.length })
              : t("map.noneHere")}
          </p>
        </div>
      )}
    </>
  );
}

/** The scratch map: fog everywhere except where the person has been. `fresh` cells are highlighted. */
export function FootprintMap({
  userId = "",
  center,
  zoom,
  fresh = [],
  className,
  children,
}: {
  userId?: string;
  center: LatLng;
  zoom: number;
  fresh?: string[];
  className?: string;
  children?: ReactNode;
}) {
  return (
    <MapFrame center={center} zoom={zoom} className={className}>
      <Fog userId={userId} fresh={fresh} />
      {children}
    </MapFrame>
  );
}
