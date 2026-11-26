import { FlashList } from "@shopify/flash-list";
import { toMillis } from "@tokuly/api";
import { compactAge } from "@tokuly/core/format";
import { useTranslation } from "react-i18next";
import { Modal, Pressable, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { Avatar } from "#/components/avatar";
import { EmptyState, Spinner } from "#/components/page";
import i18n from "#/lib/i18n";
import { useStoryViewers } from "./data";

/** For the owner: who watched one of their stories and how they reacted, most recent first. */
export function ViewersSheet({
  storyId,
  visible,
  onClose,
}: {
  storyId: string;
  visible: boolean;
  onClose(): void;
}) {
  const { t } = useTranslation("stories");
  const insets = useSafeAreaInsets();
  const viewers = useStoryViewers(storyId, visible);
  const rows = viewers.data?.pages.flatMap((page) => page.viewers) ?? [];
  const first = viewers.data?.pages[0];
  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onClose}>
      <Pressable accessibilityRole="none" className="flex-1 bg-black/40" onPress={onClose} />
      <View
        testID="story-viewers-sheet"
        className="h-[60%] rounded-t-3xl bg-surface pt-4"
        style={{ paddingBottom: insets.bottom }}
      >
        <Text className="px-5 text-lg font-black text-ink">{t("viewers.title")}</Text>
        {first && (
          <Text className="px-5 pb-2 text-sm text-ink-soft">
            {t("viewers.summary", { views: first.viewCount, reactions: first.reactionCount })}
          </Text>
        )}
        {viewers.isPending ? (
          <Spinner />
        ) : rows.length === 0 ? (
          <EmptyState emoji="👀" title={t("viewers.empty")} body={t("viewers.emptyBody")} />
        ) : (
          <FlashList
            data={rows}
            keyExtractor={(row) => row.user?.id ?? ""}
            onEndReached={() => viewers.hasNextPage && void viewers.fetchNextPage()}
            renderItem={({ item }) => (
              <View className="flex-row items-center gap-3 px-5 py-2.5">
                <Avatar user={item.user} size="md" />
                <View className="flex-1">
                  <Text className="font-black text-ink" numberOfLines={1}>
                    {item.user?.displayName}
                  </Text>
                  <Text className="text-xs text-ink-soft">
                    {compactAge(i18n, toMillis(item.viewedAt))}
                  </Text>
                </View>
                {item.reaction ? (
                  <Text
                    className="text-2xl"
                    accessibilityLabel={t("viewers.reacted", { emoji: item.reaction })}
                  >
                    {item.reaction}
                  </Text>
                ) : null}
              </View>
            )}
          />
        )}
      </View>
    </Modal>
  );
}
