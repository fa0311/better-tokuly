import { Link } from "@tanstack/react-router";
import type { common, school } from "@tokuly/api";
import { Check, Settings } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { useFriends } from "./api";

export type Section = "today" | "timetable" | "study" | "rooms" | "exams";

const SECTIONS: {
  id: Section;
  to: "/school" | "/school/timetable" | "/school/study" | "/school/rooms" | "/school/exams";
}[] = [
  { id: "today", to: "/school" },
  { id: "timetable", to: "/school/timetable" },
  { id: "study", to: "/school/study" },
  { id: "rooms", to: "/school/rooms" },
  { id: "exams", to: "/school/exams" },
];

/** A page of the school area: the shared header, the row of sections, the content. */
export function SchoolPage({
  section,
  title,
  back,
  actions,
  wide,
  children,
}: {
  section?: Section;
  title?: ReactNode;
  back?: string;
  actions?: ReactNode;
  wide?: boolean;
  children: ReactNode;
}) {
  const { t } = useTranslation("school");
  return (
    <Page
      title={title ?? t("title")}
      back={back}
      wide={wide}
      actions={
        <>
          {actions}
          <Link
            to="/school/settings"
            aria-label={t("settings.title")}
            className="grid size-9 shrink-0 place-items-center rounded-full hover:bg-surface-2"
          >
            <Settings className="size-5" />
          </Link>
        </>
      }
    >
      {section && (
        <nav
          aria-label={t("title")}
          className="scrollbar-none -mx-4 mb-4 flex gap-1.5 overflow-x-auto px-4"
        >
          {SECTIONS.map((item) => (
            <Link
              key={item.id}
              to={item.to}
              aria-current={item.id === section ? "page" : undefined}
              // The row scrolls on phones: the section in use is brought into view.
              ref={
                item.id === section
                  ? (element) => element?.scrollIntoView({ block: "nearest", inline: "center" })
                  : undefined
              }
              className={cn(
                "flex h-9 shrink-0 items-center rounded-full px-4 text-sm font-bold transition-colors",
                item.id === section
                  ? "sticker-sm bg-ink text-surface"
                  : "bg-surface-2 text-ink-soft hover:text-ink",
              )}
            >
              {t(`sections.${item.id}`)}
            </Link>
          ))}
        </nav>
      )}
      {children}
    </Page>
  );
}

/** A titled block of a page, with an optional control on the right of its heading. */
export function Block({
  title,
  action,
  children,
  className,
}: {
  title: ReactNode;
  action?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("mt-6 first:mt-0", className)}>
      <div className="mb-2 flex min-h-9 items-center gap-2 px-1">
        <h2 className="min-w-0 flex-1 truncate font-black">{title}</h2>
        {action}
      </div>
      {children}
    </section>
  );
}

/** Ink or white, whichever reads on a subject's colour. */
export function inkOn(color: string): string {
  const hex = /^#([0-9a-f]{6})$/i.exec(color)?.[1];
  if (!hex) return "#17122b";
  const [r, g, b] = [0, 2, 4].map((at) => Number.parseInt(hex.slice(at, at + 2), 16) / 255) as [
    number,
    number,
    number,
  ];
  return 0.299 * r + 0.587 * g + 0.114 * b > 0.6 ? "#17122b" : "#ffffff";
}

type SubjectLike = Pick<school.Subject, "name" | "emoji" | "color">;

/** A subject as a small coloured pill: its emoji and name on its colour. */
export function SubjectPill({
  subject,
  className,
}: {
  subject: SubjectLike | undefined;
  className?: string;
}) {
  const { t } = useTranslation("school");
  if (!subject)
    return (
      <span
        className={cn(
          "inline-flex h-6 items-center rounded-full bg-surface-2 px-2.5 text-xs font-bold text-ink-soft",
          className,
        )}
      >
        {t("subjects.none")}
      </span>
    );
  return (
    <span
      className={cn(
        "inline-flex h-6 max-w-full items-center gap-1 rounded-full border-2 border-ink px-2 text-xs font-black",
        className,
      )}
      style={{ background: subject.color, color: inkOn(subject.color) }}
    >
      {subject.emoji && <span aria-hidden>{subject.emoji}</span>}
      <span className="truncate">{subject.name}</span>
    </span>
  );
}

/** Ticks friends in a list. `only` limits the choice (for example to friends who share a timetable). */
export function FriendPicker({
  selected,
  onChange,
  only,
}: {
  selected: string[];
  onChange(userIds: string[]): void;
  only?: common.UserSummary[];
}) {
  const { t } = useTranslation("school");
  const friends = useFriends();
  const people =
    only ?? (friends.data?.friends ?? []).flatMap((friend) => (friend.user ? [friend.user] : []));
  if (!only && friends.isPending) return <Skeleton className="h-14" />;
  if (people.length === 0)
    return (
      <p className="rounded-2xl bg-surface-2 px-4 py-3 text-sm text-ink-soft">
        {t("friends.none")}
      </p>
    );
  const toggle = (userId: string) =>
    onChange(
      selected.includes(userId) ? selected.filter((id) => id !== userId) : [...selected, userId],
    );
  return (
    <ul className="-mx-1 flex max-h-64 flex-col overflow-y-auto">
      {people.map((user) => {
        const chosen = selected.includes(user.id);
        return (
          <li key={user.id}>
            <button
              type="button"
              aria-pressed={chosen}
              onClick={() => toggle(user.id)}
              className="flex w-full items-center gap-3 rounded-2xl px-2 py-2 text-left transition-colors hover:bg-surface-2"
            >
              <Avatar user={user} size="sm" />
              <span className="min-w-0 flex-1 truncate font-bold">
                {user.displayName || user.handle}
              </span>
              <span
                className={cn(
                  "grid size-6 place-items-center rounded-full border-2",
                  chosen ? "border-ink bg-lime text-[#17122b]" : "border-line",
                )}
              >
                {chosen && <Check className="size-4" />}
              </span>
            </button>
          </li>
        );
      })}
    </ul>
  );
}

/** A row of choices where exactly one is on (week patterns, timer modes, audiences). */
export function Segmented<T extends string | number>({
  value,
  onChange,
  options,
  label,
  className,
}: {
  value: T;
  onChange(value: T): void;
  options: { value: T; label: string }[];
  label: string;
  className?: string;
}) {
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className={cn("inline-flex max-w-full gap-1 rounded-full bg-surface-2 p-1", className)}
    >
      {options.map((option) => {
        const active = option.value === value;
        return (
          // biome-ignore lint/a11y/useSemanticElements: a segmented control of buttons, as the tabs are.
          <button
            key={String(option.value)}
            type="button"
            role="radio"
            aria-checked={active}
            onClick={() => onChange(option.value)}
            className={cn(
              "h-9 min-w-0 flex-1 truncate rounded-full px-3 text-sm font-bold transition-colors",
              active ? "sticker-sm bg-surface text-ink" : "text-ink-soft hover:text-ink",
            )}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}

/** A big number with a caption: study time, a streak, days left. */
export function Figure({
  value,
  label,
  className,
}: {
  value: ReactNode;
  label: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("min-w-0", className)}>
      <div className="truncate text-2xl font-black leading-tight">{value}</div>
      <div className="truncate text-xs font-bold text-ink-soft">{label}</div>
    </div>
  );
}

export function RowsSkeleton({
  rows = 3,
  className = "h-16",
}: {
  rows?: number;
  className?: string;
}) {
  return (
    <div className="flex flex-col gap-2">
      {Array.from({ length: rows }, (_, index) => (
        <Skeleton key={index} className={className} />
      ))}
    </div>
  );
}

export function LoadFailed() {
  const { t } = useTranslation("school");
  return <EmptyState emoji="🫠" title={t("loadFailed")} />;
}

/**
 * The current time, re-read every `intervalMs` while the caller is mounted.
 * Timers are drawn from this and the server's timestamps, never by counting
 * ticks, so a slowed-down background tab shows the right time when it is
 * looked at again.
 */
export function useNow(intervalMs = 1000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const update = () => setNow(Date.now());
    const interval = window.setInterval(update, intervalMs);
    document.addEventListener("visibilitychange", update);
    return () => {
      window.clearInterval(interval);
      document.removeEventListener("visibilitychange", update);
    };
  }, [intervalMs]);
  return now;
}
