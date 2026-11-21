import { wallet } from "@tokuly/api";

/** Digits after the decimal point of a currency's major unit (JPY 0, USD 2). */
function fractionDigits(currency: string): number {
  try {
    return new Intl.NumberFormat("en", { style: "currency", currency }).resolvedOptions().maximumFractionDigits ?? 2;
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
    // Not a currency code the browser knows how to write.
    return `${amount.toLocaleString(language, { minimumFractionDigits: digits, maximumFractionDigits: digits })} ${currency}`;
  }
}

/** A plain coin count with thousands separators, for running text. */
export function formatCoins(coins: bigint): string {
  return coins.toLocaleString("en-US");
}

/** What a payout of `coins` pays at the current rate, in the minor unit of the payout currency. */
export function payoutMoney(coins: bigint, minorPer100Coins: bigint): bigint {
  return (coins * minorPer100Coins) / 100n;
}

/**
 * Reads a coin amount typed by a person: digits, optionally full-width (the
 * Japanese keyboard's default) and with thousands separators. `undefined`
 * for anything that is not a whole number.
 */
export function parseCoins(text: string): bigint | undefined {
  const digits = text
    .trim()
    .replace(/[０-９]/g, (digit) => String.fromCharCode(digit.charCodeAt(0) - 0xfee0))
    .replace(/[,，\s]/g, "");
  return /^\d{1,15}$/.test(digits) ? BigInt(digits) : undefined;
}

export type PayoutProblem = "belowMinimum" | "aboveAvailable";

/** Why a payout of `coins` cannot be requested, or `null` when it can. */
export function payoutProblem(coins: bigint, available: bigint, minCoins: bigint): PayoutProblem | null {
  if (coins > available) return "aboveAvailable";
  if (coins < minCoins || coins <= 0n) return "belowMinimum";
  return null;
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
    if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") memo[key] = String(value);
  }
  return memo;
}

/** Splits a newest-first list into runs that happened on the same local day. */
export function groupByDay<T>(items: readonly T[], dateOf: (item: T) => Date | undefined): { day: string; date: Date | undefined; items: T[] }[] {
  const groups: { day: string; date: Date | undefined; items: T[] }[] = [];
  for (const item of items) {
    const date = dateOf(item);
    const day = date ? `${date.getFullYear()}-${date.getMonth() + 1}-${date.getDate()}` : "";
    const last = groups[groups.length - 1];
    if (last && last.day === day) last.items.push(item);
    else groups.push({ day, date, items: [item] });
  }
  return groups;
}

/** Whole days between the local day of `date` and the local day of `now` (0 = today, 1 = yesterday). */
export function daysAgo(date: Date, now: Date): number {
  const start = (value: Date) => new Date(value.getFullYear(), value.getMonth(), value.getDate()).getTime();
  return Math.round((start(now) - start(date)) / 86_400_000);
}

/** Hours and minutes left until `untilMs`, rounded up to the minute so it never reads "0 minutes" early. */
export function countdown(untilMs: number, nowMs: number): { hours: number; minutes: number } {
  const minutes = Math.max(0, Math.ceil((untilMs - nowMs) / 60_000));
  return { hours: Math.floor(minutes / 60), minutes: minutes % 60 };
}

/** How many steps of the bonus ladder show as collected in the current round. */
export function collectedSteps(bonus: Pick<wallet.DailyBonus, "claimable" | "streak" | "nextCycleDay" | "cycle">): number {
  // Right after the last step the server already points at step 1 of the next
  // round; until tomorrow the finished round is what the person wants to see.
  if (!bonus.claimable && bonus.streak > 0 && bonus.nextCycleDay <= 1) return bonus.cycle.length;
  return Math.max(0, bonus.nextCycleDay - 1);
}

/** Where a purchase stands, as the next thing the buyer can do about it. */
export type CheckoutStep =
  /** Paid; the coins are in the wallet. */
  | "completed"
  /** Waiting for a payment on the provider's hosted page (`checkout_url`). */
  | "redirect"
  /** Waiting for a payment the provider collects without a hosted page (an embedded form, an in-app purchase). */
  | "waiting"
  | "failed"
  | "cancelled"
  | "expired";

export function checkoutStep(purchase: Pick<wallet.Purchase, "status" | "checkoutUrl">): CheckoutStep {
  switch (purchase.status) {
    case wallet.PurchaseStatus.COMPLETED:
      return "completed";
    case wallet.PurchaseStatus.FAILED:
      return "failed";
    case wallet.PurchaseStatus.CANCELLED:
      return "cancelled";
    case wallet.PurchaseStatus.EXPIRED:
      return "expired";
    default:
      return purchase.checkoutUrl ? "redirect" : "waiting";
  }
}

/** Only web addresses may be opened as a checkout page (never `javascript:` and the like). */
export function safeCheckoutUrl(url: string): string | undefined {
  try {
    const parsed = new URL(url);
    return parsed.protocol === "https:" || parsed.protocol === "http:" ? parsed.toString() : undefined;
  } catch {
    return undefined;
  }
}

/** The top of a value axis: the next "clean" number at or above `value`, whose half is clean too (0 / 500 / 1,000). */
export function niceCeiling(value: number): number {
  if (value <= 2) return 2;
  if (value <= 4) return 4;
  if (value <= 10) return 10;
  const magnitude = 10 ** Math.floor(Math.log10(value));
  for (const step of [1, 2, 4, 5, 10]) {
    if (value <= step * magnitude) return step * magnitude;
  }
  return 10 * magnitude;
}
