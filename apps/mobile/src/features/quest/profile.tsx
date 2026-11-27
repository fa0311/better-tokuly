import { localized, profileQuest } from "@tokuly/core/quest";
import { router } from "expo-router";
import { ChevronRight } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { sticker, useTheme } from "#/lib/theme";
import type { ProfileSection, ProfileSectionProps } from "#/seams/profile";
import { BadgeMedal, LevelPill, TitlePill, useBadgeName } from "./parts";

/**
 * A person's quest card, read from their profile (no request of its own):
 * the level and the title they wear, the streak the viewer has with them,
 * and the badges they chose to show. The caller's own empty showcase
 * invites them to fill it; someone who keeps their progress to themselves
 * shows nothing but the streak.
 */
function QuestCard({ profile, isSelf }: ProfileSectionProps) {
  const { t, i18n } = useTranslation("quest");
  const { colors } = useTheme();
  const badgeName = useBadgeName();
  const read = profileQuest(profile);
  const streak = !isSelf && read.streakDays > 0;
  if (!read.shown && !streak) return null;
  const title = localized(read.title, i18n.language);
  return (
    <View
      testID="quest-profile"
      className="gap-3 rounded-card bg-surface p-4"
      style={sticker(colors)}
    >
      <View className="flex-row flex-wrap items-center gap-2">
        {read.level > 0 && <LevelPill level={read.level} />}
        {title ? <TitlePill title={title} /> : null}
        {streak && (
          <View className="rounded-full bg-surface-2 px-2.5 py-0.5">
            <Text className="text-xs font-black text-ink">
              {read.streakExpiring ? "⌛ " : "🔥 "}
              {t(read.streakExpiring ? "profile.streakExpiring" : "profile.streak", {
                count: read.streakDays,
              })}
            </Text>
          </View>
        )}
      </View>
      {read.shown && (read.showcase.length > 0 || isSelf) && (
        <>
          <Pressable
            testID="quest-profile-badges"
            accessibilityRole="button"
            onPress={() => router.push("/quest/badges")}
            className="flex-row items-center justify-between"
          >
            <Text className="font-black text-ink">{t("profile.showcase")}</Text>
            {isSelf && (
              <View className="flex-row items-center">
                <Text className="text-sm font-black text-brand">{t("profile.edit")}</Text>
                <ChevronRight size={16} color={colors.brand} />
              </View>
            )}
          </Pressable>
          {read.showcase.length === 0 ? (
            <Text className="text-sm font-bold text-ink-soft">{t("profile.showcaseEmpty")}</Text>
          ) : (
            <View className="flex-row flex-wrap gap-x-3 gap-y-4">
              {read.showcase.map((badge) => (
                <Pressable
                  key={badge.key}
                  testID={`quest-profile-badge-${badge.key}`}
                  accessibilityRole="button"
                  accessibilityLabel={badgeName(badge)}
                  onPress={() => router.push(`/quest/badges/${encodeURIComponent(badge.key)}`)}
                  className="w-16 items-center gap-1"
                >
                  <BadgeMedal badge={badge} owned size="sm" />
                  <Text numberOfLines={2} className="text-center text-[11px] font-bold text-ink">
                    {badgeName(badge)}
                  </Text>
                </Pressable>
              ))}
            </View>
          )}
        </>
      )}
    </View>
  );
}

/** Under the numbers of a profile: the person's level, title, streak and showcase. */
export default [
  { id: "quest", slot: "content", order: 5, Component: QuestCard },
] satisfies ProfileSection[];
