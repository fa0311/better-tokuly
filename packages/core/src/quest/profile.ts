import { create, type identity, quest } from "@tokuly/api";

/** The cards the quest domain attaches to a profile (`docs/events/quest.md`, "Profile cards"). */
const LEVEL_CARD = "quest.level";
const STREAK_CARD = "quest.streak";

const cardOf = (profile: identity.Profile, kind: string) =>
  profile.cards.find((card) => card.kind === kind);

/** What a profile's quest cards say: level and title, the showcase, and the streak with the viewer. */
export interface ProfileQuest {
  /** The person shows their progress (the level card is there). */
  shown: boolean;
  /** 0 when not shown. */
  level: number;
  title: { ja: string; en: string };
  /** The showcased badges, as owned badges with key, emoji, tier and name. */
  showcase: quest.Badge[];
  /** The days of the streak the viewer has with them; 0 without one. */
  streakDays: number;
  streakExpiring: boolean;
}

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

/** Reads the quest cards of a profile. */
export function profileQuest(profile: identity.Profile): ProfileQuest {
  const level = cardOf(profile, LEVEL_CARD);
  const streak = cardOf(profile, STREAK_CARD);
  return {
    shown: Boolean(level),
    level: Number(level?.data.level) || 0,
    title: { ja: level?.data.title_ja ?? "", en: level?.data.title_en ?? "" },
    showcase: showcaseOf(level),
    streakDays: Number(streak?.data.days) || 0,
    streakExpiring: Number(streak?.data.state) === quest.StreakState.EXPIRING,
  };
}
