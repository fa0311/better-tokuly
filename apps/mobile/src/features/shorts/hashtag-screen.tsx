import { compactCount } from "@tokuly/core/engagement";
import type { FeedContext } from "@tokuly/core/shorts";
import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";
import { EmptyState, Spinner } from "#/components/page";
import { useHashtag, useShortList } from "./data";
import { ShortGrid, watch } from "./grid";

/** A hashtag: how much it is used, and its shorts, the best ranked first. */
export function HashtagScreen({ tag }: { tag: string }) {
  const { t, i18n } = useTranslation("shorts");
  const context: FeedContext = { kind: "hashtag", key: tag };
  const hashtag = useHashtag(tag);
  const list = useShortList(context);
  const items = list.data?.pages.flatMap((page) => page.entries.map((entry) => entry.short)) ?? [];
  if (list.isPending) return <Spinner />;
  return (
    <View className="flex-1 bg-bg" testID={`hashtag-${tag}`}>
      <ShortGrid
        shorts={items}
        onPress={(short) => watch(context, short)}
        onEnd={() => list.hasNextPage && void list.fetchNextPage()}
        header={
          <View className="gap-1 px-4 py-4">
            <Text className="text-2xl font-black text-ink">#{tag}</Text>
            {hashtag.data && (
              <Text className="text-sm text-ink-soft">
                {t("hashtag.counts", {
                  shorts: compactCount(Number(hashtag.data.shortCount), i18n.language),
                  views: compactCount(Number(hashtag.data.viewCount), i18n.language),
                })}
                {hashtag.data.trendingRank > 0
                  ? ` · ${t("hashtag.trending", { rank: hashtag.data.trendingRank })}`
                  : ""}
              </Text>
            )}
          </View>
        }
        empty={
          <EmptyState emoji="#️⃣" title={t("hashtag.empty", { tag })} body={t("hashtag.emptyBody")} />
        }
      />
    </View>
  );
}
