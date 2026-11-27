import type { stamp } from "@tokuly/api";
import { Image } from "expo-image";
import { router } from "expo-router";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { mediaUrl } from "#/lib/config";
import { NS } from "./data";

/** Who made a pack: Tokuly, a creator, or someone who left. */
export function creatorName(t: (key: string) => string, pack: stamp.PackSummary): string {
  if (pack.official) return t("official");
  return pack.creator?.displayName || pack.creator?.handle || t("deletedCreator");
}

/** The pack's price, or what the viewer already has of it. */
export function PriceTag({ pack }: { pack: stamp.PackSummary }) {
  const { t } = useTranslation(NS);
  const { t: tm } = useTranslation("mobile");
  const label = pack.viewer?.installed
    ? t("state.installed")
    : pack.viewer?.owned
      ? t("state.owned")
      : pack.price === 0
        ? t("free")
        : tm("stamps.coins", { count: pack.price });
  const held = pack.viewer?.installed || pack.viewer?.owned;
  return (
    <Text
      className={`self-start rounded-full px-2 py-0.5 text-[11px] font-black ${
        held
          ? "bg-surface-3 text-ink-soft"
          : pack.price === 0
            ? "bg-mint/20 text-ink"
            : "bg-sun/30 text-ink"
      }`}
    >
      {label}
    </Text>
  );
}

export function PackCover({ pack, size }: { pack: stamp.PackSummary; size: number }) {
  return (
    <View
      style={{ width: size, height: size }}
      className="items-center justify-center overflow-hidden rounded-2xl bg-surface-2"
    >
      {pack.cover?.url ? (
        <Image
          source={mediaUrl(pack.cover.url)}
          placeholder={pack.cover.thumbhash ? { thumbhash: pack.cover.thumbhash } : undefined}
          contentFit="contain"
          style={{ width: size - 8, height: size - 8 }}
        />
      ) : (
        <Text className="text-2xl">🧸</Text>
      )}
    </View>
  );
}

const openPack = (pack: stamp.PackSummary) =>
  router.push({ pathname: "/stamps/[packId]", params: { packId: pack.shortId || pack.id } });

/** A pack on a store shelf: its cover over its name, who made it and the price. */
export function PackCard({ pack }: { pack: stamp.PackSummary }) {
  const { t } = useTranslation(NS);
  return (
    <Pressable
      testID={`pack-${pack.id}`}
      accessibilityRole="button"
      accessibilityLabel={pack.name}
      onPress={() => openPack(pack)}
      className="w-32 gap-1"
    >
      <PackCover pack={pack} size={128} />
      <Text numberOfLines={1} className="font-black text-ink">
        {pack.name}
      </Text>
      <Text numberOfLines={1} className="text-xs text-ink-soft">
        {creatorName(t, pack)}
      </Text>
      <PriceTag pack={pack} />
    </Pressable>
  );
}

/** A pack as a row of a list (search results, the library). `trailing` goes at its end. */
export function PackRow({
  pack,
  trailing,
}: {
  pack: stamp.PackSummary;
  trailing?: React.ReactNode;
}) {
  const { t } = useTranslation(NS);
  return (
    <Pressable
      testID={`pack-${pack.id}`}
      accessibilityRole="button"
      onPress={() => openPack(pack)}
      className="flex-row items-center gap-3 border-b border-line px-4 py-3 active:bg-surface-2"
    >
      <PackCover pack={pack} size={56} />
      <View className="min-w-0 flex-1 gap-0.5">
        <Text numberOfLines={1} className="font-black text-ink">
          {pack.name}
        </Text>
        <Text numberOfLines={1} className="text-xs text-ink-soft">
          {creatorName(t, pack)} · {t("stampCount", { count: pack.stampCount })}
        </Text>
        <PriceTag pack={pack} />
      </View>
      {trailing}
    </Pressable>
  );
}
