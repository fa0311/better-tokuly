import { wallet } from "@tokuly/api";

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
export function payoutProblem(
  coins: bigint,
  available: bigint,
  minCoins: bigint,
): PayoutProblem | null {
  if (coins > available) return "aboveAvailable";
  if (coins < minCoins || coins <= 0n) return "belowMinimum";
  return null;
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

export function checkoutStep(
  purchase: Pick<wallet.Purchase, "status" | "checkoutUrl">,
): CheckoutStep {
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
    return parsed.protocol === "https:" || parsed.protocol === "http:"
      ? parsed.toString()
      : undefined;
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
