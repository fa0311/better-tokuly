import { FlashList } from "@shopify/flash-list";
import { type notes, toMillis } from "@tokuly/api";
import { router } from "expo-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { ActionSheet } from "#/components/action-sheet";
import { EmptyState, Spinner } from "#/components/page";
import { reasonText } from "#/lib/errors";
import i18n from "#/lib/i18n";
import { useDeleteDraft, usePublishDraft } from "./actions";
import { composeHref } from "./compose-href";
import { useDrafts } from "./data";

const when = (ms: number) =>
  new Date(ms).toLocaleString(i18n.language, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });

/**
 * The drafts kept on the server (they follow the account to every device)
 * and the notes scheduled for later: a tap goes on writing one, a long
 * press posts it now or deletes it.
 */
export function DraftsScreen() {
  const { t } = useTranslation("notes");
  const drafts = useDrafts();
  const publish = usePublishDraft();
  const remove = useDeleteDraft();
  const [menuFor, setMenuFor] = useState<notes.Draft | null>(null);
  const list = drafts.data?.pages.flatMap((page) => page.drafts) ?? [];

  if (drafts.isPending) return <Spinner />;
  return (
    <View className="flex-1 bg-bg" testID="drafts-screen">
      <FlashList
        data={list}
        keyExtractor={(draft) => draft.id}
        renderItem={({ item }) => (
          <Pressable
            testID={`draft-${item.id}`}
            accessibilityRole="button"
            onPress={() => router.push(composeHref({ draft: item.id }))}
            onLongPress={() => setMenuFor(item)}
            className="gap-1 border-b border-line bg-surface px-4 py-3 active:bg-surface-2"
          >
            <Text className="text-base text-ink" numberOfLines={3}>
              {item.content?.text || t("drafts.noText")}
            </Text>
            <View className="flex-row flex-wrap gap-x-3">
              {item.attachments.length > 0 && (
                <Text className="text-xs text-ink-soft">
                  {t("drafts.attachments", { count: item.attachments.length })}
                </Text>
              )}
              {item.scheduledAt && (
                <Text className="text-xs font-bold text-brand">
                  {t("drafts.scheduledAt", { time: when(toMillis(item.scheduledAt)) })}
                </Text>
              )}
              {item.failureReason && (
                <Text className="text-xs font-bold text-danger">
                  {t("drafts.failed", {
                    reason:
                      reasonText(item.failureReason, { namespace: "notes" }) ?? item.failureReason,
                  })}
                </Text>
              )}
              {item.updatedAt && (
                <Text className="ml-auto text-xs text-ink-faint">
                  {when(toMillis(item.updatedAt))}
                </Text>
              )}
            </View>
          </Pressable>
        )}
        onEndReached={() => {
          if (drafts.hasNextPage && !drafts.isFetchingNextPage) void drafts.fetchNextPage();
        }}
        onRefresh={() => void drafts.refetch()}
        refreshing={drafts.isRefetching && !drafts.isFetchingNextPage}
        ListEmptyComponent={
          <EmptyState emoji="🗒️" title={t("drafts.empty")} body={t("drafts.emptyBody")} />
        }
      />
      <ActionSheet
        visible={menuFor !== null}
        actions={
          menuFor
            ? [
                {
                  key: "publish",
                  label: t("drafts.publish"),
                  onPress: () => void publish(menuFor),
                },
                {
                  key: "delete",
                  label: t("drafts.delete"),
                  destructive: true,
                  onPress: () =>
                    void remove(menuFor).then((gone) => {
                      if (gone) Alert.alert(t("drafts.deleted"));
                    }),
                },
              ]
            : []
        }
        onClose={() => setMenuFor(null)}
      />
    </View>
  );
}
