import { community } from "@tokuly/api";
import { CATEGORIES, inviteCodeOf } from "@tokuly/core/community";
import { router } from "expo-router";
import { Plus, Search } from "lucide-react-native";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  FlatList,
  Pressable,
  RefreshControl,
  ScrollView,
  Text,
  TextInput,
  View,
} from "react-native";
import { Button } from "#/components/button";
import { EmptyState, ErrorNote, Spinner } from "#/components/page";
import { Tabs } from "#/components/tabs";
import { webScreenHref } from "#/features/web/handoff";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import {
  itemsOf,
  NS,
  useDirectory,
  useFriendsCommunities,
  useMyCommunities,
  useTrending,
} from "./data";
import { CommunityIcon, CommunityRow } from "./parts";

type Tab = "my" | "find";

/** Making a community is the web's (its form, pictures and rules); the rest is here. */
const CREATE_HREF = webScreenHref("/communities/new");

/** `/communities`: the caller's communities, and finding more. */
export function CommunitiesScreen() {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const [tab, setTab] = useState<Tab>("my");
  return (
    <View className="flex-1 bg-bg" testID="communities">
      <View className="flex-row items-center justify-between gap-2 px-4 pt-3 pb-2">
        <Tabs
          testID="communities-tab"
          items={[
            { value: "my", label: t("discover.my") },
            { value: "find", label: t("discover.find") },
          ]}
          value={tab}
          onChange={setTab}
        />
        <Button
          testID="community-create"
          size="sm"
          variant="secondary"
          icon={<Plus size={16} color={colors.ink} />}
          onPress={() => router.push(CREATE_HREF as never)}
        >
          {t("create.short")}
        </Button>
      </View>
      {tab === "my" ? <Mine onFind={() => setTab("find")} /> : <Find />}
    </View>
  );
}

/** Where an invite code (or a pasted invite link) is typed in. */
function InviteCodeBox() {
  const { t } = useTranslation(NS);
  const [text, setText] = useState("");
  const code = inviteCodeOf(text);
  return (
    <View className="mx-4 mb-4 gap-2 rounded-card bg-surface p-4">
      <Text className="font-black text-ink">{t("discover.enterCode")}</Text>
      <Text className="text-xs text-ink-soft">{t("discover.enterCodeBody")}</Text>
      <View className="flex-row items-center gap-2">
        <TextInput
          testID="invite-code-input"
          accessibilityLabel={t("join.codeLabel")}
          autoCapitalize="none"
          autoCorrect={false}
          value={text}
          onChangeText={setText}
          placeholder={t("join.codeLabel")}
          className="h-11 flex-1 rounded-control border-2 border-line bg-bg px-3 text-base text-ink"
        />
        <Button
          testID="invite-code-open"
          size="sm"
          disabled={code === ""}
          onPress={() => router.push(`/g/join/${encodeURIComponent(code)}`)}
        >
          {t("discover.openInvite")}
        </Button>
      </View>
    </View>
  );
}

function Mine({ onFind }: { onFind(): void }) {
  const { t } = useTranslation(NS);
  const mine = useMyCommunities();
  if (mine.isPending) return <Spinner />;
  if (mine.error && !mine.communities)
    return (
      <View className="px-4">
        <ErrorNote>{errorMessage(mine.error, { namespace: NS })}</ErrorNote>
      </View>
    );
  return (
    <FlatList
      testID="my-communities"
      data={mine.communities ?? []}
      keyExtractor={(c) => c.id}
      renderItem={({ item }) => (
        <CommunityRow community={item} testID={`community-${item.handle}`} />
      )}
      refreshControl={
        <RefreshControl refreshing={mine.isRefetching} onRefresh={() => void mine.refetch()} />
      }
      onEndReached={() => {
        if (mine.hasNextPage && !mine.isFetchingNextPage) void mine.fetchNextPage();
      }}
      ListHeaderComponent={
        mine.requested.length > 0 ? (
          <View className="pb-3">
            <Text className="px-4 pb-2 text-sm font-black text-ink-soft">
              {t("discover.requested")}
            </Text>
            {mine.requested.map((c) => (
              <CommunityRow key={c.id} community={c} />
            ))}
          </View>
        ) : undefined
      }
      ListEmptyComponent={
        <EmptyState
          emoji="🏕️"
          title={t("discover.myEmpty")}
          body={t("discover.myEmptyBody")}
          action={<Button onPress={onFind}>{t("discover.find")}</Button>}
        />
      }
      ListFooterComponent={
        <View className="pt-4 pb-12">
          <InviteCodeBox />
        </View>
      }
    />
  );
}

/** A row of communities that scrolls sideways; nothing when it is empty. */
function Shelf({
  title,
  communities,
  testID,
}: {
  title: string;
  communities: community.Community[];
  testID: string;
}) {
  if (communities.length === 0) return null;
  return (
    <View testID={testID} className="gap-2 pb-5">
      <Text className="px-4 text-lg font-black text-ink">{title}</Text>
      <ScrollView horizontal showsHorizontalScrollIndicator={false}>
        <View className="flex-row gap-3 px-4">
          {communities.map((c) => (
            <Pressable
              key={c.id}
              accessibilityRole="button"
              onPress={() => router.push(`/g/${c.handle}`)}
              className="w-32 items-center gap-2 rounded-card bg-surface p-3 active:bg-surface-2"
            >
              <CommunityIcon community={c} size="lg" />
              <Text numberOfLines={2} className="text-center text-sm font-black text-ink">
                {c.name}
              </Text>
            </Pressable>
          ))}
        </View>
      </ScrollView>
    </View>
  );
}

/** `value` once it stopped changing for `delayMs`: a search is not sent on every keystroke. */
function useSettled<T>(value: T, delayMs: number): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setSettled(value), delayMs);
    return () => clearTimeout(timer);
  }, [value, delayMs]);
  return settled;
}

function Find() {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState(community.CommunityCategory.UNSPECIFIED);
  const searched = useSettled(query.trim(), 300);
  const browsing = searched === "" && category === community.CommunityCategory.UNSPECIFIED;
  const directory = useDirectory({
    category,
    tag: "",
    query: searched,
    sort: community.DirectorySort.MEMBERS,
  });
  const trending = useTrending(community.CommunityCategory.UNSPECIFIED);
  const friends = useFriendsCommunities();
  const results = itemsOf(directory.data);
  return (
    <FlatList
      testID="community-directory"
      data={results}
      keyExtractor={(c) => c.id}
      keyboardShouldPersistTaps="handled"
      renderItem={({ item }) => (
        <CommunityRow community={item} testID={`community-${item.handle}`} />
      )}
      onEndReached={() => {
        if (directory.hasNextPage && !directory.isFetchingNextPage) void directory.fetchNextPage();
      }}
      ListHeaderComponent={
        <View className="gap-3 pb-3">
          <View className="mx-4 h-11 flex-row items-center gap-2 rounded-control border-2 border-line bg-surface px-3">
            <Search size={18} color={colors["ink-soft"]} />
            <TextInput
              testID="community-search"
              accessibilityLabel={t("discover.search")}
              placeholder={t("discover.search")}
              placeholderTextColor={colors["ink-faint"]}
              value={query}
              onChangeText={setQuery}
              returnKeyType="search"
              className="flex-1 text-base text-ink"
            />
          </View>
          <ScrollView horizontal showsHorizontalScrollIndicator={false}>
            <View className="flex-row gap-2 px-4">
              {CATEGORIES.map((each) => {
                const chosen = each.value === category;
                return (
                  <Pressable
                    key={each.key}
                    testID={`category-${each.key}`}
                    accessibilityRole="button"
                    accessibilityState={{ selected: chosen }}
                    onPress={() =>
                      setCategory(chosen ? community.CommunityCategory.UNSPECIFIED : each.value)
                    }
                    className={`rounded-full px-3 py-1.5 ${chosen ? "bg-brand" : "bg-surface"}`}
                  >
                    <Text className={`text-sm font-bold ${chosen ? "text-on-brand" : "text-ink"}`}>
                      {`${each.emoji} ${t(`category.${each.key}`)}`}
                    </Text>
                  </Pressable>
                );
              })}
            </View>
          </ScrollView>
          {browsing && (
            <View className="pt-2">
              <Shelf
                testID="trending-communities"
                title={t("discover.trending")}
                communities={trending.data ?? []}
              />
              <Shelf
                testID="friends-communities"
                title={t("discover.friends")}
                communities={itemsOf(friends.data).flatMap((each) =>
                  each.community ? [each.community] : [],
                )}
              />
            </View>
          )}
          <Text className="px-4 text-lg font-black text-ink">
            {browsing ? t("discover.all") : t("discover.results")}
          </Text>
        </View>
      }
      ListEmptyComponent={
        directory.isPending ? (
          <Spinner />
        ) : (
          <EmptyState emoji="🔭" title={t("discover.none")} body={t("discover.noneBody")} />
        )
      }
      ListFooterComponent={
        <View className="pb-12">{directory.isFetchingNextPage && <Spinner />}</View>
      }
    />
  );
}
