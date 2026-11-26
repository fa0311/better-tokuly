import { FlashList } from "@shopify/flash-list";
import { useQueryClient } from "@tanstack/react-query";
import { type notify, toMillis } from "@tokuly/api";
import {
  type DayBucket,
  dayBucket,
  entriesOf,
  formatWhen,
  type InboxFilter,
} from "@tokuly/core/notifications";
import { router } from "expo-router";
import * as WebBrowser from "expo-web-browser";
import { UserRound } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { EmptyState, Spinner } from "#/components/page";
import { Tabs } from "#/components/tabs";
import { errorMessage } from "#/lib/errors";
import i18n from "#/lib/i18n";
import { useTheme } from "#/lib/theme";
import { markRead, useInbox, useUnread } from "./data";
import { PushCard } from "./push-card";
import { targetOf } from "./target";

const FILTERS = {
  all: { category: "", unreadOnly: false },
  unread: { category: "", unreadOnly: true },
} satisfies Record<string, InboxFilter>;
type Tab = keyof typeof FILTERS;

type Item = { type: "section"; bucket: DayBucket } | { type: "entry"; entry: notify.Notification };

/** The entries under their day headings (today, yesterday, this week, earlier). */
function itemsOf(entries: notify.Notification[], now: number): Item[] {
  const items: Item[] = [];
  let bucket: DayBucket | null = null;
  for (const entry of entries) {
    const next = dayBucket(toMillis(entry.occurredAt), now);
    if (next !== bucket) {
      bucket = next;
      items.push({ type: "section", bucket: next });
    }
    items.push({ type: "entry", entry });
  }
  return items;
}

function Face({ entry }: { entry: notify.Notification }) {
  const { colors } = useTheme();
  if (entry.actor) return <Avatar user={entry.actor} size="md" />;
  return (
    <View className="size-11 items-center justify-center rounded-xl bg-surface-3">
      {entry.actorUnnamed ? (
        <UserRound size={22} color={colors["ink-soft"]} />
      ) : (
        <Text className="text-xl">🔔</Text>
      )}
    </View>
  );
}

/** Tapping an entry: it becomes read, and its screen opens (or the web, for what the app has not yet). */
function open(queryClient: ReturnType<typeof useQueryClient>, entry: notify.Notification) {
  if (!entry.read) void markRead(queryClient, { ids: [entry.id] }).catch(() => {});
  const target = targetOf(entry.path);
  if (target?.type === "screen") router.push(target.href as never);
  else if (target?.type === "web") void WebBrowser.openBrowserAsync(target.url);
}

/** The inbox: what happened, grouped by day, newest first; the push switch on top. */
export function InboxScreen() {
  const { t } = useTranslation("notifications");
  const queryClient = useQueryClient();
  const [tab, setTab] = useState<Tab>("all");
  const inbox = useInbox(FILTERS[tab]);
  const unread = useUnread();
  const entries = entriesOf(inbox.data);
  const now = Date.now();

  const readAll = () => {
    const newest = entries[0];
    if (!newest) return;
    markRead(queryClient, { upToId: newest.id }).catch((error) =>
      Alert.alert(errorMessage(error, { namespace: "notifications" })),
    );
  };

  return (
    <View className="flex-1 bg-bg" testID="inbox-screen">
      <View className="flex-row items-center justify-between gap-2 px-4 pt-3 pb-2">
        <Tabs
          testID="inbox-tab"
          items={(Object.keys(FILTERS) as Tab[]).map((key) => ({
            value: key,
            label: t(`tabs.${key}`),
            badge: key === "unread" ? unread.total : 0,
          }))}
          value={tab}
          onChange={setTab}
        />
        {unread.total > 0 && (
          <Button testID="inbox-read-all" size="sm" variant="secondary" onPress={readAll}>
            {t("actions.markAllRead")}
          </Button>
        )}
      </View>
      {inbox.isPending ? (
        <Spinner />
      ) : (
        <FlashList
          data={itemsOf(entries, now)}
          keyExtractor={(item) => (item.type === "section" ? `s:${item.bucket}` : item.entry.id)}
          getItemType={(item) => item.type}
          ListHeaderComponent={
            <View className="px-4 pb-2">
              <PushCard compact />
            </View>
          }
          renderItem={({ item }) =>
            item.type === "section" ? (
              <Text className="px-5 pt-4 pb-1 text-xs font-black tracking-wider text-ink-soft">
                {t(`sections.${item.bucket}`)}
              </Text>
            ) : (
              <Pressable
                testID={`inbox-entry-${item.entry.id}`}
                accessibilityRole="button"
                onPress={() => open(queryClient, item.entry)}
                className={`flex-row gap-3 border-b border-line px-4 py-3 ${
                  item.entry.read ? "bg-surface" : "bg-brand-soft"
                }`}
              >
                <Face entry={item.entry} />
                <View className="min-w-0 flex-1 gap-0.5">
                  <View className="flex-row items-center gap-1.5">
                    {!item.entry.read && (
                      <View
                        accessibilityLabel={t("row.unread")}
                        className="h-2.5 w-2.5 rounded-full bg-pink"
                      />
                    )}
                    <Text
                      numberOfLines={1}
                      className={`shrink text-ink ${item.entry.read ? "font-bold" : "font-black"}`}
                    >
                      {item.entry.title}
                    </Text>
                  </View>
                  {item.entry.body ? (
                    <Text numberOfLines={2} className="text-sm text-ink-soft">
                      {item.entry.body}
                    </Text>
                  ) : null}
                  <Text className="text-xs font-bold text-ink-faint">
                    {formatWhen(i18n, toMillis(item.entry.occurredAt), now)}
                    {item.entry.groupCount > 1
                      ? ` · ${t("row.grouped", { count: item.entry.groupCount - 1 })}`
                      : ""}
                  </Text>
                </View>
              </Pressable>
            )
          }
          onEndReached={() => {
            if (inbox.hasNextPage && !inbox.isFetchingNextPage) void inbox.fetchNextPage();
          }}
          onRefresh={() => void inbox.refetch()}
          refreshing={inbox.isRefetching && !inbox.isFetchingNextPage}
          ListEmptyComponent={
            tab === "unread" ? (
              <EmptyState emoji="✨" title={t("empty.unreadTitle")} body={t("empty.unreadBody")} />
            ) : (
              <EmptyState emoji="🔔" title={t("empty.title")} body={t("empty.body")} />
            )
          }
        />
      )}
    </View>
  );
}
