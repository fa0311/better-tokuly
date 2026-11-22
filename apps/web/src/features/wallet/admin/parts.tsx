import { Link } from "@tanstack/react-router";
import type { common } from "@tokuly/api";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { cn } from "#/lib/cn";

/** A titled block of the staff page: quieter and denser than the app's sticker cards. */
export function Pane({
  title,
  aside,
  children,
  className,
}: {
  title?: ReactNode;
  aside?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("overflow-clip rounded-2xl border-2 border-line bg-surface", className)}>
      {(title || aside) && (
        <header className="flex min-h-11 items-center gap-2 border-b-2 border-line px-4 py-2">
          <h2 className="min-w-0 flex-1 truncate text-sm font-black">{title}</h2>
          {aside}
        </header>
      )}
      {children}
    </section>
  );
}

/** A row of mutually exclusive filter choices. */
export function FilterChips<T extends string | number>({
  label,
  value,
  onChange,
  options,
}: {
  label: string;
  value: T;
  onChange(value: T): void;
  options: { value: T; label: string }[];
}) {
  return (
    <fieldset className="scrollbar-none flex gap-1.5 overflow-x-auto" aria-label={label}>
      {options.map((option) => {
        const active = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            aria-pressed={active}
            onClick={() => onChange(option.value)}
            className={cn(
              "h-8 shrink-0 rounded-full border-2 px-3 text-xs font-black whitespace-nowrap transition-colors",
              active
                ? "border-ink bg-ink text-surface"
                : "border-line bg-surface text-ink-soft hover:border-ink-faint hover:text-ink",
            )}
          >
            {option.label}
          </button>
        );
      })}
    </fieldset>
  );
}

/** A person as staff sees them: face, name and handle, linked to their profile. Accounts that were erased say so. */
export function UserLine({
  user,
  className,
}: {
  user: common.UserSummary | undefined;
  className?: string;
}) {
  const { t } = useTranslation("wallet");
  if (!user || user.deleted) {
    return (
      <span className={cn("flex min-w-0 items-center gap-2.5", className)}>
        <Avatar user={user} size="sm" />
        <span className="truncate text-sm font-bold text-ink-soft">{t("staff.deletedUser")}</span>
      </span>
    );
  }
  return (
    <Link
      to="/u/$handle"
      params={{ handle: user.handle }}
      className={cn("flex min-w-0 items-center gap-2.5", className)}
    >
      <Avatar user={user} size="sm" />
      <span className="min-w-0">
        <span className="block truncate text-sm font-black">{user.displayName}</span>
        <span className="block truncate text-xs text-ink-soft">@{user.handle}</span>
      </span>
    </Link>
  );
}

/** Header cells of a records table (shown on wide screens, where rows line up in columns). */
export function ColumnHeads({
  columns,
  heads,
}: {
  /** `grid-template-columns` shared with the rows. */ columns: string;
  heads: string[];
}) {
  return (
    <div
      className="sticky top-14 z-10 hidden gap-3 border-b-2 border-line bg-surface-2 px-4 py-2.5 text-xs font-black text-ink-soft lg:grid"
      style={{ gridTemplateColumns: columns }}
      aria-hidden
    >
      {heads.map((head) => (
        <span key={head} className="truncate">
          {head}
        </span>
      ))}
    </div>
  );
}

/** Classes of a record row: a card of wrapping label–value pairs on narrow screens, a table row (grid) on wide ones. */
export const RECORD_ROW =
  "flex flex-wrap items-baseline gap-x-4 gap-y-1.5 px-4 py-3 text-sm lg:grid lg:items-center lg:gap-3 lg:py-2.5";

/** One value of a record: "label value" inside the card on narrow screens, a bare cell of the table on wide ones. */
export function Cell({
  label,
  children,
  full,
  className,
}: {
  label: string;
  children: ReactNode /** Takes a line of its own in the card (titles, long text). */;
  full?: boolean;
  className?: string;
}) {
  return (
    <div
      className={cn("flex min-w-0 items-baseline gap-1.5 lg:block", full && "w-full", className)}
    >
      <span className="shrink-0 text-xs font-bold text-ink-faint lg:sr-only">{label}</span>
      <span className="min-w-0 break-words">{children}</span>
    </div>
  );
}
