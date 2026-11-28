import { FlashList } from "@shopify/flash-list";
import { type moments, toMillis } from "@tokuly/api";
import { Image } from "expo-image";
import { router } from "expo-router";
import { ChevronRight, LockKeyhole } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { EmptyState, ErrorNote, Spinner } from "#/components/page";
import { SERVER_URL } from "#/lib/config";
import { useTheme } from "#/lib/theme";
import { useFriendMoments } from "./data";
import { lateLabel } from "./format";

const absolute = (url: string) => (url.startsWith("/") ? `${SERVER_URL}${url}` : url);

export function MomentFriendsScreen() {
  const { t } = useTranslation("moments");
  const friends = useFriendMoments();
  const items = friends.data?.pages.flatMap((page) => page.moments) ?? [];
  const unlocked = friends.data?.pages[0]?.unlocked ?? false;

  if (friends.isPending) {
    return (
      <View className="flex-1 bg-bg" testID="moment-friends">
        <Spinner />
      </View>
    );
  }
  if (friends.isError && !friends.data) {
    return (
      <View className="flex-1 gap-4 bg-bg px-4 pt-4" testID="moment-friends">
        <ErrorNote>{t("friends.loadError")}</ErrorNote>
        <Button variant="secondary" onPress={() => void friends.refetch()}>
          {t("retry")}
        </Button>
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
            <Text className="text-xl font-black text-ink">{t("friends.title")}</Text>
            <Text className="text-sm text-ink-soft">
              {unlocked ? t("friends.unlocked") : t("friends.locked")}
            </Text>
            {!unlocked && (
              <Button variant="secondary" onPress={() => router.push("/moments")}>
                {t("friends.shareToUnlock")}
              </Button>
            )}
          </View>
        }
        ListEmptyComponent={
          <EmptyState emoji="🌤️" title={t("friends.empty.title")} body={t("friends.empty.body")} />
        }
        ListFooterComponent={
          friends.isFetchingNextPage ? (
            <Spinner />
          ) : friends.isFetchNextPageError ? (
            <View className="p-4">
              <Button size="sm" variant="secondary" onPress={() => void friends.fetchNextPage()}>
                {t("retry")}
              </Button>
            </View>
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
          {posted && !moment.locked && (
            <Text className="text-xs text-ink-faint">
              {posted}
              {lateSeconds > 0
                ? ` · ${t("friends.late", { time: lateLabel(t, lateSeconds) })}`
                : ""}
            </Text>
          )}
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
    </Pressable>
  );
}

function PhotoTile({ image, locked }: { image?: moments.Moment["back"]; locked: boolean }) {
  const { colors } = useTheme();
  const url = !locked && image?.url ? absolute(image.url) : undefined;
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
          className="h-full w-full"
        />
      ) : locked ? (
        <LockKeyhole size={20} color={colors["ink-faint"]} />
      ) : null}
    </View>
  );
}
