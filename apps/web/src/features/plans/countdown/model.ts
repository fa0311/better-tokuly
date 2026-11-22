import { type common, plans } from "@tokuly/api";
import type { TFunction } from "i18next";

/** Card colours for countdowns and counters, picked from the ID so each keeps its own. */
const TINTS = [
  "bg-lime",
  "bg-sun",
  "bg-sky",
  "bg-pink text-white",
  "bg-mint",
  "bg-coral text-white",
] as const;

export function tintOf(id: string): string {
  let hash = 0;
  for (let i = 0; i < id.length; i++) hash = (hash * 31 + id.charCodeAt(i)) >>> 0;
  const tint = TINTS[hash % TINTS.length]!;
  return tint.includes("text-") ? tint : `${tint} text-[#17122b]`;
}

/** The server's text in the language on show. */
export function localized(text: common.LocalizedText | undefined, language: string): string {
  if (!text) return "";
  return (language.startsWith("ja") ? text.ja : text.en) || text.ja || text.en;
}

/** The parts of a countdown's big line: a lead-in, the number, and a unit ("あと" "33" "日"). */
export function countdownParts(
  daysLeft: number,
  t: TFunction<"plans">,
): { lead: string; number: string; unit: string } {
  if (daysLeft === 0) return { lead: "", number: t("countdown.today"), unit: "" };
  if (daysLeft > 0)
    return {
      lead: t("countdown.leftLead"),
      number: String(daysLeft),
      unit: daysLeft === 1 ? t("countdown.leftUnitOne") : t("countdown.leftUnit"),
    };
  return {
    lead: "",
    number: String(-daysLeft),
    unit: daysLeft === -1 ? t("countdown.agoUnitOne") : t("countdown.agoUnit"),
  };
}

/** "今日！", "あした", "あと3日" for a day that lies ahead. */
export function inDaysText(days: number, t: TFunction<"plans">): string {
  return days <= 0
    ? t("countdown.today")
    : days === 1
      ? t("plan.tomorrow")
      : t("plan.inDays", { count: days });
}

/** "ともだち記念日" / "カップル記念日", or the name the two gave it. */
export function anniversaryTitle(anniversary: plans.Anniversary, t: TFunction<"plans">): string {
  return (
    anniversary.title ||
    (anniversary.kind === plans.AnniversaryKind.COUPLE ? t("anniv.couple") : t("anniv.friendship"))
  );
}

export function anniversaryEmoji(anniversary: plans.Anniversary): string {
  return anniversary.emoji || (anniversary.kind === plans.AnniversaryKind.COUPLE ? "💞" : "🤝");
}

/** Starting points for a new countdown: an emoji and a title to build on. */
export const COUNTDOWN_PRESETS = [
  { emoji: "📝", key: "exam" },
  { emoji: "🏮", key: "festival" },
  { emoji: "🎤", key: "concert" },
  { emoji: "🎂", key: "birthday" },
  { emoji: "✈️", key: "trip" },
  { emoji: "🎓", key: "graduation" },
] as const;
