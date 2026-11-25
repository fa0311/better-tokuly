import { useQuery } from "@tanstack/react-query";
import { toDate } from "@tokuly/api";
import { profileQuery, relationshipOf } from "@tokuly/core/profile";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { EmptyState, Page, Spinner } from "#/components/page";
import { profileClient } from "#/lib/api";
import { sticker, useTheme } from "#/lib/theme";

/**
 * A person's profile, read-only for now: who they are, their status and bio,
 * how the viewer is connected to them, and their numbers. `footer` holds the
 * screen's own actions (the own profile's settings).
 */
export function ProfileScreen({ handle, footer }: { handle: string; footer?: ReactNode }) {
  const { t, i18n } = useTranslation("profile");
  const { colors } = useTheme();
  const { data, isPending, isError } = useQuery({
    ...profileQuery(profileClient, handle),
    enabled: Boolean(handle),
  });
  if (isPending) return <Spinner />;
  const profile = data?.profile;
  if (isError || !profile?.user) return <EmptyState emoji="🫥" title={t("notFound")} />;
  const { isSelf, isFriend } = relationshipOf(profile);
  const user = profile.user;
  const friendsSince = toDate(profile.friendsSince);
  const stats = profile.stats;
  const numbers = stats
    ? [
        { label: t("stats.friends"), value: stats.friendCount },
        { label: t("stats.intros"), value: stats.introCount },
        { label: t("stats.bumps"), value: stats.bumpCount },
        { label: t("stats.views"), value: Number(stats.profileViewCount) },
      ]
    : [];
  return (
    <Page testID={`profile-${user.handle}`}>
      <View className="items-center gap-2 pt-4">
        <Avatar user={user} size="xl" outlined />
        <Text className="text-2xl font-black text-ink">{user.displayName}</Text>
        <Text className="text-sm font-bold text-ink-soft">@{user.handle}</Text>
        {profile.bot && <Text className="text-xs font-black text-brand">{t("bot")}</Text>}
        {profile.status?.text || profile.status?.emoji ? (
          <Text className="rounded-full bg-surface px-3 py-1 text-sm text-ink">
            {profile.status.emoji} {profile.status.text}
          </Text>
        ) : null}
      </View>
      {profile.bio ? <Text className="text-center text-base text-ink">{profile.bio}</Text> : null}
      {!isSelf && (
        <View className="items-center gap-1">
          {profile.bestFriend && (
            <Text className="text-sm font-black text-pink">💖 {t("bestFriend")}</Text>
          )}
          {isFriend && friendsSince && (
            <Text className="text-sm text-ink-soft">
              {t("friendsSince", { date: friendsSince.toLocaleDateString(i18n.language) })}
            </Text>
          )}
          {profile.mutualFriendCount > 0 && (
            <Text className="text-sm text-ink-soft">
              {t("mutual", { count: profile.mutualFriendCount })}
            </Text>
          )}
        </View>
      )}
      {numbers.length > 0 && (
        <View className="flex-row rounded-card bg-surface py-3" style={sticker(colors)}>
          {numbers.map((cell) => (
            <View key={cell.label} className="flex-1 items-center">
              <Text className="text-lg font-black text-ink">{cell.value}</Text>
              <Text className="text-xs font-bold text-ink-soft">{cell.label}</Text>
            </View>
          ))}
        </View>
      )}
      {stats && stats.daysSinceJoined > 0 && (
        <Text className="text-center text-xs text-ink-faint">
          {t("joined", { name: user.displayName, count: stats.daysSinceJoined })}
        </Text>
      )}
      {footer}
    </Page>
  );
}
