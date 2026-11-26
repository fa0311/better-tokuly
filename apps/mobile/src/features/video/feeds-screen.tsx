import { FlashList } from "@shopify/flash-list";
import { video } from "@tokuly/api";
import {
  CATEGORIES,
  type Dismissal,
  FEED_SOURCES,
  FEEDS,
  type Feed,
  feedVideos,
} from "@tokuly/core/video";
import { router, Stack } from "expo-router";
import { Library } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Pressable,
  RefreshControl,
  ScrollView,
  Text,
  useWindowDimensions,
  View,
} from "react-native";
import { EmptyState, Spinner } from "#/components/page";
import { useTheme } from "#/lib/theme";
import { alertError, feedClient, useFeed } from "./data";
import { VideoCard } from "./video-card";

const { VideoCategory } = video;

/**
 * The video feeds: recommendations, the channels the viewer subscribes to
 * and what is trending, with genre chips over them. Holding a card in the
 * home feed turns a video, or a whole channel, away (undone in place).
 */
export function FeedsScreen() {
  const { t } = useTranslation("video");
  const { colors } = useTheme();
  const { width } = useWindowDimensions();
  const [feed, setFeed] = useState<Feed>("home");
  const [category, setCategory] = useState<video.VideoCategory>(VideoCategory.UNSPECIFIED);
  const [dismissed, setDismissed] = useState<Record<string, Dismissal>>({});
  const query = useFeed(feed, category);
  const videos = feedVideos(query.data?.pages ?? [], feed, category, dismissed);

  const mark = async (summary: video.VideoSummary, entry: Dismissal | undefined) => {
    const client = feedClient();
    const kind = entry ?? dismissed[summary.id];
    if (!kind) return;
    const target =
      kind.kind === "video"
        ? { case: "videoId" as const, value: summary.id }
        : { case: "channelId" as const, value: kind.channelId };
    try {
      if (entry) await client.markNotInterested({ target });
      else await client.undoNotInterested({ target });
    } catch (error) {
      alertError(error);
    }
  };
  const dismiss = (summary: video.VideoSummary, kind: Dismissal["kind"]) => {
    const entry: Dismissal =
      kind === "video" ? { kind } : { kind, channelId: summary.channel?.id ?? "" };
    setDismissed((current) => ({ ...current, [summary.id]: entry }));
    void mark(summary, entry);
  };
  const restore = (summary: video.VideoSummary) => {
    void mark(summary, undefined);
    setDismissed(({ [summary.id]: _restored, ...rest }) => rest);
  };

  return (
    <View className="flex-1 bg-bg" testID="video-feeds">
      <Stack.Screen
        options={{
          headerRight: () => (
            <Pressable
              testID="video-library"
              accessibilityRole="button"
              accessibilityLabel={t("app.library")}
              onPress={() => router.push("/watch/library")}
              className="p-1"
            >
              <Library color={colors.ink} size={24} />
            </Pressable>
          ),
        }}
      />
      <FlashList
        data={videos}
        keyExtractor={(summary) => summary.id}
        refreshControl={
          <RefreshControl refreshing={query.isRefetching} onRefresh={() => void query.refetch()} />
        }
        onEndReached={() => query.hasNextPage && void query.fetchNextPage()}
        ListHeaderComponent={
          <View className="gap-3 pt-3 pb-4">
            <View className="flex-row gap-2 px-4">
              {FEEDS.map((key) => (
                <Pressable
                  key={key}
                  testID={`video-feed-${key}`}
                  accessibilityRole="tab"
                  accessibilityState={{ selected: feed === key }}
                  onPress={() => setFeed(key)}
                  className={`h-9 justify-center rounded-full px-4 ${feed === key ? "bg-ink" : "bg-surface-2"}`}
                >
                  <Text className={`font-black ${feed === key ? "text-bg" : "text-ink"}`}>
                    {t(`feeds.${key}`)}
                  </Text>
                </Pressable>
              ))}
            </View>
            <ScrollView horizontal showsHorizontalScrollIndicator={false}>
              <View className="flex-row gap-2 px-4">
                <CategoryChip
                  label={t("feeds.allCategories")}
                  active={category === VideoCategory.UNSPECIFIED}
                  onPress={() => setCategory(VideoCategory.UNSPECIFIED)}
                />
                {CATEGORIES.map((entry) => (
                  <CategoryChip
                    key={entry.value}
                    label={`${entry.emoji} ${t(`category.${entry.key}`)}`}
                    active={category === entry.value}
                    onPress={() => setCategory(entry.value)}
                  />
                ))}
              </View>
            </ScrollView>
            {feed === "subscriptions" ? (
              <Pressable
                testID="video-all-subscriptions"
                accessibilityRole="link"
                onPress={() => router.push("/watch/channels")}
                className="px-4"
              >
                <Text className="font-black text-brand">{t("app.allSubscriptions")} →</Text>
              </Pressable>
            ) : null}
          </View>
        }
        ListEmptyComponent={
          query.isPending ? (
            <Spinner />
          ) : (
            <FeedEmpty feed={feed} filtered={category !== VideoCategory.UNSPECIFIED} />
          )
        }
        renderItem={({ item }) =>
          dismissed[item.id] ? (
            <View className="mx-4 mb-5 flex-row items-center gap-3 rounded-card bg-surface-2 px-4 py-4">
              <Text className="flex-1 text-sm font-bold text-ink-soft">
                {dismissed[item.id]?.kind === "video"
                  ? t("feeds.dismissedVideo")
                  : t("feeds.dismissedChannel", { name: item.channel?.name ?? "" })}
              </Text>
              <Pressable accessibilityRole="button" onPress={() => restore(item)}>
                <Text className="font-black text-brand">{t("feeds.undo")}</Text>
              </Pressable>
            </View>
          ) : (
            <VideoCard
              summary={item}
              source={FEED_SOURCES[feed]}
              layout="large"
              width={width}
              menu={
                feed === "home"
                  ? [
                      {
                        key: "not-interested",
                        label: t("feeds.notInterested"),
                        onPress: () => dismiss(item, "video"),
                      },
                      {
                        key: "hide-channel",
                        label: t("feeds.hideChannel"),
                        onPress: () => dismiss(item, "channel"),
                      },
                    ]
                  : []
              }
            />
          )
        }
        ListFooterComponent={query.isFetchingNextPage ? <Spinner /> : null}
      />
    </View>
  );
}

function CategoryChip({
  label,
  active,
  onPress,
}: {
  label: string;
  active: boolean;
  onPress(): void;
}) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityState={{ selected: active }}
      onPress={onPress}
      className={`h-8 justify-center rounded-full px-3 ${active ? "bg-brand" : "bg-surface"}`}
    >
      <Text className={`text-sm font-bold ${active ? "text-on-brand" : "text-ink"}`}>{label}</Text>
    </Pressable>
  );
}

function FeedEmpty({ feed, filtered }: { feed: Feed; filtered: boolean }) {
  const { t } = useTranslation("video");
  if (filtered)
    return (
      <EmptyState emoji="🔍" title={t("feeds.emptyCategory")} body={t("feeds.emptyCategoryBody")} />
    );
  if (feed === "subscriptions")
    return (
      <EmptyState
        emoji="📺"
        title={t("feeds.emptySubscriptions")}
        body={t("feeds.emptySubscriptionsBody")}
      />
    );
  if (feed === "trending") return <EmptyState emoji="📈" title={t("feeds.emptyTrending")} />;
  return <EmptyState emoji="🎬" title={t("feeds.emptyHome")} body={t("feeds.emptyHomeBody")} />;
}
