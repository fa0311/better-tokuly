/**
 * Money in the cost split: an integer in the currency's minor unit (yen for
 * JPY, cents for USD), as the API stores it.
 */

/** The currencies offered when a plan is created. Any ISO 4217 code the server accepts is displayed. */
export const CURRENCIES = ["JPY", "USD", "EUR", "KRW", "TWD"] as const;

/** How many decimals a currency is written with (0 for yen, 2 for dollars). */
export function minorDigits(currency: string): number {
  try {
    return (
      new Intl.NumberFormat("en", { style: "currency", currency }).resolvedOptions()
        .maximumFractionDigits ?? 2
    );
  } catch {
    return 2;
  }
}

/** "1,200円" (yen, in Japanese) / "¥1,200" / "$12.00". */
export function formatMoney(amount: bigint, currency: string, language: string): string {
  const digits = minorDigits(currency);
  const value = Number(amount) / 10 ** digits;
  // The way a receipt among friends is written.
  if (currency === "JPY" && language.startsWith("ja")) return `${value.toLocaleString("ja")}円`;
  try {
    return new Intl.NumberFormat(language, {
      style: "currency",
      currency,
      currencyDisplay: "narrowSymbol",
    }).format(value);
  } catch {
    return `${value.toLocaleString(language, { minimumFractionDigits: digits, maximumFractionDigits: digits })} ${currency}`;
  }
}

/** What someone typed ("1,200", "12.5", "１２００") as minor units; `null` when it is not a positive amount. */
export function parseMoney(text: string, currency: string): bigint | null {
  const digits = minorDigits(currency);
  const normalized = text
    .trim()
    .replace(/[０-９]/g, (char) => String.fromCharCode(char.charCodeAt(0) - 0xfee0))
    .replace(/[,，\s]/g, "")
    .replace(/．/g, ".");
  const match = /^(\d+)(?:\.(\d+))?$/.exec(normalized);
  if (!match) return null;
  const fraction = match[2] ?? "";
  if (fraction.length > digits) return null;
  const amount =
    BigInt(match[1]!) * 10n ** BigInt(digits) + BigInt(fraction.padEnd(digits, "0") || "0");
  return amount > 0n ? amount : null;
}

/** Minor units as the text of an amount field ("1200", "12.50"). */
export function moneyInputValue(amount: bigint, currency: string): string {
  const digits = minorDigits(currency);
  if (digits === 0) return amount.toString();
  const text = amount.toString().padStart(digits + 1, "0");
  return `${text.slice(0, -digits)}.${text.slice(-digits)}`;
}
