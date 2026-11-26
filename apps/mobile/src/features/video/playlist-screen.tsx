import { FlashList } from "@shopify/flash-list";
import { useQueryClient } from "@tanstack/react-query";
import { video } from "@tokuly/api";
import { playlistTitle, videoKeys } from "@tokuly/core/video";
import { Stack } from "expo-router";
import { Play } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState, Spinner } from "#/components/page";
import { usePlaylist, usePlaylistItems, useSaveTo } from "./data";
import { openVideo, VideoCard } from "./video-card";

const { PlaylistKind, TrafficSource } = video;

/**
 * A playlist: its name and who made it, "play all", and its videos, each
 * played in the list. The owner takes videos out by holding them (liked
 * videos leave by taking the like back); editing and reordering stay on the web.
 */
export function PlaylistScreen({ playlistId }: { playlistId: string }) {
  const { t } = useTranslation("video");
  const queryClient = useQueryClient();
  const playlist = usePlaylist(playlistId);
  const items = usePlaylistItems(playlistId);
  const saveTo = useSaveTo();
  if (playlist.isPending) return <Spinner />;
  const found = playlist.data;
  if (!found)
    return (
      <View className="flex-1 bg-bg" testID="playlist-not-found">
        <EmptyState emoji="🫥" title={t("playlist.notFound")} body={t("playlist.notFoundBody")} />
      </View>
    );
  const name = playlistTitle(found, t);
  const list = found.shortId || found.id;
  const videos =
    items.data?.pages.flatMap((page) =>
      page.items.flatMap((item) => (item.video ? [item.video] : [])),
    ) ?? [];
  const removable = found.isOwner && found.kind !== PlaylistKind.LIKED;
  const remove = async (summary: video.VideoSummary) => {
    if (await saveTo(found.id, summary, false))
      void queryClient.invalidateQueries({ queryKey: videoKeys.playlistItems(playlistId) });
  };
  return (
    <View className="flex-1 bg-bg" testID="playlist-screen">
      <Stack.Screen options={{ title: name }} />
      <FlashList
        data={videos}
        keyExtractor={(summary) => summary.id}
        onEndReached={() => items.hasNextPage && void items.fetchNextPage()}
        ListHeaderComponent={
          <View className="gap-2 px-4 pt-4 pb-3">
            <Text testID="playlist-title" className="text-2xl font-black text-ink">
              {name}
            </Text>
            <Text className="text-sm text-ink-soft">
              {[
                found.channel?.name ?? found.owner?.displayName,
                t("playlist.items", { count: found.itemCount }),
              ]
                .filter(Boolean)
                .join(" · ")}
            </Text>
            {found.description ? <Text className="text-ink">{found.description}</Text> : null}
            {found.kind === PlaylistKind.LIKED ? (
              <Text className="text-xs text-ink-soft">{t("playlist.likedHint")}</Text>
            ) : null}
            {videos[0] ? (
              <Button
                testID="playlist-play-all"
                icon={<Play color="white" size={18} />}
                onPress={() => videos[0] && openVideo(videos[0], TrafficSource.PLAYLIST, list)}
              >
                {t("playlist.playAll")}
              </Button>
            ) : null}
          </View>
        }
        ListEmptyComponent={
          items.isPending ? (
            <Spinner />
          ) : (
            <EmptyState
              emoji="📂"
              title={t("playlist.empty")}
              body={found.isOwner ? t("playlist.emptyOwnerBody") : undefined}
            />
          )
        }
        renderItem={({ item }) => (
          <VideoCard
            summary={item}
            source={TrafficSource.PLAYLIST}
            list={list}
            menu={
              removable
                ? [
                    {
                      key: "remove",
                      label: t("playlist.removeItem"),
                      destructive: true,
                      onPress: () => void remove(item),
                    },
                  ]
                : []
            }
          />
        )}
      />
    </View>
  );
}
