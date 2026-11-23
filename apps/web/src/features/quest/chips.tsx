import { useQuery } from "@tanstack/react-query";
import { errorReason, quest } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { cn } from "#/lib/cn";
import { questKeys, streakClient, useLocalized, useQuestProfile, useStreakList } from "./api";
import { streakRuns } from "./model";
import { LevelPill, TitlePill } from "./parts";

/**
 * A person's level, small enough to sit next to their name. Nothing while it
 * loads, and nothing for someone who keeps their progress off their profile.
 */
export function LevelChip({ userId, className }: { userId: string; className?: string }) {
  const { data } = useQuestProfile(userId);
  if (!data || data.hidden || data.level <= 0) return null;
  return <LevelPill level={data.level} className={className} />;
}

/** The title (称号) a person wears, as a small label; nothing when they wear none. */
export function TitleBadge({ userId, className }: { userId: string; className?: string }) {
  const text = useLocalized();
  const { data } = useQuestProfile(userId);
  const title = data && !data.hidden ? text(data.title?.text) : "";
  if (!title) return null;
  return <TitlePill title={title} className={className} />;
}

/**
 * The streak between the signed-in person and a friend: 🔥 and the days (⌛
 * when it is about to end). Nothing when no streak is running or the two are
 * not friends.
 */
export function StreakFlame({ with: friendId, className }: { with: string; className?: string }) {
  const { t } = useTranslation("quest");
  const list = useStreakList();
  const listed = list.data?.streaks.find((streak) => streak.friend?.id === friendId);
  // Someone with more streaks than one page holds: ask for this one.
  const beyond = Boolean(list.data?.page?.hasMore) && !listed;
  const single = useQuery({
    queryKey: questKeys.streak(friendId),
    queryFn: async () => {
      try {
        return (await streakClient().getStreak({ userId: friendId })).streak ?? null;
      } catch (error) {
        if (errorReason(error) === "NOT_FRIENDS") return null;
        throw error;
      }
    },
    enabled: beyond,
    retry: false,
  });
  const streak = listed ?? single.data ?? undefined;
  if (!streakRuns(streak)) return null;
  const expiring = streak.state === quest.StreakState.EXPIRING;
  return (
    <span
      role="img"
      aria-label={t(expiring ? "flame.expiring" : "flame.label", { count: streak.days })}
      className={cn(
        "inline-flex shrink-0 items-center text-sm leading-none font-black tabular-nums",
        className,
      )}
    >
      <span aria-hidden>{expiring ? "⌛" : "🔥"}</span>
      {streak.days}
    </span>
  );
}
