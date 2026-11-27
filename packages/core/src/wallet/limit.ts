import { toDate, type wallet } from "@tokuly/api";
import { formatMoney } from "./format";

/**
 * The monthly spending limit as the apps read it. The server decides; these
 * only say what its answer means for a screen: what is left, whether a
 * package still fits, which own limits the person may pick.
 */

/** One-tap choices for the own limit, in yen. */
const PRESETS = [0n, 1000n, 3000n, 5000n, 10_000n, 30_000n];

/** The largest own limit the server accepts, in yen. */
export const MAX_OWN_LIMIT_YEN = 100_000_000n;

/** The choices the person may pick: under 18 none above the ceiling. */
export function limitPresets(limit: Pick<wallet.SpendingLimit, "ceilingYen">): bigint[] {
  const ceiling = limit.ceilingYen;
  return ceiling === undefined ? PRESETS : PRESETS.filter((yen) => yen <= ceiling);
}

/**
 * The yen a package counts for against the limit: its price when it is in
 * yen, otherwise its coins at ¥1 each (as the server counts it).
 */
export function packageYen(
  pkg: Pick<wallet.CoinPackage, "priceMinor" | "currency" | "coins">,
): bigint {
  return pkg.currency === "JPY" ? pkg.priceMinor : pkg.coins;
}

/** Whether spending `yen` more this month stays within the limit (no limit: always). */
export function fitsLimit(limit: wallet.SpendingLimit | null | undefined, yen: bigint): boolean {
  return limit?.remainingYen === undefined || yen <= limit.remainingYen;
}

/** The share of the limit used this month, 0 to 1; `null` without a limit. */
export function usedShare(
  limit: Pick<wallet.SpendingLimit, "limitYen" | "spentYen">,
): number | null {
  if (limit.limitYen === undefined) return null;
  if (limit.limitYen <= 0n) return 1;
  return Math.min(1, Number(limit.spentYen) / Number(limit.limitYen));
}

/**
 * The reason the server would refuse `yen` as the own limit (`null` clears
 * it), checked before asking; `null` when it would accept it.
 */
export function ownLimitProblem(
  limit: Pick<wallet.SpendingLimit, "ceilingYen">,
  yen: bigint | null,
): "SPENDING_LIMIT_INVALID" | "SPENDING_LIMIT_RESTRICTED" | null {
  if (yen !== null && (yen < 0n || yen > MAX_OWN_LIMIT_YEN)) return "SPENDING_LIMIT_INVALID";
  const ceiling = limit.ceilingYen;
  if (ceiling !== undefined && (yen === null || yen > ceiling)) return "SPENDING_LIMIT_RESTRICTED";
  return null;
}

/**
 * The day the month starts over, written for people ("12月1日", "Dec 1").
 * In Japan time: that is where the server's months turn.
 */
export function resetDay(limit: Pick<wallet.SpendingLimit, "resetsAt">, language: string): string {
  const resetsAt = toDate(limit.resetsAt);
  if (!resetsAt) return "";
  return new Intl.DateTimeFormat(language, {
    month: "short",
    day: "numeric",
    timeZone: "Asia/Tokyo",
  }).format(resetsAt);
}

/**
 * The line on what is left this month (「今月はあと ¥1,200 まで使えるよ」), as a
 * key of the `wallet` namespace with its values.
 */
export function limitSummary(
  limit: wallet.SpendingLimit,
  language: string,
): { key: "limit.none" | "limit.reached" | "limit.remaining"; values: Record<string, string> } {
  if (limit.remainingYen === undefined) return { key: "limit.none", values: {} };
  if (limit.remainingYen <= 0n)
    return { key: "limit.reached", values: { date: resetDay(limit, language) } };
  return {
    key: "limit.remaining",
    values: { amount: formatMoney(limit.remainingYen, "JPY", language) },
  };
}
