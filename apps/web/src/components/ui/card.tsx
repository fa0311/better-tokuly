import type { HTMLAttributes } from "react";
import { cn } from "#/lib/cn";

/** A sticker-outlined panel. Use for standalone blocks, not for list rows. */
export function Card({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("sticker rounded-card bg-surface p-5", className)} {...props} />;
}

/** A quiet panel without the outline, for grouping inside pages. */
export function Panel({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("rounded-card bg-surface p-4", className)} {...props} />;
}

/** Small rounded label: counts, states, tags. */
export function Chip({ className, ...props }: HTMLAttributes<HTMLSpanElement>) {
  return (
    <span
      className={cn(
        "inline-flex h-6 items-center gap-1 rounded-full bg-surface-2 px-2.5 text-xs font-bold text-ink-soft",
        className,
      )}
      {...props}
    />
  );
}
