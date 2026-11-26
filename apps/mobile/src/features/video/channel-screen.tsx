import { FlashList } from "@shopify/flash-list";
import { toMillis, video } from "@tokuly/api";
import { formatCount, imageUrl } from "@tokuly/core/video";
import { Image } from "expo-image";
import { router, Stack } from "expo-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Linking, Pressable, Text, useWindowDimensions, View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState, Spinner } from "#/components/page";
import { webScreenHref } from "#/features/web/handoff";
import { mediaUrl } from "#/lib/config";
import { useChannelPage, useChannelPlaylists, useChannelVideos } from "./data";
import { PlaylistRow } from "./playlist-row";
import { SubscribeButton } from "./subscribe-button";
import { ChannelAvatar, VideoCard } from "./video-card";

const { ChannelVideoSort, TrafficSource } = video;

const TABS = ["videos", "playlists", "about"] as const;
type Tab = (typeof TABS)[number];

const SORTS = [
  { sort: ChannelVideoSort.NEWEST, key: "newest" },
  { sort: ChannelVideoSort.POPULAR, key: "popular" },
  { sort: ChannelVideoSort.OLDEST, key: "oldest" },
] as const;

type Row =
  | { kind: "video"; video: video.VideoSummary }
  | { kind: "playlist"; playlist: video.Playlist }
  | { kind: "about" };

/**
 * `/c/<handle>`: the channel's banner, face and numbers, subscribe, and its
 * videos (newest, popular, oldest), public playlists and about tab. The
 * owner edits it in the studio on the web.
 */
export function ChannelScreen({ handle }: { handle: string }) {
  const { t, i18n } = useTranslation("video");
  const { width } = useWindowDimensions();
  const page = useChannelPage(handle);
  const channel = page.data?.channel;
  const summary = channel?.summary;
  const [tab, setTab] = useState<Tab>("videos");
  const [sort, setSort] = useState<video.ChannelVideoSort>(ChannelVideoSort.NEWEST);
  const videos = useChannelVideos(handle, tab === "videos" ? summary?.id : undefined, sort);
  const playlists = useChannelPlaylists(handle, tab === "playlists" ? summary?.id : undefined);

  if (page.isPending) return <Spinner />;
  if (!channel || !summary)
    return (
      <View className="flex-1 bg-bg" testID="channel-not-found">
        <EmptyState emoji="🫥" title={t("channel.notFound")} body={t("channel.notFoundBody")} />
      </View>
    );

  const rows: Row[] =
    tab === "videos"
      ? (videos.data?.pages.flatMap((p) => p.videos) ?? []).map((entry) => ({
          kind: "video",
          video: entry,
        }))
      : tab === "playlists"
        ? (playlists.data?.pages.flatMap((p) => p.playlists) ?? []).map((entry) => ({
            kind: "playlist",
            playlist: entry,
          }))
        : [{ kind: "about" }];
  const active = tab === "videos" ? videos : tab === "playlists" ? playlists : null;
  const banner = mediaUrl(imageUrl(channel.banner, width));
  const isOwner = channel.viewer?.isOwner ?? false;

  return (
    <View className="flex-1 bg-bg" testID="channel-screen">
      <Stack.Screen options={{ title: summary.name }} />
      <FlashList
        data={rows}
        keyExtractor={(row, index) =>
          row.kind === "video"
            ? row.video.id
            : row.kind === "playlist"
              ? row.playlist.id
              : `${index}`
        }
        onEndReached={() => active?.hasNextPage && void active.fetchNextPage()}
        ListHeaderComponent={
          <View className="gap-3 pb-3">
            {banner ? (
              <Image source={banner} style={{ width, height: width / 4 }} contentFit="cover" />
            ) : null}
            <View className="flex-row items-center gap-3 px-4 pt-2">
              <ChannelAvatar channel={summary} size="lg" />
              <View className="flex-1 gap-0.5">
                <Text
                  testID="channel-name"
                  numberOfLines={1}
                  className="text-xl font-black text-ink"
                >
                  {summary.name}
                  {summary.verified ? " ✔︎" : ""}
                </Text>
                <Text numberOfLines={1} className="text-xs font-bold text-ink-soft">
                  @{summary.handle} ·{" "}
                  {t("channel.subscribers", {
                    count: Number(summary.subscriberCount),
                    formatted: formatCount(i18n.language, summary.subscriberCount),
                  })}{" "}
                  ·{" "}
                  {t("channel.videoCount", {
                    count: Number(channel.videoCount),
                    formatted: formatCount(i18n.language, channel.videoCount),
                  })}
                </Text>
              </View>
            </View>
            <View className="px-4">
              {isOwner ? (
                <Button
                  size="sm"
                  variant="secondary"
                  onPress={() =>
                    router.push(webScreenHref(`/studio/channels/${summary.id}`) as never)
                  }
                >
                  {t("channel.edit")}
                </Button>
              ) : (
                <SubscribeButton
                  channel={summary}
                  subscribed={channel.viewer?.subscribed ?? false}
                  notificationLevel={channel.viewer?.notificationLevel}
                />
              )}
            </View>
            <View className="flex-row gap-5 border-line border-b px-4">
              {TABS.map((key) => (
                <Pressable
                  key={key}
                  testID={`channel-tab-${key}`}
                  accessibilityRole="tab"
                  accessibilityState={{ selected: tab === key }}
                  onPress={() => setTab(key)}
                  className={`pb-2 ${tab === key ? "border-ink border-b-2" : ""}`}
                >
                  <Text className={`font-black ${tab === key ? "text-ink" : "text-ink-soft"}`}>
                    {t(`channel.tabs.${key}`)}
                  </Text>
                </Pressable>
              ))}
            </View>
            {tab === "videos" ? (
              <View className="flex-row gap-2 px-4">
                {SORTS.map((entry) => (
                  <Pressable
                    key={entry.key}
                    accessibilityRole="button"
                    accessibilityState={{ selected: sort === entry.sort }}
                    onPress={() => setSort(entry.sort)}
                    className={`h-8 justify-center rounded-full px-3 ${sort === entry.sort ? "bg-ink" : "bg-surface-2"}`}
                  >
                    <Text
                      className={`text-sm font-bold ${sort === entry.sort ? "text-bg" : "text-ink"}`}
                    >
                      {t(`channel.sort.${entry.key}`)}
                    </Text>
                  </Pressable>
                ))}
              </View>
            ) : null}
          </View>
        }
        ListEmptyComponent={
          active?.isPending ? (
            <Spinner />
          ) : tab === "videos" ? (
            <EmptyState emoji="📼" title={t("channel.noVideos")} />
          ) : (
            <EmptyState emoji="📂" title={t("channel.noPlaylists")} />
          )
        }
        renderItem={({ item }) =>
          item.kind === "video" ? (
            <VideoCard summary={item.video} source={TrafficSource.CHANNEL} />
          ) : item.kind === "playlist" ? (
            <PlaylistRow playlist={item.playlist} />
          ) : (
            <About channel={channel} />
          )
        }
      />
    </View>
  );
}

function About({ channel }: { channel: video.Channel }) {
  const { t, i18n } = useTranslation("video");
  const created = channel.createdAt ? new Date(toMillis(channel.createdAt)) : undefined;
  const stat = (label: string, value: string) => (
    <View className="flex-row justify-between py-1">
      <Text className="text-ink-soft">{label}</Text>
      <Text className="font-black text-ink">{value}</Text>
    </View>
  );
  return (
    <View testID="channel-about" className="gap-4 px-4 pt-2 pb-10">
      <View className="gap-1">
        <Text className="font-black text-ink">{t("channel.about.description")}</Text>
        <Text className="leading-relaxed text-ink">
          {channel.description || t("channel.about.noDescription")}
        </Text>
      </View>
      {channel.links.length > 0 ? (
        <View className="gap-1">
          <Text className="font-black text-ink">{t("channel.about.links")}</Text>
          {channel.links.map((link) => (
            <Text
              key={link.url}
              accessibilityRole="link"
              onPress={() => void Linking.openURL(link.url)}
              className="font-bold text-brand"
            >
              {link.title || link.url}
            </Text>
          ))}
        </View>
      ) : null}
      <View className="rounded-card bg-surface-2 px-4 py-3">
        <Text className="pb-1 font-black text-ink">{t("channel.about.stats")}</Text>
        {stat(
          t("channel.about.subscribers"),
          formatCount(i18n.language, channel.summary?.subscriberCount ?? 0n),
        )}
        {stat(t("channel.about.videos"), formatCount(i18n.language, channel.videoCount))}
        {stat(t("channel.about.views"), formatCount(i18n.language, channel.viewCount))}
        {created
          ? stat(
              t("channel.about.joined"),
              created.toLocaleDateString(i18n.language, {
                year: "numeric",
                month: "long",
                day: "numeric",
              }),
            )
          : null}
      </View>
    </View>
  );
}
