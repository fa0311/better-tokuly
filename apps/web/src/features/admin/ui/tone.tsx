import type { ReactNode } from "react";
import { cn } from "#/lib/cn";

export type Tone = "neutral" | "good" | "warn" | "danger" | "info" | "brand";

const TONES: Record<Tone, string> = {
  neutral: "bg-surface-2 text-ink-soft",
  good: "bg-lime text-[#17122b]",
  warn: "bg-sun text-[#17122b]",
  danger: "bg-danger text-white",
  info: "bg-sky text-[#17122b]",
  brand: "bg-brand-soft text-brand-strong",
};

/** A state label. The text always says what the colour means. */
export function ToneChip({ tone = "neutral", icon, children, className }: { tone?: Tone; icon?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <span className={cn("inline-flex h-6 shrink-0 items-center gap-1 whitespace-nowrap rounded-full px-2.5 text-xs font-bold [&>svg]:size-3.5", TONES[tone], className)}>
      {icon}
      {children}
    </span>
  );
}
