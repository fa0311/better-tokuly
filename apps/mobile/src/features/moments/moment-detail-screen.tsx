import { toMillis } from "@tokuly/api";
import { Image } from "expo-image";
import { router, useLocalSearchParams } from "expo-router";
import { MapPin } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Linking, Platform, Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { EmptyState, ErrorNote, Page, Spinner } from "#/components/page";
import { mediaUrl } from "#/lib/config";
import { useTheme } from "#/lib/theme";
import { useMoment } from "./data";
import { lateLabel } from "./format";

export function MomentDetailScreen() {
  const { momentId } = useLocalSearchParams<{ momentId: string }>();
  const { t, i18n } = useTranslation("moments");
  const { colors } = useTheme();
  const query = useMoment(momentId);
  const moment = query.data;

  if (query.isPending || query.isFetching) {
    return (
      <Page testID="moment-detail">
        <Spinner />
      </Page>
    );
  }
  if (query.isError) {
    return (
      <Page testID="moment-detail">
        <ErrorNote>{t("friends.detailError")}</ErrorNote>
        <Button variant="secondary" onPress={() => void query.refetch()}>
          {t("composer.retry")}
        </Button>
      </Page>
    );
  }
  if (!moment || moment.locked) {
    return (
      <Page testID="moment-detail">
        <EmptyState
          emoji="🔒"
          title={t("friends.lockedDetail")}
          body={t("friends.lockedDetailBody")}
        />
        <Button variant="secondary" onPress={() => router.push("/moments")}>
          {t("friends.shareToUnlock")}
        </Button>
      </Page>
    );
  }

  const posted = moment.postedAt
    ? new Intl.DateTimeFormat(i18n.language, { dateStyle: "medium", timeStyle: "short" }).format(
        new Date(toMillis(moment.postedAt)),
      )
    : undefined;
  const lateSeconds = Number(moment.lateSeconds);
  const location = moment.location;

  return (
    <Page testID="moment-detail">
      <View className="flex-row items-center gap-3 rounded-card bg-surface p-4">
        <Avatar user={moment.author} size="lg" />
        <View className="min-w-0 flex-1 gap-1">
          <Text numberOfLines={1} className="text-lg font-black text-ink">
            {moment.author?.displayName || moment.author?.handle || t("friends.someone")}
          </Text>
          {posted && <Text className="text-xs text-ink-soft">{posted}</Text>}
          {lateSeconds > 0 && (
            <Text className="text-xs font-bold text-brand">
              {t("friends.late", { time: lateLabel(t, lateSeconds) })}
            </Text>
          )}
        </View>
      </View>

      <View className="flex-row gap-3">
        <MomentPhoto url={moment.back?.url} alt={t("card.back")} />
        <MomentPhoto url={moment.front?.url} alt={t("card.front")} />
      </View>

      {moment.caption ? (
        <View className="rounded-card bg-surface p-4">
          <Text className="text-base leading-6 text-ink">{moment.caption}</Text>
        </View>
      ) : null}

      {moment.retakes > 0 ? (
        <Text className="text-xs text-ink-faint">
          {t("card.retakes", { count: moment.retakes })}
        </Text>
      ) : null}

      {location && (
        <Pressable
          accessibilityRole="button"
          onPress={() => {
            const { lat, lng } = location;
            const url =
              Platform.OS === "android"
                ? `geo:${lat},${lng}?q=${lat},${lng}`
                : `https://maps.apple.com/?ll=${lat},${lng}`;
            void Linking.openURL(url).catch(() => {});
          }}
          className="flex-row items-center gap-3 rounded-card bg-surface p-4 active:bg-surface-2"
        >
          <MapPin size={20} color={colors.brand} />
          <Text className="flex-1 text-sm font-bold text-ink">{t("card.place")}</Text>
        </Pressable>
      )}
    </Page>
  );
}

function MomentPhoto({ url, alt }: { url?: string; alt: string }) {
  return (
    <View className="aspect-[3/4] flex-1 items-center justify-center overflow-hidden rounded-card bg-surface">
      {url ? (
        <Image
          accessibilityLabel={alt}
          source={{ uri: mediaUrl(url) ?? url }}
          contentFit="cover"
          style={{ width: "100%", height: "100%" }}
        />
      ) : (
        <Text className="text-xs text-ink-faint">{alt}</Text>
      )}
    </View>
  );
}
