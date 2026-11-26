import { community, toMillis } from "@tokuly/api";
import { router } from "expo-router";
import { Search } from "lucide-react-native";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { FlatList, Pressable, Text, TextInput, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { EmptyState, Spinner } from "#/components/page";
import { Tabs } from "#/components/tabs";
import { useTheme } from "#/lib/theme";
import { itemsOf, NS, useCommunity, useMembers } from "./data";
import { RoleChip } from "./parts";

const R = community.MemberRole;

type Filter = "everyone" | "admin" | "moderator";

/** The roles the list can be narrowed to (the server filters by one role). */
const FILTERS: Record<Filter, community.MemberRole> = {
  everyone: R.UNSPECIFIED,
  admin: R.ADMIN,
  moderator: R.MODERATOR,
};

/**
 * `/g/<handle>/members`: who is in, with the roles the server names. Giving
 * roles, timeouts and bans stay on the web's member list for the staff.
 */
export function MembersScreen({ handle }: { handle: string }) {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const c = useCommunity(handle).data;
  const [typed, setTyped] = useState("");
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("everyone");
  useEffect(() => {
    const timer = setTimeout(() => setQuery(typed.trim()), 300);
    return () => clearTimeout(timer);
  }, [typed]);
  const list = useMembers(c?.id ?? "", query, FILTERS[filter]);
  const members = itemsOf(list.data);

  if (!c) return <Spinner />;
  return (
    <View className="flex-1 bg-bg" testID="community-members-screen">
      <FlatList
        data={members}
        keyExtractor={(member) => member.user?.id ?? ""}
        keyboardShouldPersistTaps="handled"
        renderItem={({ item }) => <MemberRow member={item} />}
        onEndReached={() => {
          if (list.hasNextPage && !list.isFetchingNextPage) void list.fetchNextPage();
        }}
        ListHeaderComponent={
          <View className="gap-3 px-4 pt-3 pb-3">
            <View className="h-11 flex-row items-center gap-2 rounded-control border-2 border-line bg-surface px-3">
              <Search size={18} color={colors["ink-soft"]} />
              <TextInput
                testID="member-search"
                accessibilityLabel={t("members.search")}
                placeholder={t("members.search")}
                placeholderTextColor={colors["ink-faint"]}
                autoCapitalize="none"
                value={typed}
                onChangeText={setTyped}
                className="flex-1 text-base text-ink"
              />
            </View>
            <Tabs
              testID="member-filter"
              items={[
                { value: "everyone", label: t("members.everyone") },
                { value: "admin", label: t("role.admin") },
                { value: "moderator", label: t("role.moderator") },
              ]}
              value={filter}
              onChange={setFilter}
            />
          </View>
        }
        ListEmptyComponent={
          list.isPending ? <Spinner /> : <EmptyState emoji="🔍" title={t("members.none")} />
        }
        ListFooterComponent={<View className="h-12">{list.isFetchingNextPage && <Spinner />}</View>}
      />
    </View>
  );
}

function MemberRow({ member }: { member: community.Member }) {
  const { t, i18n } = useTranslation(NS);
  const user = member.user;
  const timedOut = member.timeoutUntil && toMillis(member.timeoutUntil) > Date.now();
  return (
    <Pressable
      testID={`member-${user?.handle ?? ""}`}
      accessibilityRole="button"
      disabled={!user?.handle || user.deleted}
      onPress={() => user && router.push(`/u/${user.handle}`)}
      className="flex-row items-center gap-3 border-b border-line bg-surface px-4 py-3 active:bg-surface-2"
    >
      <Avatar user={user} />
      <View className="flex-1 gap-0.5">
        <Text numberOfLines={1} className="text-base font-black text-ink">
          {user?.deleted ? t("post.deletedUser") : user?.displayName || user?.handle}
        </Text>
        {user?.handle && !user.deleted && (
          <Text numberOfLines={1} className="text-xs text-ink-soft">{`@${user.handle}`}</Text>
        )}
        {timedOut && member.timeoutUntil && (
          <Text className="text-xs font-bold text-danger">
            {`${t("members.timedOut")} · ${t("members.timedOutUntil", {
              time: new Date(toMillis(member.timeoutUntil)).toLocaleString(i18n.language),
            })}`}
          </Text>
        )}
      </View>
      <RoleChip role={member.role} />
    </Pressable>
  );
}
