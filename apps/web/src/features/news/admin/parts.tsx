import { errorReason, type Timestamp, toDate } from "@tokuly/api";
import type { ReactNode } from "react";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import i18n from "#/lib/i18n";

/** A page inside the staff console: a compact sticky header over a full-width body. */
export function StaffPage({
  title,
  actions,
  children,
}: {
  title: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="h-full overflow-y-auto">
      <header className="glass sticky top-0 z-20 border-b-2 border-line">
        <div className="flex h-14 items-center gap-2 px-4">
          <h1 className="min-w-0 flex-1 truncate text-lg font-black">{title}</h1>
          {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
        </div>
      </header>
      <div className="px-4 pt-4 pb-32 md:pb-10">{children}</div>
    </div>
  );
}

/** A titled block of a staff page: quieter and denser than the app's sticker cards. */
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
    <section
      aria-label={typeof title === "string" ? title : undefined}
      className={cn("overflow-clip rounded-2xl border-2 border-line bg-surface", className)}
    >
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

export type Tone = "neutral" | "good" | "warn" | "danger" | "info";

const TONES: Record<Tone, string> = {
  neutral: "bg-surface-2 text-ink-soft",
  good: "bg-lime text-[#17122b]",
  warn: "bg-sun text-[#17122b]",
  danger: "bg-danger text-white",
  info: "bg-sky text-[#17122b]",
};

/** A state label; the text always says what the colour means. */
export function ToneChip({
  tone = "neutral",
  children,
  className,
}: {
  tone?: Tone;
  children: ReactNode;
  className?: string;
}) {
  return (
    <span
      className={cn(
        "inline-flex h-6 shrink-0 items-center gap-1 rounded-full px-2.5 text-xs font-bold whitespace-nowrap [&>svg]:size-3.5",
        TONES[tone],
        className,
      )}
    >
      {children}
    </span>
  );
}

/** "2026/11/22 14:05": the timestamp format of the console's tables. */
export function formatDateTime(timestamp: Timestamp | undefined): string {
  const date = toDate(timestamp);
  if (!date) return "—";
  return new Intl.DateTimeFormat(i18n.language, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

/**
 * The message for a failed staff action. Reasons of the news admin API are
 * looked up in this feature's strings first: other features use some of the
 * same reason names (`INVALID_NAME`, `INVALID_TAG`) for other things.
 */
export function staffErrorMessage(error: unknown): string {
  const reason = errorReason(error);
  if (reason && i18n.exists(`staff.errors.${reason}`, { ns: "news" }))
    return i18n.t(`staff.errors.${reason}`, { ns: "news" });
  return errorMessage(error);
}
