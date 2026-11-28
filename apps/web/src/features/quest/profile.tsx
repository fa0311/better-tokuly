import { Link } from "@tanstack/react-router";
import { localized, profileQuest } from "@tokuly/core/quest";
import { ChevronRight } from "lucide-react";
import { useTranslation } from "#/lib/use-translation";
import type { ProfileSection, ProfileSectionProps } from "#/seams/profile";
import { BadgeMedal, LevelPill, TitlePill, useBadgeName } from "./parts";

/**
 * Under the bio: the person's level and the title (称号) they wear, and — on
 * a friend's profile — the streak the viewer has with them.
 */
function QuestStatus({ profile, user, isSelf }: ProfileSectionProps) {
  const { t, i18n } = useTranslation("quest");
  const {
    shown,
    level,
    title: titleText,
    streakDays: days,
    streakExpiring: expiring,
  } = profileQuest(profile);
  if (!shown && days === 0) return null;
  const title = localized(titleText, i18n.language);
  return (
    <div className="mt-4 flex flex-wrap items-center gap-2" data-testid="quest-profile-status">
      {level > 0 && (
        <Link
          to="/quest/u/$handle"
          params={{ handle: user.handle }}
          aria-label={t("profile.open", { name: user.displayName })}
          className="inline-flex items-center gap-1.5"
        >
          <LevelPill level={level} className="h-6 px-2 text-xs" />
          {title && <TitlePill title={title} className="h-6 px-2 text-xs" />}
        </Link>
      )}
      {!isSelf && days > 0 && (
        <Link
          to="/quest/streaks"
          className="sticker-sm inline-flex h-7 items-center gap-1 rounded-full bg-surface px-2.5 text-xs font-black"
        >
          <span aria-hidden>{expiring ? "⌛" : "🔥"}</span>
          {t(expiring ? "profile.streakExpiring" : "profile.streak", { count: days })}
        </Link>
      )}
    </div>
  );
}

/** The badges the person chose to show. Their own empty showcase invites them to fill it. */
function QuestShowcase({ profile, user, isSelf }: ProfileSectionProps) {
  const { t } = useTranslation("quest");
  const badgeName = useBadgeName();
  const { shown, showcase: badges } = profileQuest(profile);
  if (!shown) return null;
  if (badges.length === 0 && !isSelf) return null;
  return (
    <section
      aria-label={t("profile.showcase")}
      className="sticker mt-5 rounded-card bg-surface p-4"
    >
      <div className="flex items-center justify-between gap-3">
        <h2 className="font-black">{t("profile.showcase")}</h2>
        <Link
          to={isSelf ? "/quest/showcase" : "/quest/u/$handle"}
          params={isSelf ? undefined : { handle: user.handle }}
          className="inline-flex items-center text-sm font-black text-brand"
        >
          {isSelf ? t("profile.edit") : t("profile.more")}
          <ChevronRight className="size-4" />
        </Link>
      </div>
      {badges.length === 0 ? (
        <p className="mt-2 text-sm font-bold text-ink-soft">{t("profile.showcaseEmpty")}</p>
      ) : (
        <ul className="mt-3 flex flex-wrap gap-x-3 gap-y-4">
          {badges.map((badge) => (
            <li key={badge.key} className="w-20">
              <Link
                to="/quest/badges/$key"
                params={{ key: badge.key }}
                className="flex flex-col items-center gap-2 text-center"
              >
                <BadgeMedal badge={badge} owned size="md" />
                <span className="line-clamp-2 text-[11px] leading-tight font-bold">
                  {badgeName(badge)}
                </span>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

export default [
  { id: "quest", slot: "status", order: 30, Component: QuestStatus },
  { id: "quest-showcase", slot: "card", order: 40, Component: QuestShowcase },
] satisfies ProfileSection[];
