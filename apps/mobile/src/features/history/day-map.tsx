import {
  Camera,
  GeoJSONSource,
  Layer,
  Map as MapLibre,
  Marker,
} from "@maplibre/maplibre-react-native";
import { buildMapStyle } from "@tokuly/core/map";
import type { LatLng } from "@tokuly/core/places";
import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";
import { TILE_CONFIG } from "#/lib/config";
import { sticker, useTheme } from "#/lib/theme";

/** A stop of the day to mark on the map: its number in the day and its emoji. */
export interface DayStop {
  id: string;
  position: LatLng;
  order: number;
  emoji: string;
}

/**
 * The day on the app's own map, as a still picture: the path travelled and
 * the numbered stops, framed on them. Only the account's own points are on
 * it — never anyone else's position. The map cannot be touched, so the
 * data's credit is written on it.
 */
export function DayMap({ path, stops }: { path: LatLng[]; stops: DayStop[] }) {
  const theme = useTheme();
  const { i18n } = useTranslation();
  const {
    sky: _sky,
    projection: _projection,
    ...style
  } = buildMapStyle("tokuly", theme.dark, i18n.language, TILE_CONFIG);
  const points = [...path, ...stops.map((stop) => stop.position)];
  if (points.length === 0) return null;
  const lats = points.map((point) => point.lat);
  const lngs = points.map((point) => point.lng);
  // A single spot gets a little room around it instead of the closest zoom.
  const pad = points.length === 1 ? 0.004 : 0;
  const bounds: [number, number, number, number] = [
    Math.min(...lngs) - pad,
    Math.min(...lats) - pad,
    Math.max(...lngs) + pad,
    Math.max(...lats) + pad,
  ];
  const line = theme.dark ? "#9a82ff" : "#7c5cff";
  return (
    <View
      testID="history-day-map"
      pointerEvents="none"
      className="h-64 overflow-hidden rounded-card"
      style={sticker(theme.colors)}
    >
      <MapLibre
        style={{ flex: 1 }}
        mapStyle={style}
        logo={false}
        attribution={false}
        compass={false}
        dragPan={false}
        touchZoom={false}
        touchRotate={false}
        touchPitch={false}
      >
        <Camera
          initialViewState={{ bounds, padding: { top: 40, right: 40, bottom: 40, left: 40 } }}
        />
        {path.length > 1 && (
          <GeoJSONSource
            id="history-day-path"
            data={{
              type: "Feature",
              properties: {},
              geometry: { type: "LineString", coordinates: path.map((p) => [p.lng, p.lat]) },
            }}
          >
            <Layer
              id="history-day-path-line"
              type="line"
              layout={{ "line-cap": "round", "line-join": "round" }}
              paint={{ "line-color": line, "line-width": 4 }}
            />
          </GeoJSONSource>
        )}
        {stops.map((stop) => (
          <Marker
            key={stop.id}
            id={`stop-${stop.id}`}
            lngLat={[stop.position.lng, stop.position.lat]}
          >
            <View
              className="size-9 items-center justify-center rounded-full bg-surface"
              style={sticker(theme.colors, "sm")}
            >
              <Text>{stop.emoji}</Text>
              <View className="absolute -top-1.5 -right-1.5 size-4 items-center justify-center rounded-full border-2 border-ink bg-lime">
                <Text className="text-[8px] font-black text-[#17122b]">{stop.order}</Text>
              </View>
            </View>
          </Marker>
        ))}
      </MapLibre>
      <Text className="absolute right-2 bottom-1 text-[10px] text-ink-soft">© OpenStreetMap</Text>
    </View>
  );
}
