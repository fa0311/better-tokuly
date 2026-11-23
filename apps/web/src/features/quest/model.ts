import { quest, toMillis } from "@tokuly/api";
import { untilNextDay } from "#/lib/account-time";

/** How far a bar is filled, 0–1, for counts the API sends as 64-bit integers. */
export function fraction(done: bigint | number, of: bigint | number): number {
  const total = Number(of);
  if (!(total > 0)) return 0;
  return Math.min(1, Math.max(0, Number(done) / total));
}

/** How far into the current level a player is, 0–1. */
export const levelFraction = (level: quest.LevelProgress | undefined): number =>
  level ? fraction(level.xpIntoLevel, level.xpForNextLevel) : 0;

/** XP still missing for the next level. */
export const xpToNextLevel = (level: quest.LevelProgress | undefined): number =>
  level ? Math.max(0, Number(level.xpForNextLevel - level.xpIntoLevel)) : 0;

/** The key of a tier's name in the strings (`tiers.<key>`); `undefined` for a badge that stands alone. */
export function tierKey(tier: quest.BadgeTier): "bronze" | "silver" | "gold" | undefined {
  switch (tier) {
    case quest.BadgeTier.BRONZE:
      return "bronze";
    case quest.BadgeTier.SILVER:
      return "silver";
    case quest.BadgeTier.GOLD:
      return "gold";
    default:
      return undefined;
  }
}

export type RarityKey = "common" | "uncommon" | "rare" | "epic" | "legendary";

/** The key of a rarity's name in the strings (`rarity.<key>`); `undefined` while it is not known. */
export function rarityKey(rarity: quest.BadgeRarity): RarityKey | undefined {
  switch (rarity) {
    case quest.BadgeRarity.COMMON:
      return "common";
    case quest.BadgeRarity.UNCOMMON:
      return "uncommon";
    case quest.BadgeRarity.RARE:
      return "rare";
    case quest.BadgeRarity.EPIC:
      return "epic";
    case quest.BadgeRarity.LEGENDARY:
      return "legendary";
    default:
      return undefined;
  }
}

/** A share of players as a percentage that stays readable however small: "42%", "3.2%", "0.4%". */
export function formatShare(share: number): string {
  const percent = share * 100;
  if (percent >= 10) return `${Math.round(percent)}%`;
  if (percent >= 0.1) return `${percent.toFixed(1)}%`;
  return "<0.1%";
}

/** A secret badge the viewer has not earned: it has no name, no emoji and a stand-in key. */
export const isHiddenSecret = (badge: quest.Badge): boolean => badge.secret && !badge.owned;

/**
 * The badges of a family as rows of the collection: tiers of one achievement
 * (a "line") side by side, badges that stand alone one each. Order follows
 * the catalogue.
 */
export function badgeLines(badges: quest.Badge[]): quest.Badge[][] {
  const lines = new Map<string, quest.Badge[]>();
  for (const badge of badges) {
    // Unearned secret badges have no line: each is its own.
    const key = badge.line || badge.key;
    const line = lines.get(key);
    if (line) line.push(badge);
    else lines.set(key, [badge]);
  }
  return [...lines.values()].map((line) => [...line].sort((a, b) => a.tier - b.tier));
}

/** The tier of a line a player works towards: the first they do not own (the last when they own all). */
export function nextInLine(line: quest.Badge[]): quest.Badge | undefined {
  return line.find((badge) => !badge.owned) ?? line.at(-1);
}

/** What to say about a streak right now. */
export type StreakStatus =
  /** Nothing is running. */
  | "none"
  /** It ended within the last day and can be restored. */
  | "lost"
  /** It ends within hours unless continued. */
  | "expiring"
  /** Today's part of the signed-in player is still missing. */
  | "mine"
  /** The friend's is. */
  | "theirs"
  | "both"
  /** Today is complete (what is open is tomorrow's). */
  | "done";

/**
 * Reads a streak as the signed-in player needs it at `nowMs`. The day that
 * is open may already be tomorrow (both did their part today): then nobody
 * has to do anything yet, whatever `waiting_on` says about that next day.
 */
export function streakStatus(streak: quest.Streak, nowMs: number, zone: string): StreakStatus {
  switch (streak.state) {
    case quest.StreakState.LOST:
      return "lost";
    case quest.StreakState.EXPIRING:
      return "expiring";
    case quest.StreakState.ACTIVE:
      break;
    default:
      return "none";
  }
  const expires = streak.expiresAt ? toMillis(streak.expiresAt) : 0;
  // A minute of slack: the deadline is a midnight, and so is the end of today.
  if (expires > nowMs + untilNextDay(nowMs, zone) + 60_000) return "done";
  switch (streak.waitingOn) {
    case quest.StreakTurn.MINE:
      return "mine";
    case quest.StreakTurn.THEIRS:
      return "theirs";
    case quest.StreakTurn.BOTH:
      return "both";
    default:
      return "done";
  }
}

/** A streak that is running (and so worth a flame next to a name). */
export const streakRuns = (streak: quest.Streak | undefined): streak is quest.Streak =>
  Boolean(
    streak &&
      streak.days > 0 &&
      (streak.state === quest.StreakState.ACTIVE || streak.state === quest.StreakState.EXPIRING),
  );

/** A span of time in its two largest sensible units, for "あと…" lines. */
export interface Remaining {
  unit: "days" | "hours" | "minutes";
  count: number;
}

/** How long is left until `untilMs`, rounded the way a countdown is read: "あと2日", "あと5時間", "あと12分". */
export function remaining(untilMs: number, nowMs: number): Remaining {
  const minutes = Math.max(0, Math.ceil((untilMs - nowMs) / 60_000));
  if (minutes >= 48 * 60) return { unit: "days", count: Math.floor(minutes / (24 * 60)) };
  if (minutes >= 60) return { unit: "hours", count: Math.floor(minutes / 60) };
  return { unit: "minutes", count: minutes };
}

/** A `YYYY-MM-DD` date of the API in words ("11月23日", "Nov 23"), without going through the device's zone. */
export function formatDay(date: string, language: string, withYear = false): string {
  const [year = 1970, month = 1, day = 1] = date.split("-").map(Number);
  return new Intl.DateTimeFormat(language, {
    month: "short",
    day: "numeric",
    year: withYear ? "numeric" : undefined,
    timeZone: "UTC",
  }).format(new Date(Date.UTC(year, month - 1, day)));
}

/** The missions of a response that can be claimed right now: today's, this week's and older ones still waiting. */
export function claimable(missions: quest.ListMissionsResponse | undefined): quest.Mission[] {
  if (!missions) return [];
  return [...missions.daily, ...(missions.weekly ? [missions.weekly] : []), ...missions.unclaimed]
    .filter((mission) => mission.state === quest.MissionState.COMPLETED)
    .filter((mission, index, all) => all.findIndex((other) => other.id === mission.id) === index);
}

/**
 * How loud a celebration is, from the ドパガキ度 factor (0 with motion off):
 * - `toast`: words only;
 * - `card`: a card pops in over the page, with a little confetti;
 * - `full`: the whole screen lights up.
 */
export type Loudness = "toast" | "card" | "full";

export function loudness(factor: number): Loudness {
  if (factor >= 1.3) return "full";
  if (factor >= 0.75) return "card";
  return "toast";
}

/**
 * Whether restoring a lost streak can be confirmed right now:
 * - `ready`: confirm;
 * - `checking`: the balance is not known yet;
 * - `short`: the balance does not cover the cost (the dialog offers to buy coins);
 * - `used`: this month's restore is gone;
 * - `over`: the 24 hours to restore it have passed.
 */
export type RestoreState = "ready" | "checking" | "short" | "used" | "over";

export function restoreState({
  cost,
  coins,
  available,
  restorableUntilMs,
  nowMs,
}: {
  cost: number;
  /** The wallet balance; `undefined` while it loads. */
  coins: bigint | undefined;
  /** `RestoreOffer.available`. */
  available: boolean;
  restorableUntilMs: number;
  nowMs: number;
}): RestoreState {
  if (!available) return "used";
  if (restorableUntilMs <= nowMs) return "over";
  if (coins === undefined) return "checking";
  return coins < BigInt(cost) ? "short" : "ready";
}

/** What a failed restore leads to: buying coins when there were too few, otherwise just the reason. */
export const restoreFailure = (reason: string | undefined): "buy" | "explain" =>
  reason === "INSUFFICIENT_COINS" ? "buy" : "explain";

/**
 * Adds a reveal at the end. The first one is on screen and always stays;
 * past the limit the oldest of those still waiting make room.
 */
export function enqueue<T>(queue: T[], item: T, limit: number): T[] {
  const next = [...queue, item];
  if (next.length <= limit) return next;
  const [shown, ...waiting] = next;
  return shown === undefined ? next : [shown, ...waiting.slice(-(limit - 1))];
}
