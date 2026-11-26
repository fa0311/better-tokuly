import type { video } from "@tokuly/api";
import { imageUrl, playlistTitle } from "@tokuly/core/video";
import { Image } from "expo-image";
import { router } from "expo-router";
import { ListVideo } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { mediaUrl } from "#/lib/config";

/** A playlist in a list: its first picture, its name and how many videos it holds. */
export function PlaylistRow({ playlist }: { playlist: video.Playlist }) {
  const { t } = useTranslation("video");
  const uri = mediaUrl(imageUrl(playlist.thumbnail, 160));
  const name = playlistTitle(playlist, t);
  return (
    <Pressable
      testID={`playlist-${playlist.shortId || playlist.id}`}
      accessibilityRole="button"
      accessibilityLabel={name}
      onPress={() =>
        router.push({ pathname: "/playlist/[id]", params: { id: playlist.shortId || playlist.id } })
      }
      className="flex-row items-center gap-3 px-4 py-2"
    >
      <View className="aspect-video w-32 overflow-hidden rounded-xl bg-surface-3">
        {uri ? <Image source={uri} style={{ flex: 1 }} contentFit="cover" /> : null}
        <View className="absolute inset-y-0 right-0 w-12 items-center justify-center bg-black/60">
          <ListVideo color="white" size={18} />
          <Text className="text-xs font-black text-white">{playlist.itemCount}</Text>
        </View>
      </View>
      <View className="flex-1 gap-0.5">
        <Text numberOfLines={2} className="font-black text-ink">
          {name}
        </Text>
        <Text numberOfLines={1} className="text-xs text-ink-soft">
          {playlist.channel?.name ?? t("app.playlistLabel", { count: playlist.itemCount })}
        </Text>
      </View>
    </Pressable>
  );
}
