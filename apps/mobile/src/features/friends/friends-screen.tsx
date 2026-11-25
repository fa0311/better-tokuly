import { FlashList } from "@shopify/flash-list";
import { Link } from "expo-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, TextInput, View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState, Spinner } from "#/components/page";
import { useTheme } from "#/lib/theme";
import { useFriends, useIncomingRequests, useOutgoingRequests } from "./data";
import { PersonRow } from "./friend-row";
import { RequestActions } from "./requests";

type Tab = "friends" | "requests";

/** Friends and the requests waiting on either side, with a way to add more. */
export function FriendsScreen() {
  const { t } = useTranslation("friends");
  const [tab, setTab] = useState<Tab>("friends");
  const incoming = useIncomingRequests();
  const waiting = incoming.data?.requests.length ?? 0;
  return (
    <View className="flex-1 bg-bg" testID="friends-screen">
      <View className="flex-row gap-2 px-4 pt-3 pb-2">
        {(["friends", "requests"] as const).map((key) => (
          <Pressable
            key={key}
            testID={`friends-tab-${key}`}
            accessibilityRole="tab"
            accessibilityState={{ selected: tab === key }}
            onPress={() => setTab(key)}
            className={`rounded-full px-4 py-2 ${tab === key ? "bg-ink" : "bg-surface"}`}
          >
            <Text className={`font-black ${tab === key ? "text-surface" : "text-ink"}`}>
              {t(`tabs.${key}`)}
              {key === "requests" && waiting > 0 ? ` ${waiting}` : ""}
            </Text>
          </Pressable>
        ))}
        <View className="ml-auto">
          <Link href="/add-friend" asChild>
            <Button testID="friends-add" size="sm">
              {t("actions.add")}
            </Button>
          </Link>
        </View>
      </View>
      {tab === "friends" ? <FriendList /> : <RequestLists />}
    </View>
  );
}

function FriendList() {
  const { t } = useTranslation("friends");
  const { colors } = useTheme();
  const [query, setQuery] = useState("");
  const { data, isPending } = useFriends(query.trim());
  const friends = data?.friends ?? [];
  return (
    <View className="flex-1">
      <TextInput
        accessibilityLabel={t("search.placeholder")}
        placeholder={t("search.placeholder")}
        placeholderTextColor={colors["ink-faint"]}
        value={query}
        onChangeText={setQuery}
        autoCapitalize="none"
        className="mx-4 mb-2 h-11 rounded-full bg-surface px-4 text-ink"
      />
      {isPending ? (
        <Spinner />
      ) : (
        <FlashList
          data={friends}
          keyExtractor={(friend) => friend.user?.id ?? ""}
          renderItem={({ item }) => (
            <PersonRow user={item.user} online={item.online} detail={item.nickname || undefined} />
          )}
          ListEmptyComponent={
            query ? (
              <EmptyState emoji="🔍" title={t("search.empty")} />
            ) : (
              <EmptyState emoji="👋" title={t("list.empty")} body={t("list.emptyBody")} />
            )
          }
        />
      )}
    </View>
  );
}

function RequestLists() {
  const { t } = useTranslation("friends");
  const incoming = useIncomingRequests();
  const outgoing = useOutgoingRequests();
  if (incoming.isPending || outgoing.isPending) return <Spinner />;
  const rows = [
    { kind: "title" as const, id: "in", title: t("requests.incoming") },
    ...(incoming.data?.requests ?? []).map((request) => ({
      kind: "incoming" as const,
      id: request.id,
      request,
    })),
    { kind: "title" as const, id: "out", title: t("requests.outgoing") },
    ...(outgoing.data?.requests ?? []).map((request) => ({
      kind: "outgoing" as const,
      id: request.id,
      request,
    })),
  ];
  return (
    <FlashList
      data={rows}
      keyExtractor={(row) => `${row.kind}-${row.id}`}
      getItemType={(row) => row.kind}
      renderItem={({ item }) => {
        if (item.kind === "title")
          return (
            <View className="px-4 pt-4 pb-1">
              <Text className="text-xs font-black text-ink-soft">{item.title}</Text>
              {item.id === "in" && !incoming.data?.requests.length && (
                <Text className="py-3 text-sm text-ink-soft">{t("requests.emptyIncoming")}</Text>
              )}
              {item.id === "out" && !outgoing.data?.requests.length && (
                <Text className="py-3 text-sm text-ink-soft">{t("requests.emptyOutgoing")}</Text>
              )}
            </View>
          );
        const user = item.kind === "incoming" ? item.request.from : item.request.to;
        return (
          <PersonRow
            user={user}
            detail={
              item.request.mutualFriendCount > 0
                ? t("requests.mutual", { count: item.request.mutualFriendCount })
                : undefined
            }
          >
            <RequestActions request={item.request} incoming={item.kind === "incoming"} />
          </PersonRow>
        );
      }}
    />
  );
}
