import { FlashList } from "@shopify/flash-list";
import { type moments, toMillis } from "@tokuly/api";
import { latenessText } from "@tokuly/core/moments";
import { Image } from "expo-image";
import { router } from "expo-router";
import { ChevronRight, LockKeyhole } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { EmptyState, ErrorNote, Spinner } from "#/components/page";
import { mediaUrl } from "#/lib/config";
import { useTheme } from "#/lib/theme";
import { useFriendMoments, useMomentToday } from "./data";

export function MomentFriendsScreen() {
  const { t } = useTranslation("moments");
  const friends = useFriendMoments();
  const today = useMomentToday();
  const mine = today.data?.mine;
  const friendItems = friends.data?.pages.flatMap((page) => page.moments) ?? [];
  const items = [
    ...(mine ? [mine] : []),
    ...friendItems.filter((moment) => moment.id !== mine?.id),
  ];
  const listUnlocked = friends.data?.pages[0]?.unlocked ?? false;
  const unlocked = today.data?.unlocked ?? listUnlocked;
  const relocking = listUnlocked && today.data?.unlocked !== true;
  const shouldShowError =
    (friends.isError && (!friends.data || relocking)) || (relocking && today.isError);

  if (shouldShowError) {
    return (
      <View className="flex-1 gap-4 bg-bg px-4 pt-4" testID="moment-friends">
        <ErrorNote>{t("friends.loadError")}</ErrorNote>
        <Button
          variant="secondary"
          onPress={() => {
            void friends.refetch();
            void today.refetch();
          }}
        >
          {t("composer.retry")}
        </Button>
      </View>
    );
  }
  if ((friends.isPending && !friends.data) || relocking) {
    return (
      <View className="flex-1 bg-bg" testID="moment-friends">
        <Spinner />
      </View>
    );
  }
  return (
    <View className="flex-1 bg-bg" testID="moment-friends">
      <FlashList
        data={items}
        keyExtractor={(moment) => moment.id}
        renderItem={({ item }) => (
          <MomentCard
            moment={item}
            onPress={() =>
              router.push({
                pathname: "/moments/friends/[momentId]",
                params: { momentId: item.id },
              })
            }
          />
        )}
        ListHeaderComponent={
          <View className="gap-2 px-4 pt-4 pb-1">
            <Text className="text-sm text-ink-soft">
              {unlocked ? t("camera.posted") : t("locked.title")}
            </Text>
            {!unlocked && (
              <Button variant="secondary" onPress={() => router.push("/moments")}>
                {t("locked.action")}
              </Button>
            )}
          </View>
        }
        ListEmptyComponent={
          <EmptyState
            emoji="🌤️"
            title={t("today.friendsEmpty")}
            body={t("today.friendsEmptyBody")}
          />
        }
        ListFooterComponent={
          friends.isFetchingNextPage ? (
            <Spinner />
          ) : friends.isFetchNextPageError ? (
            <View className="p-4">
              <Button size="sm" variant="secondary" onPress={() => void friends.fetchNextPage()}>
                {t("composer.retry")}
              </Button>
            </View>
          ) : mine && friendItems.length === 0 ? (
            <EmptyState
              emoji="🌤️"
              title={t("today.friendsEmpty")}
              body={t("today.friendsEmptyBody")}
            />
          ) : null
        }
        onEndReached={() => {
          if (friends.hasNextPage && !friends.isFetchingNextPage) void friends.fetchNextPage();
        }}
        onRefresh={() => void friends.refetch()}
        refreshing={friends.isRefetching && !friends.isFetchingNextPage}
        contentContainerClassName="gap-3 px-4 pb-10"
      />
    </View>
  );
}

function MomentCard({ moment, onPress }: { moment: moments.Moment; onPress(): void }) {
  const { t, i18n } = useTranslation("moments");
  const { colors } = useTheme();
  const author = moment.author;
  const posted = moment.postedAt
    ? new Intl.DateTimeFormat(i18n.language, { dateStyle: "medium", timeStyle: "short" }).format(
        new Date(toMillis(moment.postedAt)),
      )
    : undefined;
  const lateSeconds = Number(moment.lateSeconds);

  return (
    <Pressable
      testID={`moment-friend-${moment.id}`}
      accessibilityRole="button"
      accessibilityLabel={
        moment.locked
          ? t("friends.lockedCard", { name: author?.displayName ?? author?.handle ?? "" })
          : moment.mine
            ? t("friends.ownCard")
            : t("friends.openCard", { name: author?.displayName ?? author?.handle ?? "" })
      }
      disabled={moment.locked}
      onPress={onPress}
      className="gap-3 rounded-card bg-surface p-4 active:bg-surface-2"
    >
      <View className="flex-row items-center gap-3">
        <Avatar user={author} />
        <View className="min-w-0 flex-1 gap-0.5">
          <Text numberOfLines={1} className="font-black text-ink">
            {author?.displayName || author?.handle || t("friends.someone")}
          </Text>
          <View className="flex-row items-center gap-2">
            {posted && !moment.locked && (
              <Text className="text-xs text-ink-faint">
                {posted}
                {lateSeconds > 0 ? ` · ${latenessText(i18n, moment.lateSeconds)}` : ""}
              </Text>
            )}
            {moment.mine && (
              <Text className="text-xs font-bold text-brand">{t("friends.yours")}</Text>
            )}
          </View>
        </View>
        {moment.locked ? (
          <LockKeyhole size={18} color={colors["ink-faint"]} />
        ) : (
          <ChevronRight size={18} color={colors["ink-faint"]} />
        )}
      </View>

      <View className="flex-row gap-2">
        <PhotoTile image={moment.back} locked={moment.locked} />
        <PhotoTile image={moment.front} locked={moment.locked} />
      </View>

      {moment.locked ? (
        <Text className="text-sm font-bold text-ink-soft">{t("friends.lockedCardHint")}</Text>
      ) : moment.caption ? (
        <Text numberOfLines={3} className="text-sm leading-5 text-ink">
          {moment.caption}
        </Text>
      ) : null}
      {!moment.locked && moment.retakes > 0 ? (
        <Text className="text-xs text-ink-faint">
          {t("card.retakes", { count: moment.retakes })}
        </Text>
      ) : null}
    </Pressable>
  );
}

function PhotoTile({ image, locked }: { image?: moments.Moment["back"]; locked: boolean }) {
  const { colors } = useTheme();
  const url = !locked && image?.url ? mediaUrl(image.url) : undefined;
  return (
    <View
      className="aspect-[3/4] flex-1 items-center justify-center overflow-hidden rounded-control"
      style={{ backgroundColor: image?.dominantColor || colors["surface-2"] }}
    >
      {url ? (
        <Image
          source={{ uri: url }}
          placeholder={image?.thumbhash ? { thumbhash: image.thumbhash } : undefined}
          contentFit="cover"
          style={{ width: "100%", height: "100%" }}
        />
      ) : locked ? (
        <LockKeyhole size={20} color={colors["ink-faint"]} />
      ) : null}
    </View>
  );
}
