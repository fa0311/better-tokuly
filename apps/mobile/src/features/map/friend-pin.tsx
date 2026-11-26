import type { geo, social } from "@tokuly/api";
import { pinState } from "@tokuly/core/map";
import { Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { sticker, useTheme } from "#/lib/theme";
import { friendName } from "./data";
import { pinLabel } from "./pin-label";

/** A friend on the map: their face as a sticker, and a pill with how fresh the pin is. */
export function FriendPin({
  friend,
  location,
  online,
  now,
  onPress,
}: {
  friend: social.Friend | undefined;
  location: geo.FriendLocation;
  online: boolean;
  now: number;
  onPress(): void;
}) {
  const { colors } = useTheme();
  const faded = pinState(location, now) !== "live";
  return (
    <Pressable
      testID={`friend-pin-${friend?.user?.handle ?? location.userId}`}
      accessibilityRole="button"
      accessibilityLabel={`${friendName(friend)} · ${pinLabel(location, now)}`}
      onPress={onPress}
      className="items-center"
      style={{ opacity: faded ? 0.6 : 1 }}
    >
      <Avatar user={friend?.user} size="md" outlined online={online} />
      <View className="mt-1 rounded-full bg-surface px-2 py-0.5" style={sticker(colors, "sm")}>
        <Text numberOfLines={1} className="text-[11px] font-black text-ink">
          {pinLabel(location, now)}
        </Text>
      </View>
    </Pressable>
  );
}

/** Friends too close to tell apart at this zoom: up to three faces fanned like cards, and how many. */
export function CrowdPin({
  members,
  label,
  onPress,
}: {
  members: (social.Friend | undefined)[];
  label: string;
  onPress(): void;
}) {
  const { colors } = useTheme();
  const faces = members.slice(0, 3);
  return (
    <Pressable
      testID="crowd-pin"
      accessibilityRole="button"
      accessibilityLabel={label}
      onPress={onPress}
      className="flex-row px-2 pt-2"
    >
      {faces.map((friend, index) => (
        <View
          key={friend?.user?.id ?? index}
          style={{
            marginLeft: index > 0 ? -24 : 0,
            zIndex: faces.length - index,
            transform: [{ rotate: `${(index - 1) * 7}deg` }],
          }}
        >
          <Avatar user={friend?.user} size="md" outlined />
        </View>
      ))}
      <View
        className="absolute top-0 right-0 z-10 h-6 min-w-6 items-center justify-center rounded-full bg-brand px-1.5"
        style={sticker(colors, "sm")}
      >
        <Text className="text-xs font-black text-on-brand">{members.length}</Text>
      </View>
    </Pressable>
  );
}

/** Where this device is: a brand-coloured dot with a halo. */
export function OwnDot() {
  const { colors } = useTheme();
  return (
    <View className="size-7 items-center justify-center rounded-full bg-brand/25">
      <View className="size-4 rounded-full bg-brand" style={sticker(colors, "sm")} />
    </View>
  );
}
