import { useQueryClient } from "@tanstack/react-query";
import { type community, errorReason, toMillis } from "@tokuly/api";
import {
  canRead,
  categoryOf,
  communityKeys,
  imageUrl,
  isMember,
  isStaff,
  rereadCommunity,
  timeoutUntilMs,
  VISIBILITY_KEYS,
} from "@tokuly/core/community";
import { Image } from "expo-image";
import { router, Stack } from "expo-router";
import { CalendarDays, PenLine, Settings2, UsersRound } from "lucide-react-native";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { FlatList, RefreshControl, ScrollView, Text, View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState, Row, Section, Spinner } from "#/components/page";
import { Tabs } from "#/components/tabs";
import { useMe } from "#/features/session/store";
import { webScreenHref } from "#/features/web/handoff";
import { mediaUrl } from "#/lib/config";
import { errorMessage } from "#/lib/errors";
import { realtime } from "#/lib/realtime";
import { useTheme } from "#/lib/theme";
import { BoardControls, BoardEmpty, boardRows, renderThread, useBoard } from "./board";
import { itemsOf, NS, useAnnouncements, useChannels, useCommunity } from "./data";
import { MembershipButton } from "./membership";
import { CommunityIcon, useMemberCount } from "./parts";

type Tab = "boards" | "news" | "about";

/**
 * Follows the community's topic while its screen is open: board and
 * calendar events arrive. In effect (first, or again after a gap) its board
 * is read again; removed, banned or closed, the community itself is.
 */
function useCommunityTopic(c: community.Community | null | undefined, readable: boolean) {
  const queryClient = useQueryClient();
  const communityId = c?.id;
  const handle = c?.handle ?? "";
  useEffect(() => {
    if (!communityId || !readable) return;
    const { release } = realtime().subscribe(`community:${communityId}`, {
      onSubscribed: () => void rereadCommunity(queryClient, communityId),
      onRevoked: () =>
        void queryClient.invalidateQueries({ queryKey: communityKeys.community(handle) }),
    });
    realtime().start();
    return release;
  }, [communityId, handle, readable, queryClient]);
}

/** `/g/<handle>`: a community — its front, its board, its announcements and what it is about. */
export function CommunityScreen({ handle }: { handle: string }) {
  const { t } = useTranslation(NS);
  const query = useCommunity(handle);
  const c = query.data;
  const readable = canRead(c ?? undefined, useMe()?.roles);
  useCommunityTopic(c, readable);

  if (query.isPending) return <Spinner />;
  if (!c) {
    const missing = !query.error || errorReason(query.error) === "COMMUNITY_NOT_FOUND";
    return (
      <View className="flex-1 bg-bg" testID="community-not-found">
        <EmptyState
          emoji={missing ? "🫥" : "🌧️"}
          title={missing ? t("notFound.title") : errorMessage(query.error, { namespace: NS })}
          body={missing ? t("notFound.body") : undefined}
          action={
            missing ? undefined : (
              <Button variant="secondary" onPress={() => void query.refetch()}>
                {t("retry")}
              </Button>
            )
          }
        />
      </View>
    );
  }
  return (
    <>
      <Stack.Screen options={{ title: c.name }} />
      {readable ? <Inside community={c} /> : <FrontDoor community={c} />}
    </>
  );
}

/** Banner, icon, name, what it is and the caller's membership. */
function Header({ community: c }: { community: community.Community }) {
  const { t, i18n } = useTranslation(NS);
  const members = useMemberCount();
  const banner = mediaUrl(imageUrl(c.banner, 400));
  const category = categoryOf(c.category);
  const timedOut = timeoutUntilMs(c.viewer, Date.now());
  return (
    <View className="gap-3 pb-4">
      {banner ? (
        <Image source={banner} style={{ width: "100%", aspectRatio: 3 }} contentFit="cover" />
      ) : (
        <View className="h-16 bg-surface-2" />
      )}
      <View className="-mt-10 gap-3 px-4">
        <CommunityIcon community={c} size="lg" />
        <View className="gap-1">
          <Text testID="community-name" className="text-2xl font-black text-ink">
            {c.name}
          </Text>
          <Text className="text-sm text-ink-soft">
            {`${category.emoji} ${t(`category.${category.key}`)} · ${members(c.memberCount)} · ${t(`visibility.${VISIBILITY_KEYS[c.visibility]}`)}`}
          </Text>
        </View>
        {c.description !== "" && (
          <Text numberOfLines={4} className="text-sm leading-5 text-ink">
            {c.description}
          </Text>
        )}
        <MembershipButton community={c} />
        {timedOut > 0 && (
          <Text className="rounded-control bg-sun/30 px-3 py-2 text-xs font-bold text-ink">
            {t("timeout.you", {
              time: new Date(timedOut).toLocaleString(i18n.language, {
                month: "numeric",
                day: "numeric",
                hour: "2-digit",
                minute: "2-digit",
              }),
            })}
          </Text>
        )}
      </View>
    </View>
  );
}

/** The ways out of the community's screen: its members, its events, the staff's pages. */
function Links({ community: c }: { community: community.Community }) {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const waiting = (c.viewer?.pendingRequestCount ?? 0) + (c.viewer?.reviewQueueCount ?? 0);
  return (
    <View className="px-4 pb-4">
      <Section>
        <Row
          testID="community-members"
          icon={<UsersRound size={20} color={colors.ink} />}
          label={t("tabs.members")}
          onPress={() => router.push(`/g/${c.handle}/members`)}
        />
        <Row
          testID="community-events"
          icon={<CalendarDays size={20} color={colors.ink} />}
          label={t("tabs.events")}
          onPress={() => router.push(webScreenHref(`/g/${c.handle}/events`) as never)}
        />
        {isStaff(c) && (
          <Row
            testID="community-manage"
            icon={<Settings2 size={20} color={colors.ink} />}
            label={t("manage.title")}
            badge={waiting}
            onPress={() => router.push(webScreenHref(`/g/${c.handle}/manage`) as never)}
          />
        )}
      </Section>
    </View>
  );
}

function Inside({ community: c }: { community: community.Community }) {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const [tab, setTab] = useState<Tab>("boards");
  const board = useBoard(c.id);
  const news = useAnnouncements(c.id, tab === "news");
  const channels = useChannels(c.id);
  // `can_post` is the server's word on the channel, the caller's role and a timeout together.
  const mayPost = isMember(c) && (channels.data ?? []).some((channel) => channel.canPost);
  const rows = tab === "boards" ? boardRows(board) : tab === "news" ? itemsOf(news.data) : [];
  const list = tab === "boards" ? board.threads : news;

  return (
    <View className="flex-1 bg-bg" testID="community-screen">
      <FlatList
        data={rows}
        keyExtractor={(thread) => thread.id}
        renderItem={renderThread(c.handle)}
        refreshControl={
          <RefreshControl refreshing={list.isRefetching} onRefresh={() => void list.refetch()} />
        }
        onEndReached={() => {
          if (tab !== "about" && list.hasNextPage && !list.isFetchingNextPage)
            void list.fetchNextPage();
        }}
        ListHeaderComponent={
          <View>
            <Header community={c} />
            <Links community={c} />
            <View className="px-4 pb-3">
              <Tabs
                testID="community-tab"
                items={(["boards", "news", "about"] as const).map((key) => ({
                  value: key,
                  label: t(`tabs.${key}`),
                }))}
                value={tab}
                onChange={setTab}
              />
            </View>
            {tab === "boards" && <BoardControls communityId={c.id} board={board} />}
            {tab === "about" && <About community={c} />}
          </View>
        }
        ListEmptyComponent={
          tab === "boards" ? (
            <BoardEmpty board={board} member={isMember(c)} />
          ) : tab === "news" ? (
            news.isPending ? (
              <Spinner />
            ) : (
              <EmptyState emoji="📣" title={t("news.empty")} body={t("news.emptyBody")} />
            )
          ) : undefined
        }
        ListFooterComponent={<View className="h-24">{list.isFetchingNextPage && <Spinner />}</View>}
      />
      {mayPost && tab === "boards" && (
        <View className="absolute right-4 bottom-6">
          <Button
            testID="thread-new"
            size="lg"
            icon={<PenLine size={20} color={colors["on-brand"]} />}
            onPress={() =>
              router.push(
                `/g/${c.handle}/new${board.channelId ? `?channel=${board.channelId}` : ""}`,
              )
            }
          >
            {t("compose.new")}
          </Button>
        </View>
      )}
    </View>
  );
}

/** Its rules and facts. */
function About({ community: c }: { community: community.Community }) {
  const { t, i18n } = useTranslation(NS);
  const day = (at: community.Community["createdAt"]) =>
    at ? new Date(toMillis(at)).toLocaleDateString(i18n.language) : "—";
  return (
    <View testID="community-about" className="gap-4 px-4 pb-8">
      {c.rules.length > 0 && (
        <View className="gap-2 rounded-card bg-surface p-4">
          <Text className="font-black text-ink">{t("about.rules")}</Text>
          {c.rules.map((rule, index) => (
            <View key={`${index}-${rule.title}`} className="gap-0.5">
              <Text className="text-sm font-bold text-ink">{`${index + 1}. ${rule.title}`}</Text>
              {rule.body !== "" && <Text className="text-sm text-ink-soft">{rule.body}</Text>}
            </View>
          ))}
        </View>
      )}
      <View className="gap-2 rounded-card bg-surface p-4">
        <Fact
          label={t("about.visibility")}
          value={t(`visibility.${VISIBILITY_KEYS[c.visibility]}Hint`)}
        />
        <Fact
          label={t("about.members")}
          value={t("about.memberOf", { count: c.memberCount, max: c.maxMembers })}
        />
        <Fact label={t("about.threads")} value={String(c.threadCount)} />
        <Fact label={t("about.created")} value={day(c.createdAt)} />
        <Fact label={t("about.active")} value={day(c.lastActivityAt)} />
      </View>
    </View>
  );
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <View className="flex-row gap-3">
      <Text className="w-24 text-sm font-bold text-ink-soft">{label}</Text>
      <Text className="flex-1 text-sm text-ink">{value}</Text>
    </View>
  );
}

/** A community only its members read: what it is, its rules, and the way in. */
function FrontDoor({ community: c }: { community: community.Community }) {
  const { t } = useTranslation(NS);
  return (
    <ScrollView testID="community-front-door" className="flex-1 bg-bg">
      <Header community={c} />
      <Text className="mx-4 mb-4 rounded-control bg-surface-2 px-4 py-3 text-sm text-ink">
        {`🔒 ${t("frontDoor.membersOnly")}`}
      </Text>
      {c.joinQuestions.length > 0 && (
        <View className="mx-4 mb-4 gap-1 rounded-card bg-surface p-4">
          <Text className="font-black text-ink">{t("frontDoor.questions")}</Text>
          {c.joinQuestions.map((question) => (
            <Text key={question} className="text-sm text-ink-soft">{`・${question}`}</Text>
          ))}
        </View>
      )}
      <About community={c} />
    </ScrollView>
  );
}
