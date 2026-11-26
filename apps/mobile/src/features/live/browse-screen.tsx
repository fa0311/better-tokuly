import { useQueryClient } from "@tanstack/react-query";
import { live, video } from "@tokuly/api";
import { liveKeys } from "@tokuly/core/live";
import { router } from "expo-router";
import { Gift, Video } from "lucide-react-native";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { FlatList, Pressable, RefreshControl, ScrollView, Text, View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState, Spinner } from "#/components/page";
import { Tabs } from "#/components/tabs";
import { webScreenHref } from "#/features/web/handoff";
import { realtime } from "#/lib/realtime";
import { useTheme } from "#/lib/theme";
import { useFriendStreams, useLiveNow, useSubscribedStreams, useUpcoming } from "./data";
import { StreamCard } from "./parts";

type Tab = "now" | "upcoming";

/**
 * Going live stays on the web screen: the phone app has no WebRTC (WHIP) or
 * RTMP publisher, and the studio there sets the stream up and publishes from
 * the camera in the browser.
 */
export const GO_LIVE_HREF = webScreenHref("/studio/live");

/** A row of cards that scrolls sideways; nothing when it is empty. */
function Shelf({
  title,
  streams,
  testID,
}: {
  title: string;
  streams: live.StreamSummary[];
  testID: string;
}) {
  if (streams.length === 0) return null;
  return (
    <View testID={testID} className="gap-2 pb-5">
      <Text className="px-4 text-lg font-black text-ink">{title}</Text>
      <ScrollView horizontal showsHorizontalScrollIndicator={false}>
        <View className="flex-row gap-3 px-4">
          {streams.map((stream) => (
            <StreamCard key={stream.id} summary={stream} width={240} />
          ))}
        </View>
      </ScrollView>
    </View>
  );
}

/** Reads every list again when the connection comes back from a gap: streams start and end all the time. */
function useListsAfterGaps() {
  const queryClient = useQueryClient();
  useEffect(() => {
    let welcomed = false;
    return realtime().onFrame((frame) => {
      if (frame.body.case !== "welcome") return;
      if (welcomed) void queryClient.invalidateQueries({ queryKey: liveKeys.lists });
      welcomed = true;
    });
  }, [queryClient]);
}

/**
 * `/live`: what is on air — friends first, then the channels the viewer
 * subscribes to, then everyone public — and what is scheduled.
 */
export function BrowseLiveScreen() {
  const { t } = useTranslation("live");
  const { colors } = useTheme();
  const [tab, setTab] = useState<Tab>("now");
  useListsAfterGaps();
  return (
    <View className="flex-1 bg-bg" testID="live-browse">
      <View className="flex-row items-center justify-between gap-2 px-4 pt-3 pb-2">
        <Tabs
          testID="live-tab"
          items={(["now", "upcoming"] as const).map((key) => ({
            value: key,
            label: t(`browse.tabs.${key}`),
          }))}
          value={tab}
          onChange={setTab}
        />
        <View className="flex-row items-center gap-1">
          <Pressable
            testID="live-gift-history"
            accessibilityRole="button"
            accessibilityLabel={t("giftHistory.title")}
            onPress={() => router.push(webScreenHref("/live/gifts") as never)}
            className="p-2"
          >
            <Gift size={22} color={colors.ink} />
          </Pressable>
          <Button
            testID="live-go-live"
            size="sm"
            icon={<Video size={16} color={colors["on-brand"]} />}
            onPress={() => router.push(GO_LIVE_HREF as never)}
          >
            {t("browse.goLive")}
          </Button>
        </View>
      </View>
      {tab === "now" ? <LiveNow /> : <Upcoming />}
    </View>
  );
}

function LiveNow() {
  const { t } = useTranslation("live");
  const friends = useFriendStreams();
  const subscribed = useSubscribedStreams();
  const query = useLiveNow(video.VideoCategory.UNSPECIFIED);
  const streams = query.data?.pages.flatMap((page) => page.streams) ?? [];
  // A friend's stream is not repeated under the channels the viewer subscribes to.
  const friendIds = new Set(friends.data?.map((stream) => stream.id));
  const onAirFriends = (friends.data ?? []).filter(
    (stream) => stream.status === live.StreamStatus.LIVE,
  );
  return (
    <FlatList
      testID="live-now-list"
      data={streams}
      keyExtractor={(stream) => stream.id}
      contentContainerClassName="pb-12"
      refreshControl={
        <RefreshControl
          refreshing={query.isRefetching}
          onRefresh={() => {
            void friends.refetch();
            void subscribed.refetch();
            void query.refetch();
          }}
        />
      }
      ListHeaderComponent={
        <View className="pt-2">
          <Shelf title={t("browse.friends")} streams={onAirFriends} testID="friend-streams" />
          <Shelf
            title={t("browse.subscribed")}
            streams={(subscribed.data ?? []).filter((stream) => !friendIds.has(stream.id))}
            testID="subscribed-streams"
          />
          <Text className="px-4 pb-2 text-lg font-black text-ink">{t("browse.liveNow")}</Text>
        </View>
      }
      renderItem={({ item }) => (
        <View className="px-4 pb-5">
          <StreamCard summary={item} />
        </View>
      )}
      ListEmptyComponent={
        query.isPending ? (
          <Spinner />
        ) : (
          <EmptyState
            emoji="📡"
            title={t("browse.emptyNowTitle")}
            body={t("browse.emptyNowBody")}
          />
        )
      }
      onEndReached={() => {
        if (query.hasNextPage && !query.isFetchingNextPage) void query.fetchNextPage();
      }}
    />
  );
}

function Upcoming() {
  const { t } = useTranslation("live");
  const friends = useFriendStreams();
  const query = useUpcoming(video.VideoCategory.UNSPECIFIED);
  const streams = query.data?.pages.flatMap((page) => page.streams) ?? [];
  return (
    <FlatList
      testID="live-upcoming-list"
      data={streams}
      keyExtractor={(stream) => stream.id}
      contentContainerClassName="pb-12"
      refreshControl={
        <RefreshControl refreshing={query.isRefetching} onRefresh={() => void query.refetch()} />
      }
      ListHeaderComponent={
        <View className="pt-2">
          <Shelf
            title={t("browse.friendsUpcoming")}
            streams={(friends.data ?? []).filter(
              (stream) => stream.status === live.StreamStatus.UPCOMING,
            )}
            testID="friend-upcoming"
          />
        </View>
      }
      renderItem={({ item }) => (
        <View className="px-4 pb-5">
          <StreamCard summary={item} />
        </View>
      )}
      ListEmptyComponent={
        query.isPending ? (
          <Spinner />
        ) : (
          <EmptyState
            emoji="🗓️"
            title={t("browse.emptyUpcomingTitle")}
            body={t("browse.emptyUpcomingBody")}
          />
        )
      }
      onEndReached={() => {
        if (query.hasNextPage && !query.isFetchingNextPage) void query.fetchNextPage();
      }}
    />
  );
}
