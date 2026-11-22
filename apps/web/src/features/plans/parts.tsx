import type { common } from "@tokuly/api";
import { type ReactNode, useEffect, useState } from "react";
import { Avatar } from "#/components/ui/avatar";
import { cn } from "#/lib/cn";

/** A heading inside a page of this area, with an optional control on the right. */
export function SectionTitle({ children, action, className }: { children: ReactNode; action?: ReactNode; className?: string }) {
  return (
    <div className={cn("mb-2 flex min-h-9 items-center gap-2 px-1", className)}>
      <h2 className="min-w-0 flex-1 truncate text-base font-black">{children}</h2>
      {action}
    </div>
  );
}

/** A few faces overlapping, and how many more there are. */
export function AvatarStack({ users, max = 4, total }: { users: common.UserSummary[]; max?: number; total?: number }) {
  const shown = users.slice(0, max);
  const more = (total ?? users.length) - shown.length;
  return (
    <span className="flex items-center">
      <span className="flex -space-x-2">
        {shown.map((user) => (
          <Avatar key={user.id} user={user} size="xs" className="rounded-[0.5rem] ring-2 ring-surface" />
        ))}
      </span>
      {more > 0 && <span className="ml-1.5 text-xs font-black text-ink-soft">+{more}</span>}
    </span>
  );
}

/** The current time, refreshed every `intervalMs`, for texts like "あと2時間". */
export function useNow(intervalMs = 30_000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), intervalMs);
    return () => window.clearInterval(timer);
  }, [intervalMs]);
  return now;
}

/** A choice among a few options, as chips that wrap; the chosen one is inked. */
export function ChoiceChips<T extends string | number>({
  options,
  value,
  onChange,
  label,
  className,
}: {
  options: { value: T; label: ReactNode }[];
  value: T;
  onChange(value: T): void;
  /** Names the group for assistive technology. */
  label: string;
  className?: string;
}) {
  return (
    <div role="radiogroup" aria-label={label} className={cn("flex flex-wrap gap-2", className)}>
      {options.map((option) => {
        const active = option.value === value;
        return (
          // biome-ignore lint/a11y/useSemanticElements: a chip group, not a form field.
          <button
            key={String(option.value)}
            type="button"
            role="radio"
            aria-checked={active}
            onClick={() => onChange(option.value)}
            className={cn(
              "inline-flex h-10 items-center gap-1.5 rounded-full border-2 px-4 text-sm font-bold transition-colors",
              active ? "border-ink bg-brand text-on-brand" : "border-line bg-surface text-ink-soft hover:border-ink-faint",
            )}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}
