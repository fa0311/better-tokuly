import { Camera, Map as MapLibre, Marker } from "@maplibre/maplibre-react-native";
import type { plans } from "@tokuly/api";
import { buildMapStyle } from "@tokuly/core/map";
import { MapPin } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Linking, Platform, View } from "react-native";
import { TILE_CONFIG } from "#/lib/config";
import { INK_ON_BRIGHT, sticker, useTheme } from "#/lib/theme";

/**
 * Where the place is in the device's own map app, for the way there: its
 * point when it has one, else what its name and address say.
 */
export function mapsUrl(place: plans.PlanPlace): string {
  const label = [place.name, place.address].filter(Boolean).join(" ");
  const query = encodeURIComponent(label);
  const at = place.location && `${place.location.lat},${place.location.lng}`;
  if (Platform.OS === "ios") return at ? `maps:?ll=${at}&q=${query}` : `maps:?q=${query}`;
  return at ? `geo:${at}?q=${at}(${query})` : `geo:0,0?q=${query}`;
}

export function openInMaps(place: plans.PlanPlace): void {
  void Linking.openURL(mapsUrl(place));
}

/** The place on the app's own map, as a still picture with its pin; a tap opens the map app. */
export function PlaceMap({ location }: { location: NonNullable<plans.PlanPlace["location"]> }) {
  const theme = useTheme();
  const { i18n } = useTranslation();
  const {
    sky: _sky,
    projection: _projection,
    ...style
  } = buildMapStyle("tokuly", theme.dark, i18n.language, TILE_CONFIG);
  const center: [number, number] = [location.lng, location.lat];
  return (
    <View
      testID="plan-place-map"
      pointerEvents="none"
      className="mt-2 h-36 overflow-hidden rounded-2xl"
      style={sticker(theme.colors, "sm")}
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
        <Camera initialViewState={{ center, zoom: 15 }} />
        <Marker id="plan-place" lngLat={center} anchor="bottom">
          <View
            className="size-9 items-center justify-center rounded-full bg-sky"
            style={sticker(theme.colors, "sm")}
          >
            <MapPin size={18} color={INK_ON_BRIGHT} />
          </View>
        </Marker>
      </MapLibre>
    </View>
  );
}
