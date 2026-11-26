import type { shorts } from "@tokuly/api";
import { compactCount } from "@tokuly/core/engagement";
import { type FeedContext, imageUrl, searchOfContext } from "@tokuly/core/shorts";
import { Image } from "expo-image";
import { router } from "expo-router";
import type { ReactElement } from "react";
import { useTranslation } from "react-i18next";
import { FlatList, Pressable, Text, useWindowDimensions, View } from "react-native";
import { mediaUrl } from "#/lib/config";

const COLUMNS = 3;

/** One tile: the cover, the views, and (for the creator's own list) the state. */
export function ShortTile({
  short,
  onPress,
  status,
}: {
  short: shorts.Short;
  onPress(): void;
  status?: string;
}) {
  const { t, i18n } = useTranslation("shorts");
  const { width } = useWindowDimensions();
  const size = (width - 4) / COLUMNS;
  const cover = mediaUrl(imageUrl(short.cover, size));
  return (
    <Pressable
      testID={`short-tile-${short.shortId || short.id}`}
      accessibilityRole="button"
      accessibilityLabel={t("tile.label", {
        caption: short.caption,
        views: compactCount(Number(short.viewCount), i18n.language),
      })}
      onPress={onPress}
      style={{ width: size, height: size * (16 / 9), padding: 1 }}
    >
      <View className="flex-1 overflow-hidden bg-surface-3">
        {cover ? <Image source={cover} style={{ flex: 1 }} contentFit="cover" /> : null}
        <Text className="absolute bottom-1 left-1.5 text-xs font-black text-white">
          ▶ {compactCount(Number(short.viewCount), i18n.language)}
        </Text>
        {status ? (
          <Text className="absolute top-1 left-1 rounded-full bg-black/60 px-2 py-0.5 text-[10px] font-black text-white">
            {status}
          </Text>
        ) : null}
      </View>
    </Pressable>
  );
}

/** Opens a short in the swipe feed of the list it was tapped in. */
export function watch(context: FeedContext, short: shorts.Short): void {
  router.push({
    pathname: "/shorts/watch",
    params: { ...searchOfContext(context), start: short.shortId || short.id },
  });
}

/** A grid of shorts with a header above it, paged as it scrolls. */
export function ShortGrid({
  shorts: items,
  header,
  empty,
  onEnd,
  onPress,
  statusOf,
}: {
  shorts: shorts.Short[];
  header?: ReactElement;
  empty?: ReactElement;
  onEnd?: () => void;
  onPress(short: shorts.Short): void;
  statusOf?: (short: shorts.Short) => string | undefined;
}) {
  return (
    <FlatList
      data={items}
      numColumns={COLUMNS}
      keyExtractor={(short) => short.id}
      ListHeaderComponent={header}
      ListEmptyComponent={empty}
      onEndReached={onEnd}
      renderItem={({ item }) => (
        <ShortTile short={item} onPress={() => onPress(item)} status={statusOf?.(item)} />
      )}
    />
  );
}
