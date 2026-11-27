import type { common, geo } from "@tokuly/api";
import { visitEmoji, visitName, visitTimes } from "@tokuly/core/history";
import { router } from "expo-router";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Linking, Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { sticker, useTheme } from "#/lib/theme";
import { NS, useAccountTimeZone } from "./data";

/**
 * The credit the gazetteer's licence asks for, wherever area names from the
 * server are on screen. Not translated: it is a credit line.
 */
export function GazetteerCredit() {
  return (
    <Text
      accessibilityRole="link"
      onPress={() => void Linking.openURL("https://www.geonames.org/")}
      className="text-center text-[11px] text-ink-faint"
    >
      Place names © GeoNames, CC BY 4.0
    </Text>
  );
}

/** A small fact of a day as a sticker pill: "👟 3.2 km". */
export function Fact({ emoji, text, tint }: { emoji: string; text: string; tint?: string }) {
  const { colors } = useTheme();
  return (
    <View
      className={`flex-row items-center gap-1.5 rounded-full px-3 py-1.5 ${tint ?? "bg-surface"}`}
      style={sticker(colors, "sm")}
    >
      <Text>{emoji}</Text>
      <Text className={`text-sm font-black ${tint ? "text-[#17122b]" : "text-ink"}`}>{text}</Text>
    </View>
  );
}

/** Faces of friends in a row, each leading to their profile. */
export function Faces({ users, size = "sm" }: { users: common.UserSummary[]; size?: "xs" | "sm" }) {
  return (
    <View className="flex-row flex-wrap gap-1">
      {users.map((user) => (
        <Pressable
          key={user.id}
          accessibilityRole="button"
          accessibilityLabel={user.displayName}
          onPress={() => router.push(`/u/${user.handle}`)}
        >
          <Avatar user={user} size={size} />
        </Pressable>
      ))}
    </View>
  );
}

/**
 * A visit wherever one is listed: emoji (or `leading`), name, when on the
 * account's clock, where, with whom, the check-in message. A day of another
 * date says which (`dateLabel`).
 */
export function VisitCard({
  visit,
  leading,
  dateLabel,
  footer,
}: {
  visit: geo.Visit;
  leading?: ReactNode;
  dateLabel?: string;
  footer?: ReactNode;
}) {
  const { t, i18n } = useTranslation(NS);
  const { colors } = useTheme();
  const zone = useAccountTimeZone();
  const name = visitName(visit);
  return (
    <View
      testID={`visit-${visit.id}`}
      className="flex-row items-start gap-3 rounded-card bg-surface p-4"
      style={sticker(colors)}
    >
      {leading ?? <VisitEmoji visit={visit} />}
      <View className="min-w-0 flex-1 gap-0.5">
        <View className="flex-row items-center gap-2">
          <Text numberOfLines={1} className="shrink font-black text-ink">
            {name}
          </Text>
          {visit.checkIn && (
            <Text className="overflow-hidden rounded-full bg-lime px-2 py-0.5 text-[10px] font-black text-[#17122b]">
              {t("visit.checkIn")}
            </Text>
          )}
        </View>
        <Text className="text-sm font-bold text-ink-soft">
          {dateLabel ? `${dateLabel} ` : ""}
          {visitTimes(i18n, visit, zone)}
        </Text>
        {visit.locality !== "" && visit.locality !== name && (
          <Text numberOfLines={1} className="text-xs text-ink-faint">
            {visit.locality}
          </Text>
        )}
        {visit.message !== "" && (
          <Text className="mt-1 rounded-2xl bg-surface-2 px-3 py-2 text-sm text-ink">
            {visit.message}
          </Text>
        )}
        {visit.withFriends.length > 0 && (
          <View className="mt-1 flex-row items-center gap-2">
            <Faces users={visit.withFriends.slice(0, 5)} size="xs" />
            <Text className="text-xs font-bold text-ink-soft">
              {t("visit.with", { count: visit.withFriends.length })}
            </Text>
          </View>
        )}
        {footer}
      </View>
    </View>
  );
}

/** A visit's emoji in its sticker tile, with its number in the day when it has one. */
export function VisitEmoji({ visit, order }: { visit: geo.Visit; order?: number }) {
  const { colors } = useTheme();
  return (
    <View
      className="size-11 items-center justify-center rounded-2xl bg-surface"
      style={sticker(colors, "sm")}
    >
      <Text className="text-xl">{visitEmoji(visit)}</Text>
      {order !== undefined && (
        <View className="absolute -top-2 -right-2 size-5 items-center justify-center rounded-full border-2 border-ink bg-lime">
          <Text className="text-[10px] font-black text-[#17122b]">{order}</Text>
        </View>
      )}
    </View>
  );
}

/** "11月22日(日)": a `YYYY-MM-DD` day in the person's language. */
export function dayTitle(date: string, language: string, withYear = false): string {
  const [year, month, day] = date.split("-").map(Number);
  return new Date(year ?? 1970, (month ?? 1) - 1, day ?? 1).toLocaleDateString(language, {
    ...(withYear ? { year: "numeric" } : {}),
    month: "long",
    day: "numeric",
    weekday: "short",
  });
}
