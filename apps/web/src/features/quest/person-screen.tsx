import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { identity, type quest, toMillis } from "@tokuly/api";
import { profileQuery } from "@tokuly/core/profile";
import { formatDay, streakRuns, streakStatus } from "@tokuly/core/quest";
import { EyeOff, MessageCircle } from "lucide-react";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { type BackTarget, EmptyState, Page, Skeleton } from "#/components/ui/page";
import { openDirectChat } from "#/features/chat";
import { EmojiText } from "#/features/emoji";
import { useAccountNow, useAccountTimeZone, zonedDate } from "#/lib/account-time";
import { profileClient } from "#/lib/api";
import { useTranslation } from "#/lib/use-translation";
import { useLocalized, useQuestProfile, useUserBadges } from "./api";
import { BadgeMedal, LevelPill, SectionTitle, TitlePill, useBadgeName } from "./parts";

/**
 * The page is opened from rankings, badges, profiles and leaderboards alike: "back" returns to
 * wherever the person came from, and to the rankings when it was opened directly.
 */
const PERSON_BACK: BackTarget = { fallback: "/quest/ranking" };

/**
 * `/quest/u/<handle>`: what a person shows of their quest progress — level,
 * title, showcase, the streak with the viewer — and, for themselves and
 * their friends, every badge they earned.
 */
export function PersonScreen({ handle }: { handle: string }) {
  const { t } = useTranslation("quest");
  const text = useLocalized();
  const zone = useAccountTimeZone();
  const { ms: now } = useAccountNow(60_000);
  // The same query the profile page uses: coming from there, the person is already known.
  const person = useQuery(profileQuery(profileClient, handle));
  const profile = person.data?.profile;
  const user = profile?.user;
  const isSelf = profile?.relationship === identity.Relationship.SELF;
  const isFriend = profile?.relationship === identity.Relationship.FRIENDS;
  const questProfile = useQuestProfile(user?.id);
  const badges = useUserBadges(user?.id ?? "", Boolean(user));

  if (person.isPending || (user && questProfile.isPending))
    return (
      <Page title={t("person.title")} back={PERSON_BACK}>
        <Skeleton className="h-44" />
        <Skeleton className="mt-4 h-40" />
      </Page>
    );
  const shown = questProfile.data;
  if (!user || !shown)
    return (
      <Page title={t("person.title")} back={PERSON_BACK}>
        <EmptyState emoji="🧭" title={t("person.notFound")} />
      </Page>
    );

  const title = text(shown.title?.text);
  const streak = shown.streak;
  const status = streak ? streakStatus(streak, now, zone) : "none";
  const showcaseKeys = new Set(shown.showcase.map((badge) => badge.key));
  const others = (badges.data?.badges ?? []).filter((badge) => !showcaseKeys.has(badge.key));
  const seesAll = isSelf || isFriend;

  return (
    <Page title={t("person.titleOf", { name: user.displayName })} back={PERSON_BACK}>
      <section className="sticker rounded-card bg-surface p-5">
        <div className="flex items-center gap-4">
          <Link to="/u/$handle" params={{ handle: user.handle }} className="shrink-0">
            <Avatar user={user} size="lg" outlined />
          </Link>
          <div className="min-w-0 flex-1">
            <p className="truncate text-xl font-black">
              <EmojiText text={user.displayName} />
            </p>
            <p className="truncate text-sm font-bold text-ink-soft">@{user.handle}</p>
            {!shown.hidden && (
              <p className="mt-1.5 flex flex-wrap items-center gap-1.5">
                <LevelPill level={shown.level} className="h-6 px-2 text-xs" />
                {title && <TitlePill title={title} className="h-6 px-2 text-xs" />}
              </p>
            )}
          </div>
        </div>

        {shown.hidden ? (
          <p className="mt-4 flex items-center gap-2 rounded-2xl bg-surface-2 px-4 py-3 text-sm font-bold text-ink-soft">
            <EyeOff className="size-4 shrink-0" />
            {t("person.hidden", { name: user.displayName })}
          </p>
        ) : (
          <p className="mt-4 text-sm font-bold text-ink-soft">
            {t("person.badgeCount", { count: shown.badgeCount })}
          </p>
        )}

        {isSelf && (
          <div className="mt-4 flex flex-wrap gap-2">
            <Link
              to="/quest/showcase"
              className="pressable sticker-sm inline-flex h-9 items-center rounded-full bg-lime px-4 text-sm font-black text-[#17122b]"
            >
              {t("badges.editShowcase")}
            </Link>
            <Link
              to="/quest/settings"
              className="inline-flex h-9 items-center rounded-full px-3 text-sm font-black text-brand hover:bg-surface-2"
            >
              {t("settings.title")}
            </Link>
          </div>
        )}
      </section>

      {streak && streakRuns(streak) && (
        <section
          aria-label={t("person.streak")}
          className="sticker mt-5 flex items-center gap-3 rounded-card bg-surface p-4"
        >
          <span className="text-4xl leading-none" aria-hidden>
            {status === "expiring" ? "⌛" : "🔥"}
          </span>
          <div className="min-w-0 flex-1">
            <p className="font-black">{t("person.streakDays", { count: streak.days })}</p>
            <p className="text-sm font-bold text-ink-soft">
              {t(`streaks.status.${status === "expiring" ? "expiringSoon" : status}`, {
                name: user.displayName,
              })}
            </p>
          </div>
          <Button
            size="sm"
            icon={<MessageCircle className="size-4" />}
            onClick={() => void openDirectChat(user.id)}
          >
            {t("streaks.message")}
          </Button>
        </section>
      )}

      {!shown.hidden && (
        <>
          <section aria-label={t("person.showcase")} className="mt-7">
            <SectionTitle>{t("person.showcase")}</SectionTitle>
            {shown.showcase.length === 0 ? (
              <p className="mt-2 text-sm font-bold text-ink-soft">
                {isSelf ? t("badges.showcaseEmpty") : t("person.showcaseEmpty")}
              </p>
            ) : (
              <BadgeGrid badges={shown.showcase} />
            )}
          </section>

          {seesAll && others.length > 0 && (
            <section aria-label={t("person.allBadges")} className="mt-7">
              <SectionTitle aside={t("person.onlyFriends")}>{t("person.allBadges")}</SectionTitle>
              <BadgeGrid badges={others} dated />
            </section>
          )}
        </>
      )}
    </Page>
  );
}

function BadgeGrid({ badges, dated }: { badges: quest.Badge[]; dated?: boolean }) {
  const { i18n } = useTranslation("quest");
  const zone = useAccountTimeZone();
  const badgeName = useBadgeName();
  return (
    <ul className="mt-3 grid grid-cols-3 gap-2 sm:grid-cols-4">
      {badges.map((badge) => {
        const earned =
          dated && badge.earnedAt ? zonedDate(toMillis(badge.earnedAt), zone) : undefined;
        return (
          <li key={badge.key}>
            <Link
              to="/quest/badges/$key"
              params={{ key: badge.key }}
              className="pressable flex h-full flex-col items-center gap-2 rounded-2xl border-2 border-line bg-surface px-1 pt-3 pb-2 text-center"
            >
              <BadgeMedal badge={badge} owned size="md" />
              <span className="line-clamp-2 text-xs leading-tight font-bold">
                {badgeName(badge)}
              </span>
              {earned && (
                <span className="text-[10px] font-bold text-ink-faint">
                  {formatDay(earned, i18n.language)}
                </span>
              )}
            </Link>
          </li>
        );
      })}
    </ul>
  );
}
