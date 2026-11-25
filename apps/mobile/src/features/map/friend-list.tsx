import type { geo, social } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { friendName } from "./data";
import { pinLabel } from "./pin-label";

/**
 * The friends on the map as a list, freshest first: the way in to each pin
 * for a screen reader (MapLibre's map view does not expose its markers),
 * and a quicker one than panning for everyone.
 */
export function FriendList({
  friends,
  locations,
  online,
  now,
  onPick,
}: {
  friends: Record<string, social.Friend>;
  locations: geo.FriendLocation[];
  online: Record<string, boolean>;
  now: number;
  onPick(location: geo.FriendLocation): void;
}) {
  const { t } = useTranslation("map");
  const rows = [...locations].sort(
    (a, b) => Number(b.recordedAt?.seconds ?? 0) - Number(a.recordedAt?.seconds ?? 0),
  );
  return (
    <View className="gap-1" testID="map-friend-list">
      <Text className="pb-1 text-lg font-black text-ink">{t("list.title")}</Text>
      {rows.map((location) => {
        const friend = friends[location.userId];
        return (
          <Pressable
            key={location.userId}
            testID={`map-friend-${friend?.user?.handle ?? location.userId}`}
            accessibilityRole="button"
            onPress={() => onPick(location)}
            className="flex-row items-center gap-3 rounded-control px-1 py-2"
          >
            <Avatar user={friend?.user} size="sm" online={Boolean(online[location.userId])} />
            <Text numberOfLines={1} className="flex-1 font-bold text-ink">
              {friendName(friend)}
            </Text>
            <Text className="text-xs font-bold text-ink-soft">{pinLabel(location, now)}</Text>
          </Pressable>
        );
      })}
    </View>
  );
}
