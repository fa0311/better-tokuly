import { errorReason, toMillis } from "@tokuly/api";
import { DEFAULT_TIME_ZONE, zonedDate } from "@tokuly/core/format";
import { formatDay, formatShare, fraction, isHiddenSecret, tierKey } from "@tokuly/core/quest";
import { router, Stack } from "expo-router";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { EmptyState, Page, Spinner } from "#/components/page";
import { useMe } from "#/features/session/store";
import { errorMessage } from "#/lib/errors";
import { INK_ON_BRIGHT, sticker, useTheme } from "#/lib/theme";
import { useBadgeChoices } from "./actions";
import { useBadge, useLocalized, useUserBadges } from "./data";
import { BadgeMedal, ProgressBar, RarityChip, TitlePill, useBadgeName } from "./parts";

/** How many badges a showcase holds. */
const SHOWCASE_SIZE = 5;

/**
 * `/quest/badges/<key>`: one badge — how it is earned, how rare it is, how
 * far the caller is, which friends have it — and, once earned, showing it
 * on the profile and wearing its title.
 */
export function BadgeScreen({ badgeKey }: { badgeKey: string }) {
  const { t, i18n } = useTranslation(["quest", "common"]);
  const { colors } = useTheme();
  const text = useLocalized();
  const badgeName = useBadgeName();
  const zone = useMe()?.timeZone || DEFAULT_TIME_ZONE;
  const { data, isPending, error } = useBadge(badgeKey);
  const mine = useUserBadges("");
  const { showcase, title: wear } = useBadgeChoices();
  const badge = data?.badge;

  if (isPending)
    return (
      <Page>
        <Spinner />
      </Page>
    );
  if (!badge)
    return (
      <Page>
        <EmptyState
          emoji="🫥"
          title={
            errorReason(error) === "BADGE_NOT_FOUND" || !error
              ? t("badge.notFound")
              : errorMessage(error, { namespace: "quest" })
          }
          body={t("badge.notFoundBody")}
        />
      </Page>
    );

  const secret = isHiddenSecret(badge);
  const name = badgeName(badge);
  const title = text(badge.title);
  const tier = tierKey(badge.tier);
  // The day it was earned, on the account's calendar.
  const earned = badge.earnedAt ? zonedDate(toMillis(badge.earnedAt), zone) : undefined;
  // The showcase is sent whole, so it is changed only from the one known to be shown now.
  const known = mine.data !== undefined;
  const showcaseKeys = mine.data?.showcase.map((item) => item.key) ?? [];
  const shown = showcaseKeys.includes(badge.key);
  const wearing = mine.data?.title?.badgeKey === badge.key;
  const full = !shown && showcaseKeys.length >= SHOWCASE_SIZE;

  return (
    <Page testID="quest-badge-screen">
      <Stack.Screen options={{ title: name }} />
      <View
        className="items-center gap-3 rounded-card bg-surface px-5 pt-8 pb-6"
        style={sticker(colors)}
      >
        <BadgeMedal badge={badge} size="xl" />
        <Text testID="quest-badge-name" className="text-center text-2xl font-black text-ink">
          {name}
        </Text>
        <View className="flex-row flex-wrap items-center justify-center gap-2">
          {tier && (
            <View className="rounded-full bg-surface-2 px-2.5 py-0.5">
              <Text className="text-xs font-black text-ink-soft">
                {t("badge.tier", { tier: t(`tiers.${tier}`) })}
              </Text>
            </View>
          )}
          <RarityChip rarity={badge.rarity} />
          {badge.ownerShare > 0 && (
            <Text className="text-xs font-bold text-ink-soft">
              {t("badge.ownerShare", { share: formatShare(badge.ownerShare) })}
            </Text>
          )}
        </View>
        <Text className="text-center leading-6 text-ink-soft">
          {secret ? t("badges.secretHint") : text(badge.description)}
        </Text>
        {title ? (
          <View className="flex-row flex-wrap items-center justify-center gap-2">
            <Text className="text-sm font-bold text-ink">{t("badge.unlocksTitle")}</Text>
            <TitlePill title={title} />
          </View>
        ) : null}
        <View className={`rounded-full px-4 py-1.5 ${badge.owned ? "bg-lime" : "bg-surface-2"}`}>
          <Text
            testID="quest-badge-state"
            className="text-sm font-black"
            style={{ color: badge.owned ? INK_ON_BRIGHT : colors["ink-soft"] }}
          >
            {badge.owned
              ? `${earned ? t("badge.earnedOn", { date: formatDay(earned, i18n.language, true) }) : t("badge.earned")}${badge.times > 1 ? ` ${t("badge.times", { count: badge.times })}` : ""}`
              : t("badge.locked")}
          </Text>
        </View>
        {badge.target > 0n && !badge.owned && (
          <View className="w-full flex-row items-center gap-2">
            <ProgressBar
              value={fraction(badge.progress, badge.target)}
              label={t("badges.progressLabel", { name })}
            />
            <Text className="text-sm font-black text-ink">
              {Number(badge.progress).toLocaleString()} / {Number(badge.target).toLocaleString()}
            </Text>
          </View>
        )}
        {badge.owned && (
          <View className="w-full gap-2 pt-2">
            <Button
              testID="quest-badge-showcase"
              block
              variant={shown ? "secondary" : "lime"}
              loading={showcase.isPending}
              disabled={!known || full}
              onPress={() =>
                showcase.mutate(
                  shown
                    ? showcaseKeys.filter((key) => key !== badge.key)
                    : [...showcaseKeys, badge.key],
                )
              }
            >
              {shown ? t("badge.hide") : t("badge.show")}
            </Button>
            {mine.isError && (
              <View className="flex-row flex-wrap items-center justify-center gap-2">
                <Text className="text-sm font-bold text-ink-soft">{t("badges.error")}</Text>
                <Button variant="secondary" size="sm" onPress={() => void mine.refetch()}>
                  {t("common:actions.retry")}
                </Button>
              </View>
            )}
            {full && (
              <Text className="text-center text-sm font-bold text-ink-soft">
                {t("badge.showcaseFull", { count: SHOWCASE_SIZE })}
              </Text>
            )}
            {title ? (
              <Button
                testID="quest-badge-title"
                block
                variant={wearing ? "secondary" : "primary"}
                loading={wear.isPending}
                disabled={!known}
                onPress={() => wear.mutate(wearing ? "" : badge.key)}
              >
                {wearing ? t("badge.removeTitle") : t("badge.wearTitle", { title })}
              </Button>
            ) : null}
          </View>
        )}
      </View>

      <View className="gap-2">
        <Text className="px-1 text-lg font-black text-ink">{t("badge.friends")}</Text>
        {data.friendOwnerCount === 0 ? (
          <Text className="px-1 text-sm font-bold text-ink-soft">
            {badge.owned ? t("badge.friendsNoneOwned") : t("badge.friendsNone")}
          </Text>
        ) : (
          <View className="rounded-card bg-surface p-2" style={sticker(colors)}>
            {data.friendOwners.map((friend) => (
              <Pressable
                key={friend.id}
                accessibilityRole="button"
                onPress={() => router.push(`/u/${friend.handle}`)}
                className="flex-row items-center gap-3 rounded-2xl px-3 py-2 active:bg-surface-2"
              >
                <Avatar user={friend} size="sm" />
                <Text numberOfLines={1} className="flex-1 font-bold text-ink">
                  {friend.displayName}
                </Text>
              </Pressable>
            ))}
            {data.friendOwnerCount > data.friendOwners.length && (
              <Text className="px-3 py-2 text-sm font-bold text-ink-soft">
                {t("badge.friendsMore", {
                  count: data.friendOwnerCount - data.friendOwners.length,
                })}
              </Text>
            )}
          </View>
        )}
      </View>
    </Page>
  );
}
