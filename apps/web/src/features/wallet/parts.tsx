import { createLink } from "@tanstack/react-router";
import type { AnchorHTMLAttributes, ReactNode } from "react";
import { cn } from "#/lib/cn";

const LINK_VARIANTS = {
  primary: "sticker bg-brand text-on-brand",
  lime: "sticker bg-lime text-[#17122b]",
  secondary: "sticker bg-surface text-ink",
  ghost: "text-ink hover:bg-surface-2 active:bg-surface-3",
} as const;

const LINK_SIZES = {
  sm: "h-9 px-4 text-sm",
  md: "h-11 px-5 text-[15px]",
  lg: "h-14 px-7 text-lg",
} as const;

interface AnchorButtonProps extends AnchorHTMLAttributes<HTMLAnchorElement> {
  variant?: keyof typeof LINK_VARIANTS;
  size?: keyof typeof LINK_SIZES;
  block?: boolean;
  icon?: ReactNode;
}

function AnchorButton({ variant = "primary", size = "md", block, icon, className, children, ...props }: AnchorButtonProps) {
  return (
    <a
      className={cn(
        "pressable inline-flex select-none items-center justify-center gap-2 whitespace-nowrap rounded-full font-bold",
        LINK_VARIANTS[variant],
        LINK_SIZES[size],
        block && "w-full",
        className,
      )}
      {...props}
    >
      {icon}
      {children}
    </a>
  );
}

/** A router link that looks like a `Button` (navigation is a link, not a click handler). */
export const LinkButton = createLink(AnchorButton);

/** Heading of a block inside a wallet page, with an optional control on the right. */
export function SectionTitle({ children, action, className }: { children: ReactNode; action?: ReactNode; className?: string }) {
  return (
    <div className={cn("flex min-h-9 items-center justify-between gap-3 px-1", className)}>
      <h2 className="text-sm font-black text-ink-soft">{children}</h2>
      {action}
    </div>
  );
}

const TONES = {
  neutral: "bg-surface-2 text-ink-soft",
  wait: "bg-sun text-[#17122b]",
  good: "bg-lime text-[#17122b]",
  info: "bg-sky text-[#17122b]",
  bad: "bg-danger text-white",
} as const;

export type Tone = keyof typeof TONES;

/** A state label (pending, paid, rejected…). The word carries the meaning; the colour only helps scanning. */
export function StatusChip({ tone, children, className }: { tone: Tone; children: ReactNode; className?: string }) {
  return <span className={cn("inline-flex h-6 shrink-0 items-center rounded-full px-2.5 text-xs font-black", TONES[tone], className)}>{children}</span>;
}

/** One "label: value" line of a details block. `stack` puts the label above the value, for long text (addresses, messages, IDs). */
export function Fact({ label, children, className, stack }: { label: ReactNode; children: ReactNode; className?: string; stack?: boolean }) {
  return (
    <div className={cn("py-2", stack ? "flex flex-col gap-0.5" : "flex items-baseline justify-between gap-4", className)}>
      <dt className="shrink-0 text-sm font-bold text-ink-soft">{label}</dt>
      <dd className={cn("min-w-0 font-bold break-words", !stack && "text-right")}>{children}</dd>
    </div>
  );
}

/** A big number with a caption under it, for summaries (one group of a `<dl>`). */
export function Stat({ label, children, className }: { label: ReactNode; children: ReactNode; className?: string }) {
  return (
    <div className={cn("flex min-w-0 flex-col-reverse", className)}>
      <dt className="truncate text-xs font-bold text-ink-faint">{label}</dt>
      <dd className="truncate text-lg font-black tabular-nums">{children}</dd>
    </div>
  );
}
