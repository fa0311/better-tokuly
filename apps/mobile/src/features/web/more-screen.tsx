import { router } from "expo-router";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Page } from "#/components/page";
import { useMe } from "#/features/session/store";
import { sticker, useTheme } from "#/lib/theme";
import { featuresFor, type WebFeature } from "./features";
import { webScreenHref } from "./handoff";

/** 「もっと見る」: every part of Tokuly the app does not have yet, each opening on the web, signed in. */
export function MoreScreen() {
  const { t } = useTranslation("mobile");
  const me = useMe();
  const features = featuresFor(me?.roles ?? []);
  return (
    <Page testID="more-screen">
      <Text className="px-1 text-base font-black text-ink">{t("more.lead")}</Text>
      <View className="flex-row flex-wrap gap-3">
        {features.map((feature) => (
          <Tile key={feature.key} feature={feature} />
        ))}
      </View>
      <Text className="px-1 text-xs text-ink-soft">{t("more.note")}</Text>
    </Page>
  );
}

function Tile({ feature }: { feature: WebFeature }) {
  const { t } = useTranslation("mobile");
  const { colors } = useTheme();
  const Icon = feature.icon;
  const label = t(`more.features.${feature.key}`);
  return (
    <Pressable
      testID={`more-${feature.key}`}
      accessibilityRole="button"
      accessibilityLabel={label}
      onPress={() => router.push(webScreenHref(feature.path) as never)}
      className="w-[30%] grow items-center gap-2 rounded-card bg-surface px-2 py-4"
      style={sticker(colors)}
    >
      <Icon size={26} color={colors.brand} />
      <Text className="text-center text-sm font-bold text-ink" numberOfLines={1}>
        {label}
      </Text>
    </Pressable>
  );
}
