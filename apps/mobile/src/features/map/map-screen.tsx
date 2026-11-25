import { Camera, type CameraRef, Map as MapLibre, Marker } from "@maplibre/maplibre-react-native";
import { buildMapStyle } from "@tokuly/core/map";
import { Ghost, LocateFixed, X } from "lucide-react-native";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, ScrollView, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { TILE_CONFIG } from "#/lib/config";
import { sticker, useTheme } from "#/lib/theme";
import { useNow } from "./clock";
import { useFriendsById, useGhostSettings } from "./data";
import { FriendCard } from "./friend-card";
import { FriendPin, OwnDot } from "./friend-pin";
import { GhostPanel, ghostChipText } from "./ghost";
import { useLocations } from "./locations-store";
import { SharingCard } from "./sharing-card";

/** Tokyo, until this device or a friend says where to look. */
const DEFAULT_CENTER: [number, number] = [139.7006, 35.6595];

type Panel = { kind: "friend"; userId: string } | { kind: "ghost" } | { kind: "sharing" } | null;

/**
 * Pins are `Marker`s, real native views: Android draws a `ViewAnnotation`'s
 * children into a bitmap, which can neither be tapped nor read by a screen
 * reader (or a test).
 *
 * The live friend map: friends' faces where they are, a card for the one
 * tapped, this device's own dot, ghost mode and sharing one tap away. Pins
 * move as the realtime connection reports them (`useMapSync`).
 */
export function MapScreen() {
  const { t, i18n } = useTranslation("map");
  const { t: tm } = useTranslation("mobile");
  const theme = useTheme();
  const insets = useSafeAreaInsets();
  const camera = useRef<CameraRef>(null);
  const now = useNow();
  const friends = useFriendsById();
  const locations = useLocations((state) => state.friends);
  const online = useLocations((state) => state.online);
  const own = useLocations((state) => state.own);
  const { data: ghost } = useGhostSettings();
  const [panel, setPanel] = useState<Panel>(null);

  // MapLibre Native draws the same basemap as the web; the globe and sky are GL JS only.
  const {
    sky: _sky,
    projection: _projection,
    ...style
  } = buildMapStyle("tokuly", theme.dark, i18n.language, TILE_CONFIG);
  const pins = Object.values(locations).filter((location) => location.position);
  const ghostChip = ghostChipText(ghost, now, t, i18n.language);
  const selected = panel?.kind === "friend" ? panel.userId : null;

  const focus = (lng: number, lat: number) =>
    camera.current?.easeTo({ center: [lng, lat], zoom: 15, duration: 600 });

  return (
    <View className="flex-1 bg-bg" testID="map-screen">
      <MapLibre
        style={{ flex: 1 }}
        mapStyle={style}
        attribution
        attributionPosition={{ bottom: 8, left: 8 }}
        logo={false}
        compass={false}
        onPress={() => setPanel(null)}
      >
        <Camera ref={camera} initialViewState={{ center: DEFAULT_CENTER, zoom: 11 }} />
        {own && (
          <Marker id="own" lngLat={[own.lng, own.lat]}>
            <OwnDot />
          </Marker>
        )}
        {pins.map((location) => (
          <Marker
            key={location.userId}
            id={`friend-${location.userId}`}
            lngLat={[location.position?.lng ?? 0, location.position?.lat ?? 0]}
            anchor="bottom"
          >
            <FriendPin
              friend={friends[location.userId]}
              location={location}
              online={Boolean(online[location.userId])}
              now={now}
              onPress={() => {
                setPanel({ kind: "friend", userId: location.userId });
                if (location.position) focus(location.position.lng, location.position.lat);
              }}
            />
          </Marker>
        ))}
      </MapLibre>

      <View
        pointerEvents="box-none"
        className="absolute inset-x-0 flex-row flex-wrap items-center gap-2 px-4"
        style={{ top: insets.top + 8 }}
      >
        <Chip
          testID="ghost-chip"
          label={ghostChip ?? tm("map.ghost")}
          active={Boolean(ghostChip)}
          icon={<Ghost size={16} color={ghostChip ? theme.colors.surface : theme.colors.ink} />}
          onPress={() => setPanel({ kind: "ghost" })}
        />
        <Chip
          testID="sharing-chip"
          label={own ? tm("sharing.sharing.title") : tm("map.sharingOff")}
          active={false}
          icon={<Text>{own ? "🟢" : "⚪️"}</Text>}
          onPress={() => setPanel({ kind: "sharing" })}
        />
        <Text className="ml-auto rounded-full bg-surface/90 px-3 py-1.5 text-xs font-black text-ink">
          {tm("map.friendsOnMap", { count: pins.length })}
        </Text>
      </View>

      <View pointerEvents="box-none" className="absolute inset-x-0 bottom-0 gap-3 px-4 pb-4">
        {own && (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={t("recenter")}
            onPress={() => focus(own.lng, own.lat)}
            className="size-12 items-center justify-center self-end rounded-full bg-surface"
            style={sticker(theme.colors)}
          >
            <LocateFixed size={22} color={theme.colors.ink} />
          </Pressable>
        )}
        <Sheet panel={panel} onClose={() => setPanel(null)}>
          {panel?.kind === "friend" && selected ? (
            <FriendCard
              friend={friends[selected]}
              location={locations[selected]}
              online={Boolean(online[selected])}
              now={now}
            />
          ) : panel?.kind === "ghost" ? (
            <GhostPanel now={now} />
          ) : (
            <SharingCard />
          )}
        </Sheet>
      </View>
    </View>
  );
}

/** The card at the bottom: the sharing state by default, or what was tapped. */
function Sheet({
  panel,
  onClose,
  children,
}: {
  panel: Panel;
  onClose(): void;
  children: React.ReactNode;
}) {
  const { colors } = useTheme();
  const { t } = useTranslation("common");
  return (
    <View className="max-h-[420px] rounded-card bg-surface" style={sticker(colors)}>
      {panel && (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={t("actions.close")}
          onPress={onClose}
          hitSlop={10}
          className="absolute top-3 right-3 z-10 size-8 items-center justify-center rounded-full bg-surface-2"
        >
          <X size={16} color={colors.ink} />
        </Pressable>
      )}
      <ScrollView contentContainerClassName="p-4">{children}</ScrollView>
    </View>
  );
}

function Chip({
  label,
  icon,
  active,
  onPress,
  testID,
}: {
  label: string;
  icon: React.ReactNode;
  active: boolean;
  onPress(): void;
  testID?: string;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      onPress={onPress}
      className={`h-9 flex-row items-center gap-1.5 rounded-full px-3 ${active ? "bg-ink" : "bg-surface"}`}
      style={sticker(colors, "sm")}
    >
      {icon}
      <Text
        numberOfLines={1}
        className={`text-xs font-black ${active ? "text-surface" : "text-ink"}`}
      >
        {label}
      </Text>
    </Pressable>
  );
}
