import { type geo, type social, toMillis } from "@tokuly/api";
import { compactAge } from "@tokuly/core/format";
import { ghostRuleFor, isBlurred, pinState } from "@tokuly/core/map";
import { router } from "expo-router";
import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { friendName, useGhostSettings } from "./data";
import { GhostPick, useSetGhost } from "./ghost";
import { ACTIVITY_KEYS } from "./pin-label";

/** A friend's card: who, how fresh, what they are doing, how I show myself to them. */
export function FriendCard({
  friend,
  location,
  online,
  now,
}: {
  friend: social.Friend | undefined;
  location: geo.FriendLocation | undefined;
  online: boolean;
  now: number;
}) {
  const { t, i18n } = useTranslation("map");
  const { data: ghost } = useGhostSettings();
  const setGhost = useSetGhost();
  const user = friend?.user;
  if (!user) {
    return (
      <View className="gap-1">
        <Text className="text-lg font-black text-ink">{t("friend.gone")}</Text>
        <Text className="text-sm text-ink-soft">{t("friend.goneBody")}</Text>
      </View>
    );
  }
  const state = location ? pinState(location, now) : null;
  const facts = location
    ? [
        ACTIVITY_KEYS[location.activity] ? t(`activity.${ACTIVITY_KEYS[location.activity]}`) : null,
        location.batteryPercent !== undefined
          ? location.charging
            ? `⚡ ${t("battery", { percent: location.batteryPercent })}`
            : t("battery", { percent: location.batteryPercent })
          : null,
        t("lastSeen", { age: compactAge(i18n, toMillis(location.recordedAt), now) }),
      ].filter(Boolean)
    : [];
  const note = !location
    ? t("friend.noLocation")
    : state === "paused"
      ? t("friend.paused")
      : isBlurred(location)
        ? t("friend.blurred")
        : state === "stale"
          ? t("friend.stale", { age: compactAge(i18n, toMillis(location.recordedAt), now) })
          : null;
  return (
    <View className="gap-3" testID="friend-card">
      <View className="flex-row items-center gap-3">
        <Avatar user={user} size="lg" outlined online={online} />
        <View className="min-w-0 flex-1">
          <Text numberOfLines={1} className="text-xl font-black text-ink">
            {friendName(friend)}
          </Text>
          <Text numberOfLines={1} className="text-sm font-bold text-ink-soft">
            @{user.handle}
            {location?.locality ? ` · ${location.locality}` : ""}
          </Text>
        </View>
      </View>
      {facts.length > 0 && <Text className="text-sm font-bold text-ink">{facts.join(" · ")}</Text>}
      {note && (
        <Text className="rounded-control bg-surface-2 px-3 py-2 text-xs font-bold text-ink-soft">
          {note}
        </Text>
      )}
      <View className="gap-1.5">
        <Text className="text-xs font-black text-ink-soft">
          {t("ghost.forThisFriend", { name: friendName(friend) })}
        </Text>
        <GhostPick
          label={t("ghost.forThisFriend", { name: friendName(friend) })}
          current={ghostRuleFor(ghost, user.id, now)}
          onPick={(mode) => setGhost.mutate({ userIds: [user.id], mode, durationSeconds: 0 })}
        />
      </View>
      <Button
        testID="friend-card-profile"
        variant="secondary"
        block
        onPress={() => router.push({ pathname: "/u/[handle]", params: { handle: user.handle } })}
      >
        {t("actions.profile")}
      </Button>
    </View>
  );
}
