import { cn } from "#/lib/cn";

/** A chunky progress bar: how far along towards the next level. `value` is 0..1. */
export function Meter({ value, label, className, tint = "bg-lime" }: { value: number; label: string; className?: string; tint?: string }) {
  const percent = Math.round(Math.min(1, Math.max(0, value)) * 100);
  return (
    <div role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent} className={cn("h-3.5 overflow-hidden rounded-full border-2 border-ink bg-surface-3", className)}>
      <div className={cn("h-full rounded-full transition-[width] duration-700 ease-out-soft", tint)} style={{ width: `${Math.max(percent, value > 0 ? 4 : 0)}%` }} />
    </div>
  );
}

/** The flag of a country code as an emoji ("JP" → 🇯🇵); a globe when there is none. */
export function flagEmoji(countryCode: string): string {
  if (!/^[A-Za-z]{2}$/.test(countryCode)) return "🌏";
  return String.fromCodePoint(...[...countryCode.toUpperCase()].map((letter) => 0x1f1e6 + letter.charCodeAt(0) - 65));
}
