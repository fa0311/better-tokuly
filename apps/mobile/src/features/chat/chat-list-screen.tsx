import { FlashList } from "@shopify/flash-list";
import { useQueryClient } from "@tanstack/react-query";
import type { chat } from "@tokuly/api";
import { isMuted, type ListFilter } from "@tokuly/core/chat";
import { router } from "expo-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { ActionSheet, type SheetAction } from "#/components/action-sheet";
import { Button } from "#/components/button";
import { EmptyState, Spinner } from "#/components/page";
import { PostsBanner } from "#/features/media/posts-banner";
import { useMe } from "#/features/session/session";
import { StoryTray } from "#/features/stories/tray";
import { errorMessage } from "#/lib/errors";
import { ConversationRow } from "./conversation-row";
import { markRead, setPreferences, useConversationList, useUnread } from "./data";
import { titleOf } from "./text";

const FILTERS: ListFilter[] = ["inbox", "requests", "archived"];

/** The conversations: the inbox, message requests and the archive, newest (and pinned) first. */
export function ChatListScreen() {
  const { t } = useTranslation("chat");
  const { t: tm } = useTranslation("mobile");
  const [filter, setFilter] = useState<ListFilter>("inbox");
  const unread = useUnread();
  const counts: Record<ListFilter, number> = {
    inbox: unread.unreadConversations,
    requests: unread.pendingRequests,
    archived: unread.archivedUnread,
  };
  return (
    <View className="flex-1 bg-bg" testID="chat-list-screen">
      <View className="flex-row items-center gap-2 px-4 pt-3 pb-2">
        {FILTERS.map((key) => (
          <Pressable
            key={key}
            testID={`chat-tab-${key}`}
            accessibilityRole="tab"
            accessibilityState={{ selected: filter === key }}
            onPress={() => setFilter(key)}
            className={`rounded-full px-3 py-2 ${filter === key ? "bg-ink" : "bg-surface"}`}
          >
            <Text className={`font-black ${filter === key ? "text-surface" : "text-ink"}`}>
              {t(`list.tabs.${key}`)}
              {counts[key] > 0 ? ` ${counts[key]}` : ""}
            </Text>
          </Pressable>
        ))}
        <View className="ml-auto">
          <Button testID="chat-new" size="sm" onPress={() => router.push("/new-chat")}>
            {tm("chat.newChat")}
          </Button>
        </View>
      </View>
      {filter === "inbox" && (
        <>
          <StoryTray />
          <PostsBanner kind="story" />
        </>
      )}
      <ConversationList filter={filter} />
    </View>
  );
}

function ConversationList({ filter }: { filter: ListFilter }) {
  const { t } = useTranslation("chat");
  const queryClient = useQueryClient();
  const myId = useMe()?.user?.id ?? "";
  const list = useConversationList(filter);
  const conversations = list.data?.pages.flatMap((page) => page.conversations) ?? [];

  const run = (work: Promise<unknown>) =>
    work.catch((error) => Alert.alert(errorMessage(error, { namespace: "chat" })));

  // A long press: what the web's row menu offers that matters on the go.
  const [menuFor, setMenuFor] = useState<chat.Conversation | null>(null);
  const viewer = menuFor?.viewer;
  const id = menuFor?.id ?? "";
  const actions: SheetAction[] = menuFor
    ? [
        ...((viewer?.unreadCount ?? 0) > 0
          ? [
              {
                key: "read",
                label: t("menu.markRead"),
                onPress: () => void run(markRead(queryClient, id)),
              },
            ]
          : []),
        {
          key: "pin",
          label: viewer?.pinned ? t("menu.unpin") : t("menu.pin"),
          onPress: () => void run(setPreferences(queryClient, id, { pinned: !viewer?.pinned })),
        },
        {
          key: "mute",
          label: isMuted(viewer) ? t("menu.unmute") : t("menu.mute"),
          onPress: () => void run(setPreferences(queryClient, id, { muted: !isMuted(viewer) })),
        },
        {
          key: "archive",
          label: viewer?.archived ? t("menu.unarchive") : t("menu.archive"),
          onPress: () => void run(setPreferences(queryClient, id, { archived: !viewer?.archived })),
        },
      ]
    : [];

  if (list.isPending) return <Spinner />;
  return (
    <>
      <ActionSheet
        visible={menuFor !== null}
        title={menuFor ? titleOf(menuFor, myId) : undefined}
        actions={actions}
        onClose={() => setMenuFor(null)}
      />
      <FlashList
        data={conversations}
        keyExtractor={(conversation) => conversation.id}
        renderItem={({ item }) => (
          <ConversationRow conversation={item} myId={myId} onLongPress={() => setMenuFor(item)} />
        )}
        onEndReached={() => {
          if (list.hasNextPage && !list.isFetchingNextPage) void list.fetchNextPage();
        }}
        onRefresh={() => void list.refetch()}
        refreshing={list.isRefetching && !list.isFetchingNextPage}
        ListEmptyComponent={
          <EmptyState
            emoji={filter === "inbox" ? "💬" : filter === "requests" ? "📮" : "🗃️"}
            title={t(`list.empty.${filter}.title`)}
            body={t(`list.empty.${filter}.body`)}
          />
        }
      />
    </>
  );
}
