import { type FeedContext, soundTitle } from "@tokuly/core/shorts";
import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";
import { EmptyState, Spinner } from "#/components/page";
import { useShortList, useSound } from "./data";
import { ShortGrid, watch } from "./grid";

/** A sound: what it is, and the public shorts using it, newest first. */
export function SoundScreen({ soundId }: { soundId: string }) {
  const { t } = useTranslation("shorts");
  const context: FeedContext = { kind: "sound", key: soundId };
  const sound = useSound(soundId);
  const list = useShortList(context);
  const items = list.data?.pages.flatMap((page) => page.entries.map((entry) => entry.short)) ?? [];
  if (sound.isPending || list.isPending) return <Spinner />;
  return (
    <View className="flex-1 bg-bg" testID="sound-screen">
      <ShortGrid
        shorts={items}
        onPress={(short) => watch(context, short)}
        onEnd={() => list.hasNextPage && void list.fetchNextPage()}
        header={
          <View className="gap-1 px-4 py-4">
            <Text className="text-xl font-black text-ink">
              🎵 {sound.data ? soundTitle(sound.data, (name) => t("sound.original", { name })) : ""}
            </Text>
            {sound.data && (
              <Text className="text-sm text-ink-soft">
                {t("sound.uses", { count: Number(sound.data.useCount) })}
              </Text>
            )}
          </View>
        }
        empty={<EmptyState emoji="🎵" title={t("sound.empty")} body={t("sound.emptyBody")} />}
      />
    </View>
  );
}
