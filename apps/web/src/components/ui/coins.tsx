import { cn } from "#/lib/cn";

const SIZES = {
  sm: { text: "text-sm", coin: "size-4 text-[9px]" },
  md: { text: "text-base", coin: "size-5 text-[11px]" },
  lg: { text: "text-3xl", coin: "size-8 text-base" },
} as const;

/** The coin glyph used wherever a price or balance is shown. */
export function CoinIcon({ className }: { className?: string }) {
  return (
    <span aria-hidden className={cn("inline-grid shrink-0 place-items-center rounded-full border-2 border-ink bg-sun font-black leading-none text-[#17122b]", className)}>
      T
    </span>
  );
}

/** An amount of Tokuly coins (the API sends them as 64-bit integers). */
export function Coins({ amount, size = "md", signed, className }: { amount: bigint | number; size?: keyof typeof SIZES; /** Prefix gains with "+" (ledger rows). */ signed?: boolean; className?: string }) {
  const value = typeof amount === "bigint" ? amount : BigInt(Math.trunc(amount));
  const text = `${signed && value > 0n ? "+" : ""}${value.toLocaleString("en-US")}`;
  return (
    <span className={cn("inline-flex items-center gap-1 font-black tabular-nums", SIZES[size].text, className)}>
      <CoinIcon className={SIZES[size].coin} />
      {text}
    </span>
  );
}
