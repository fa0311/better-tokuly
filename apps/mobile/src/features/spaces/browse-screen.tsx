import { useQueryClient } from "@tanstack/react-query";
import { spaces } from "@tokuly/api";
import { forgetHangout, nameOf, rememberHangout, spacesKeys } from "@tokuly/core/spaces";
import { router } from "expo-router";
import { Plus } from "lucide-react-native";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, RefreshControl, ScrollView, Text, View } from "react-native";
import { Button } from "#/components/button";
import { CountBadge, EmptyState, Spinner } from "#/components/page";
import { Tabs } from "#/components/tabs";
import { webScreenHref } from "#/features/web/handoff";
import { errorMessage } from "#/lib/errors";
import { sticker, useTheme } from "#/lib/theme";
import { CreateSheet } from "./create-sheet";
import {
  hangoutClient,
  useHangouts,
  useLiveSpaces,
  useSpacesConfig,
  useUpcomingSpaces,
} from "./data";
import { HangoutCard, InsideBar, SpaceCard } from "./parts";

type Tab = "discover" | "hangouts";

/** A heading over a group of cards; nothing at all when the group is empty. */
function Shelf({
  title,
  children,
  testID,
}: {
  title: string;
  children: ReactNode[];
  testID: string;
}) {
  if (children.length === 0) return null;
  return (
    <View testID={testID} className="gap-3 pb-6">
      <Text className="text-lg font-black text-ink">{title}</Text>
      {children}
    </View>
  );
}

/** `/spaces`: who is talking now (friends first), what is coming, and the viewer's hangouts. */
export function BrowseSpacesScreen() {
  const { t } = useTranslation("spaces");
  const [tab, setTab] = useState<Tab>("discover");
  const invitations = useHangouts().data?.invitations.length ?? 0;
  return (
    <View className="flex-1 bg-bg" testID="spaces-browse">
      <View className="px-4 pt-3 pb-2">
        <Tabs
          testID="spaces-tab"
          items={[
            { value: "discover", label: t("tabs.discover") },
            { value: "hangouts", label: t("tabs.hangouts"), badge: invitations },
          ]}
          value={tab}
          onChange={setTab}
        />
      </View>
      <InsideBar />
      {tab === "discover" ? <Discover /> : <Hangouts />}
    </View>
  );
}

function Discover() {
  const { t } = useTranslation("spaces");
  const { t: tm } = useTranslation("mobile");
  const { colors } = useTheme();
  const [creating, setCreating] = useState(false);
  const config = useSpacesConfig();
  const friends = useLiveSpaces(spaces.LiveScope.FRIENDS);
  const trending = useLiveSpaces(spaces.LiveScope.PUBLIC);
  const upcoming = useUpcomingSpaces();
  const friendRooms = friends.data?.pages.flatMap((page) => page.spaces) ?? [];
  const shown = new Set(friendRooms.map((space) => space.id));
  // A friends' room that is also public is not listed twice.
  const publicRooms = (trending.data?.pages.flatMap((page) => page.spaces) ?? []).filter(
    (space) => !shown.has(space.id),
  );
  const planned = upcoming.data?.pages.flatMap((page) => page.spaces) ?? [];
  const audioOff = config.data?.audioEnabled === false;
  const loading = friends.isPending || trending.isPending;
  return (
    <>
      <ScrollView
        testID="spaces-discover"
        contentContainerClassName="px-4 pt-2 pb-12"
        refreshControl={
          <RefreshControl
            refreshing={friends.isRefetching || trending.isRefetching}
            onRefresh={() => {
              void friends.refetch();
              void trending.refetch();
              void upcoming.refetch();
            }}
          />
        }
      >
        <View className="flex-row flex-wrap gap-2 pb-5">
          <Button
            testID="spaces-create"
            icon={<Plus size={18} color={colors["on-brand"]} />}
            disabled={audioOff}
            onPress={() => setCreating(true)}
          >
            {t("create")}
          </Button>
          <Button
            testID="spaces-schedule"
            variant="secondary"
            onPress={() => router.push(webScreenHref("/spaces?tab=mine") as never)}
          >
            {tm("spaces.mineOnWeb")}
          </Button>
        </View>
        {audioOff && (
          <Text className="mb-5 rounded-card bg-sun/40 px-4 py-3 text-sm font-bold text-ink">
            {t("audioOff")}
          </Text>
        )}
        <Shelf title={t("discover.friends")} testID="spaces-friends">
          {friendRooms.map((space) => (
            <SpaceCard key={space.id} space={space} />
          ))}
        </Shelf>
        <Shelf title={t("discover.trending")} testID="spaces-trending">
          {publicRooms.map((space) => (
            <SpaceCard key={space.id} space={space} />
          ))}
        </Shelf>
        {trending.hasNextPage && (
          <View className="items-center pb-6">
            <Button
              variant="ghost"
              loading={trending.isFetchingNextPage}
              onPress={() => void trending.fetchNextPage()}
            >
              {tm("spaces.more")}
            </Button>
          </View>
        )}
        {loading ? (
          <Spinner />
        ) : friendRooms.length + publicRooms.length === 0 ? (
          <EmptyState emoji="🎙️" title={t("discover.empty")} body={t("discover.emptyBody")} />
        ) : null}
        <Shelf title={t("discover.upcoming")} testID="spaces-upcoming">
          {planned.map((space) => (
            <SpaceCard key={space.id} space={space} />
          ))}
        </Shelf>
      </ScrollView>
      <CreateSheet visible={creating} onClose={() => setCreating(false)} />
    </>
  );
}

function Hangouts() {
  const { t } = useTranslation("spaces");
  const { t: tm } = useTranslation("mobile");
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const board = useHangouts();
  const [answering, setAnswering] = useState("");
  const respond = async (hangout: spaces.Hangout, accept: boolean) => {
    setAnswering(hangout.id);
    try {
      const response = await hangoutClient().respondToHangoutInvite({
        hangoutId: hangout.id,
        accept,
      });
      if (accept) {
        rememberHangout(queryClient, response.hangout);
        Alert.alert(t("toast.hangoutJoined"));
      } else forgetHangout(queryClient, hangout.id);
      void queryClient.invalidateQueries({ queryKey: spacesKeys.hangouts });
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: "spaces" }));
    } finally {
      setAnswering("");
    }
  };
  const invitations = board.data?.invitations ?? [];
  const hangouts = board.data?.hangouts ?? [];
  return (
    <ScrollView
      testID="spaces-hangouts"
      contentContainerClassName="gap-3 px-4 pt-2 pb-12"
      refreshControl={
        <RefreshControl refreshing={board.isRefetching} onRefresh={() => void board.refetch()} />
      }
    >
      <Button
        testID="hangouts-create"
        variant="secondary"
        icon={<Plus size={18} color={colors.ink} />}
        onPress={() => router.push(webScreenHref("/spaces?tab=hangouts") as never)}
      >
        {tm("spaces.createHangoutOnWeb")}
      </Button>
      {invitations.length > 0 && (
        <View className="flex-row items-center gap-2 pt-3">
          <Text className="text-lg font-black text-ink">{t("hangouts.invitations")}</Text>
          <CountBadge count={invitations.length} />
        </View>
      )}
      {invitations.map((hangout) => (
        <View
          key={hangout.id}
          testID="hangout-invitation"
          className="gap-3 rounded-card bg-surface p-4"
          style={sticker(colors)}
        >
          <Text className="text-base font-black text-ink">
            {`${hangout.emoji} ${hangout.name}`.trim()}
          </Text>
          <Text className="text-xs font-bold text-ink-soft">
            {t("hangouts.invitedBy", {
              name: nameOf(hangout.owner),
              count: hangout.memberCount,
            })}
          </Text>
          <View className="flex-row gap-2">
            <Button
              testID="hangout-accept"
              size="sm"
              loading={answering === hangout.id}
              onPress={() => void respond(hangout, true)}
            >
              {t("hangouts.accept")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              disabled={answering === hangout.id}
              onPress={() => void respond(hangout, false)}
            >
              {t("hangouts.decline")}
            </Button>
          </View>
        </View>
      ))}
      {hangouts.length > 0 && invitations.length > 0 && <View className="h-3" />}
      {hangouts.map((hangout) => (
        <HangoutCard key={hangout.id} hangout={hangout} />
      ))}
      {board.isPending ? (
        <Spinner />
      ) : hangouts.length + invitations.length === 0 ? (
        <EmptyState emoji="🛋️" title={t("hangouts.empty")} body={t("hangouts.emptyBody")} />
      ) : null}
    </ScrollView>
  );
}
