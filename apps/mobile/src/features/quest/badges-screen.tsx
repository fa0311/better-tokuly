import type { quest } from "@tokuly/api";
import { badgeLines, fraction, isHiddenSecret, nextInLine } from "@tokuly/core/quest";
import { router } from "expo-router";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState, Page, Spinner } from "#/components/page";
import { sticker, useTheme } from "#/lib/theme";
import { useBadgeCatalog, useLocalized, useUserBadges } from "./data";
import { BadgeMedal, ProgressBar, RarityChip, TitlePill, useBadgeName } from "./parts";

/**
 * `/quest/badges`: the whole collection by family, with what the caller
 * shows on their profile. Tiers of one achievement sit side by side; what
 * is not earned yet is greyed with how far it is, and secret badges stay
 * question marks until they are earned.
 */
export function BadgesScreen() {
  const { t } = useTranslation(["quest", "common"]);
  const { colors } = useTheme();
  const text = useLocalized();
  const { data, isPending, isError, refetch } = useBadgeCatalog();
  const mine = useUserBadges("");

  if (isPending)
    return (
      <Page>
        <Spinner />
      </Page>
    );
  if (isError || !data)
    return (
      <Page>
        <EmptyState
          emoji="🛰️"
          title={t("badges.error")}
          action={
            <Button variant="secondary" onPress={() => void refetch()}>
              {t("common:actions.retry")}
            </Button>
          }
        />
      </Page>
    );

  return (
    <Page testID="quest-badges-screen">
      <View className="gap-2 rounded-card bg-surface p-4" style={sticker(colors)}>
        <Text testID="quest-badge-count" className="text-2xl font-black text-ink">
          {t("badges.count", { owned: data.ownedCount, total: data.totalCount })}
        </Text>
        <Text className="text-xs font-black text-ink-soft">{t("badges.showcase")}</Text>
        <View className="min-h-10 flex-row flex-wrap items-center gap-2">
          {mine.data?.showcase.length ? (
            mine.data.showcase.map((badge) => (
              <BadgeMedal key={badge.key} badge={badge} owned size="sm" />
            ))
          ) : (
            <Text className="text-sm font-bold text-ink-soft">{t("badges.showcaseEmpty")}</Text>
          )}
          {mine.data?.title && <TitlePill title={text(mine.data.title.text)} />}
        </View>
      </View>
      {data.families.map((family) => (
        <Family key={family.key} family={family} />
      ))}
    </Page>
  );
}

function Family({ family }: { family: quest.BadgeFamily }) {
  const { t } = useTranslation("quest");
  const { colors } = useTheme();
  const text = useLocalized();
  return (
    <View className="gap-1 rounded-card bg-surface p-4" style={sticker(colors)}>
      <View className="flex-row items-center justify-between gap-3">
        <Text className="text-lg font-black text-ink">
          {family.emoji} {text(family.name)}
        </Text>
        <Text className="text-sm font-black text-ink-soft">
          {t("badges.familyCount", { owned: family.ownedCount, total: family.badges.length })}
        </Text>
      </View>
      {badgeLines(family.badges).map((line) => (
        <Line key={line[0]?.key} line={line} />
      ))}
    </View>
  );
}

/** One achievement: its tiers as medals, its name, and the way to the next tier. */
function Line({ line }: { line: quest.Badge[] }) {
  const { t } = useTranslation("quest");
  const text = useLocalized();
  const badgeName = useBadgeName();
  const next = nextInLine(line);
  if (!next) return null;
  const secret = isHiddenSecret(next);
  const ownedAll = line.every((badge) => badge.owned);
  const counted = next.target > 0n && !next.owned;
  const name = text(next.name) || t("badges.secretName");
  return (
    <View className="flex-row items-center gap-3 py-2">
      <View className="flex-row gap-1.5">
        {line.map((badge) => (
          <Pressable
            key={badge.key}
            testID={`badge-${badge.key}`}
            accessibilityRole="button"
            accessibilityLabel={t(badge.owned ? "badges.openOwned" : "badges.openLocked", {
              name: badgeName(badge),
            })}
            onPress={() => router.push(`/quest/badges/${encodeURIComponent(badge.key)}`)}
          >
            <BadgeMedal badge={badge} size={line.length > 1 ? "sm" : "md"} />
          </Pressable>
        ))}
      </View>
      <View className="flex-1 gap-0.5">
        <View className="flex-row flex-wrap items-center gap-x-2 gap-y-0.5">
          <Text className="shrink font-black text-ink">{name}</Text>
          {!secret && <RarityChip rarity={next.rarity} />}
        </View>
        <Text className="text-sm text-ink-soft">
          {secret ? t("badges.secretHint") : text(next.description)}
        </Text>
        {counted && (
          <View className="mt-1 flex-row items-center gap-2">
            <ProgressBar
              thin
              value={fraction(next.progress, next.target)}
              label={t("badges.progressLabel", { name })}
            />
            <Text className="text-xs font-black text-ink-soft">
              {Number(next.progress).toLocaleString()} / {Number(next.target).toLocaleString()}
            </Text>
          </View>
        )}
        {ownedAll && !secret && (
          <Text className="text-xs font-black text-success">{t("badges.complete")}</Text>
        )}
      </View>
    </View>
  );
}
