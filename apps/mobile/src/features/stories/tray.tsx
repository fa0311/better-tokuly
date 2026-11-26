import type { notes } from "@tokuly/api";
import { router } from "expo-router";
import { Plus } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Pressable, ScrollView, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { useMe } from "#/features/session/session";
import { useTheme } from "#/lib/theme";
import { useStoryTray } from "./data";

function Ring({ reel, mine }: { reel: notes.StoryReel; mine: boolean }) {
  const { t } = useTranslation("stories");
  const { colors } = useTheme();
  const user = reel.user;
  const name = user?.displayName ?? "";
  const unseen = !mine && reel.hasUnseen;
  return (
    <Pressable
      testID={`story-ring-${user?.handle}`}
      accessibilityRole="button"
      accessibilityLabel={
        mine ? t("tray.mine") : unseen ? t("tray.unseen", { name }) : t("tray.seen", { name })
      }
      onPress={() =>
        router.push({ pathname: "/stories/[userId]", params: { userId: user?.id ?? "" } })
      }
      className="w-[72px] items-center gap-1"
    >
      <View
        testID={unseen ? `story-unseen-${user?.handle}` : undefined}
        className="rounded-[22px] p-[3px]"
        style={{ borderWidth: 3, borderColor: unseen ? colors.pink : colors.line }}
      >
        <Avatar user={user} size="lg" />
      </View>
      <Text
        numberOfLines={1}
        className={`text-xs ${unseen ? "font-black text-ink" : "text-ink-soft"}`}
      >
        {mine ? t("tray.mine") : name}
      </Text>
    </Pressable>
  );
}

/**
 * The row of rings: the viewer's own first (or a "+" to leave one), then
 * friends with stories they have not watched, then the rest. A ring opens
 * the viewer, which plays on through the row.
 */
export function StoryTray() {
  const { t } = useTranslation("stories");
  const me = useMe();
  const reels = useStoryTray().data ?? [];
  const myId = me?.user?.id;
  const own = reels.find((reel) => reel.user?.id === myId);
  const others = reels.filter((reel) => reel.user?.id !== myId);
  return (
    <ScrollView
      horizontal
      testID="story-tray"
      showsHorizontalScrollIndicator={false}
      contentContainerClassName="gap-2 px-3 py-2"
    >
      <View className="items-center">
        {own ? (
          <Ring reel={own} mine />
        ) : (
          <Pressable
            testID="story-add"
            accessibilityRole="button"
            accessibilityLabel={t("tray.add")}
            onPress={() => router.push("/stories/new")}
            className="w-[72px] items-center gap-1"
          >
            <View
              className="rounded-[22px] p-[3px]"
              style={{ borderWidth: 3, borderColor: "transparent" }}
            >
              <Avatar user={me?.user} size="lg" />
            </View>
            <Text numberOfLines={1} className="text-xs font-black text-brand">
              {t("tray.add")}
            </Text>
          </Pressable>
        )}
        <Pressable
          testID="story-new"
          accessibilityRole="button"
          accessibilityLabel={t("tray.add")}
          onPress={() => router.push("/stories/new")}
          className="absolute top-12 right-1 rounded-full border-2 border-surface bg-brand p-0.5"
        >
          <Plus color="white" size={16} strokeWidth={3} />
        </Pressable>
      </View>
      {others.map((reel) => (
        <Ring key={reel.user?.id} reel={reel} mine={false} />
      ))}
    </ScrollView>
  );
}
