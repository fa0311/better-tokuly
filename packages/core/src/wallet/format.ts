import type { wallet } from "@tokuly/api";

/** Digits after the decimal point of a currency's major unit (JPY 0, USD 2). */
function fractionDigits(currency: string): number {
  try {
    return (
      new Intl.NumberFormat("en", { style: "currency", currency }).resolvedOptions()
        .maximumFractionDigits ?? 2
    );
  } catch {
    return 2;
  }
}

/** Money given in the minor unit of an ISO 4217 currency: 1000 JPY → "¥1,000", 499 USD → "$4.99". */
export function formatMoney(minor: bigint, currency: string, language: string): string {
  const digits = fractionDigits(currency);
  const amount = Number(minor) / 10 ** digits;
  try {
    return new Intl.NumberFormat(language, { style: "currency", currency }).format(amount);
  } catch {
    // Not a currency code `Intl` knows how to write.
    return `${amount.toLocaleString(language, { minimumFractionDigits: digits, maximumFractionDigits: digits })} ${currency}`;
  }
}

/** A plain coin count with thousands separators, for running text. */
export function formatCoins(coins: bigint): string {
  return coins.toLocaleString("en-US");
}

/**
 * The context a domain attached to a ledger entry (`memo_json`), flattened to
 * text for display. Nested values are skipped; a broken memo is empty.
 */
export function parseMemo(memoJson: string): Record<string, string> {
  let parsed: unknown;
  try {
    parsed = JSON.parse(memoJson || "{}");
  } catch {
    return {};
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) return {};
  const memo: Record<string, string> = {};
  for (const [key, value] of Object.entries(parsed)) {
    if (typeof value === "string" || typeof value === "number" || typeof value === "boolean")
      memo[key] = String(value);
  }
  return memo;
}

/** Hours and minutes left until `untilMs`, rounded up to the minute so it never reads "0 minutes" early. */
export function countdown(untilMs: number, nowMs: number): { hours: number; minutes: number } {
  const minutes = Math.max(0, Math.ceil((untilMs - nowMs) / 60_000));
  return { hours: Math.floor(minutes / 60), minutes: minutes % 60 };
}

/** How many steps of the bonus ladder show as collected in the current round. */
export function collectedSteps(
  bonus: Pick<wallet.DailyBonus, "claimable" | "streak" | "nextCycleDay" | "cycle">,
): number {
  // Right after the last step the server already points at step 1 of the next
  // round; until tomorrow the finished round is what the person wants to see.
  if (!bonus.claimable && bonus.streak > 0 && bonus.nextCycleDay <= 1) return bonus.cycle.length;
  return Math.max(0, bonus.nextCycleDay - 1);
}
