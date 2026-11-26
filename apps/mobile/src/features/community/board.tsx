import { community } from "@tokuly/api";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, ScrollView, Text, View } from "react-native";
import { EmptyState, Spinner } from "#/components/page";
import { Tabs } from "#/components/tabs";
import { NS, useChannels, useThreads } from "./data";
import { ThreadCard } from "./parts";

type Sort = "hot" | "new" | "top";

const SORTS: Record<Sort, community.ThreadSort> = {
  hot: community.ThreadSort.HOT,
  new: community.ThreadSort.NEW,
  top: community.ThreadSort.TOP,
};

/** The board's state above its list: which channel, which order. */
export function useBoard(communityId: string) {
  const [channelId, setChannelId] = useState("");
  const [sort, setSort] = useState<Sort>("hot");
  const threads = useThreads(communityId, {
    channelId,
    sort: SORTS[sort],
    window: community.TopWindow.ALL,
    tag: "",
  });
  return { channelId, setChannelId, sort, setSort, threads };
}

export type Board = ReturnType<typeof useBoard>;

/** Channel chips and the order of the board, over its threads. */
export function BoardControls({ communityId, board }: { communityId: string; board: Board }) {
  const { t } = useTranslation(NS);
  const channels = useChannels(communityId);
  const chip = (id: string, label: string, unread?: boolean) => {
    const chosen = board.channelId === id;
    return (
      <Pressable
        key={id || "all"}
        testID={`channel-${id || "all"}`}
        accessibilityRole="button"
        accessibilityState={{ selected: chosen }}
        onPress={() => board.setChannelId(id)}
        className={`flex-row items-center gap-1 rounded-full px-3 py-1.5 ${chosen ? "bg-brand" : "bg-surface"}`}
      >
        <Text className={`text-sm font-bold ${chosen ? "text-on-brand" : "text-ink"}`}>
          {label}
        </Text>
        {unread && <View className="size-2 rounded-full bg-pink" />}
      </Pressable>
    );
  };
  const current = channels.data?.find((channel) => channel.id === board.channelId);
  return (
    <View className="gap-3 pb-3">
      <ScrollView horizontal showsHorizontalScrollIndicator={false}>
        <View className="flex-row gap-2 px-4">
          {chip("", t("channel.all"))}
          {(channels.data ?? []).map((channel) =>
            chip(
              channel.id,
              `${channel.emoji ? `${channel.emoji} ` : ""}${channel.name}${channel.anonymous ? ` · ${t("anonymous.chip")}` : ""}`,
              channel.hasUnread,
            ),
          )}
        </View>
      </ScrollView>
      {current?.anonymous && (
        <Text className="mx-4 rounded-control bg-surface-2 px-3 py-2 text-xs text-ink-soft">
          {`🎭 ${t("anonymous.title")}`}
        </Text>
      )}
      <View className="px-4">
        <Tabs
          testID="board-sort"
          items={(["hot", "new", "top"] as const).map((key) => ({
            value: key,
            label: t(`sort.${key}`),
          }))}
          value={board.sort}
          onChange={board.setSort}
        />
      </View>
    </View>
  );
}

/** The threads of the board as rows of a list: the pinned ones first, once. */
export function boardRows(board: Board): community.Thread[] {
  const pinned = board.channelId === "" && board.sort === "hot" ? board.threads.pinned : [];
  const pinnedIds = new Set(pinned.map((thread) => thread.id));
  return [...pinned, ...(board.threads.threads ?? []).filter((each) => !pinnedIds.has(each.id))];
}

export function BoardEmpty({ board, member }: { board: Board; member: boolean }) {
  const { t } = useTranslation(NS);
  if (board.threads.isPending) return <Spinner />;
  return (
    <EmptyState
      emoji="🪧"
      title={t("thread.empty")}
      body={member ? t("thread.emptyBody") : t("thread.emptyJoin")}
    />
  );
}

export function renderThread(handle: string) {
  return ({ item }: { item: community.Thread }) => <ThreadCard thread={item} handle={handle} />;
}
