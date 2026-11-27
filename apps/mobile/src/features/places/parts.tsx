import { type geo, toMillis } from "@tokuly/api";
import { compactAge } from "@tokuly/core/format";
import { labelMeta, placeEmoji } from "@tokuly/core/places";
import { router } from "expo-router";
import { ChevronRight } from "lucide-react-native";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { sticker, useTheme } from "#/lib/theme";
import { NS } from "./data";

/** The emoji of a place in its coloured sticker bubble. */
export function PlaceBubble({
  place,
  size = "md",
}: {
  place: Pick<geo.Place, "emoji" | "label">;
  size?: "md" | "lg";
}) {
  const { colors } = useTheme();
  return (
    <View
      className={`items-center justify-center ${labelMeta(place.label).tint} ${size === "lg" ? "size-16 rounded-3xl" : "size-12 rounded-2xl"}`}
      style={sticker(colors, "sm")}
    >
      <Text className={size === "lg" ? "text-3xl" : "text-2xl"}>{placeEmoji(place)}</Text>
    </View>
  );
}

/** One place in a list, leading to its page. */
export function PlaceRow({
  place,
  subtitle,
  trailing,
}: {
  place: geo.Place;
  subtitle?: string;
  trailing?: ReactNode;
}) {
  const { t, i18n } = useTranslation(NS);
  const { colors } = useTheme();
  const lastVisit = toMillis(place.myLastVisitAt);
  const facts = [
    place.myVisitCount > 0 ? t("row.visits", { count: place.myVisitCount }) : "",
    lastVisit > 0 ? t("row.lastVisit", { age: compactAge(i18n, lastVisit) }) : "",
  ].filter(Boolean);
  return (
    <Pressable
      testID={`place-${place.id}`}
      accessibilityRole="button"
      onPress={() => router.push(`/places/${place.id}`)}
      className="flex-row items-center gap-3 rounded-card bg-surface p-3"
      style={sticker(colors)}
    >
      <PlaceBubble place={place} />
      <View className="min-w-0 flex-1">
        <Text numberOfLines={1} className="font-black text-ink">
          {place.name}
        </Text>
        <Text numberOfLines={1} className="text-sm text-ink-soft">
          {subtitle || facts.join(" · ") || place.address || t("row.noVisits")}
        </Text>
      </View>
      {trailing ?? <ChevronRight size={20} color={colors["ink-faint"]} />}
    </Pressable>
  );
}
