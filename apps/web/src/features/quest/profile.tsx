import { Link } from "@tanstack/react-router";
import { create, type identity, quest } from "@tokuly/api";
import { ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { ProfileSection, ProfileSectionProps } from "#/seams/profile";
import { localized } from "./api";
import { BadgeMedal, LevelPill, TitlePill, useBadgeName } from "./parts";

/** The cards the quest domain attaches to a profile (`docs/events/quest.md`, "Profile cards"). */
const LEVEL_CARD = "quest.level";
const STREAK_CARD = "quest.streak";

const cardOf = (profile: identity.Profile, kind: string) =>
  profile.cards.find((card) => card.kind === kind);

/** The showcase as the level card carries it: a JSON array of `{ key, emoji, tier, name_ja, name_en }`. */
function showcaseOf(card: identity.ProfileCard | undefined): quest.Badge[] {
  try {
    const rows: unknown = JSON.parse(card?.data.badges ?? "[]");
    if (!Array.isArray(rows)) return [];
    return rows.flatMap((row) => {
      if (!row || typeof row !== "object") return [];
      const { key, emoji, tier, name_ja, name_en } = row as Record<string, unknown>;
      if (typeof key !== "string") return [];
      return [
        create(quest.BadgeSchema, {
          key,
          emoji: typeof emoji === "string" ? emoji : "",
          tier: Number(tier) || 0,
          name: { ja: String(name_ja ?? ""), en: String(name_en ?? "") },
          owned: true,
        }),
      ];
    });
  } catch {
    return [];
  }
}

/**
 * Under the bio: the person's level and the title (称号) they wear, and — on
 * a friend's profile — the streak the viewer has with them.
 */
function QuestStatus({ profile, user, isSelf }: ProfileSectionProps) {
  const { t, i18n } = useTranslation("quest");
  const levelCard = cardOf(profile, LEVEL_CARD);
  const streakCard = cardOf(profile, STREAK_CARD);
  if (!levelCard && !streakCard) return null;
  const level = Number(levelCard?.data.level) || 0;
  const title = localized(
    create(quest.TitleSchema, {
      text: { ja: levelCard?.data.title_ja ?? "", en: levelCard?.data.title_en ?? "" },
    }).text,
    i18n.language,
  );
  const days = Number(streakCard?.data.days) || 0;
  const expiring = Number(streakCard?.data.state) === quest.StreakState.EXPIRING;
  return (
    <div className="mt-4 flex flex-wrap items-center gap-2" data-testid="quest-profile-status">
      {levelCard && level > 0 && (
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
  const levelCard = cardOf(profile, LEVEL_CARD);
  if (!levelCard) return null;
  const badges = showcaseOf(levelCard);
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
