import type { moments } from "@tokuly/api";
import { Image } from "expo-image";
import { router } from "expo-router";
import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { EmptyState, ErrorNote, Spinner } from "#/components/page";
import { mediaUrl } from "#/lib/config";
import { useTheme } from "#/lib/theme";
import type { useMapMoments } from "./moments-data";

export function MomentMapPanel({
  query,
  selected,
  onRetry,
}: {
  query: ReturnType<typeof useMapMoments>;
  selected?: moments.Moment;
  onRetry(): void;
}) {
  const { t } = useTranslation("map");
  const { colors } = useTheme();
  const momentsOnMap = query.data?.moments.filter((moment) => moment.location) ?? [];

  if (query.isPending || query.isFetching) {
    return (
      <View className="items-center gap-2 py-5" testID="moment-map-loading">
        <Spinner />
        <Text className="text-sm text-ink-soft">{t("moments.loading")}</Text>
      </View>
    );
  }

  if (query.isError) {
    return (
      <View className="gap-3" testID="moment-map-error">
        <ErrorNote>{t("moments.loadError")}</ErrorNote>
        <Button variant="secondary" onPress={onRetry}>
          {t("moments.retry")}
        </Button>
      </View>
    );
  }

  if (!query.data?.unlocked) {
    return (
      <View className="gap-3" testID="moment-map-locked">
        <EmptyState emoji="🔒" title={t("moments.lockedTitle")} body={t("moments.lockedBody")} />
        <Button variant="secondary" onPress={() => router.push("/moments")}>
          {t("moments.share")}
        </Button>
      </View>
    );
  }

  if (momentsOnMap.length === 0) {
    return (
      <View className="gap-3" testID="moment-map-empty">
        <EmptyState emoji="🌤️" title={t("moments.emptyTitle")} body={t("moments.emptyBody")} />
        <Button variant="secondary" onPress={() => router.push("/moments/friends")}>
          {t("moments.feed")}
        </Button>
      </View>
    );
  }

  if (selected) {
    const image = selected.back;
    const imageUrl = image?.url ? mediaUrl(image.url) : undefined;
    const name = selected.author?.displayName || selected.author?.handle || t("moments.someone");

    return (
      <View className="gap-3" testID={`moment-map-preview-${selected.id}`}>
        <View className="flex-row items-center gap-3 pr-8">
          <Avatar user={selected.author} />
          <Text numberOfLines={1} className="flex-1 font-black text-ink">
            {name}
          </Text>
        </View>
        {imageUrl ? (
          <Image
            source={{ uri: imageUrl }}
            placeholder={image?.thumbhash ? { thumbhash: image.thumbhash } : undefined}
            contentFit="cover"
            style={{
              width: "100%",
              height: 144,
              borderRadius: 12,
              backgroundColor: image?.dominantColor || colors["surface-2"],
            }}
          />
        ) : null}
        <Text numberOfLines={2} className="text-sm text-ink-soft">
          {selected.caption || t("moments.noCaption")}
        </Text>
        <Button
          variant="secondary"
          onPress={() =>
            router.push({
              pathname: "/moments/friends/[momentId]",
              params: { momentId: selected.id },
            })
          }
        >
          {t("moments.open")}
        </Button>
      </View>
    );
  }

  return (
    <View className="gap-3" testID="moment-map-count">
      <View className="gap-1 pr-8">
        <Text className="font-black text-ink">{t("moments.title")}</Text>
        <Text className="text-sm text-ink-soft">
          {t("moments.count", { count: momentsOnMap.length })}
        </Text>
      </View>
      <Button variant="secondary" onPress={() => router.push("/moments/friends")}>
        {t("moments.feed")}
      </Button>
    </View>
  );
}
