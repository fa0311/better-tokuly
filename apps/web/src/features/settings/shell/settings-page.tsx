import type { ReactNode } from "react";
import { Page } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { useIsWide } from "./use-is-wide";

/**
 * A settings screen. On phones it leads back to the section list; on wide
 * screens that list is already next to it, so only sub-pages (`back` given)
 * show a way back.
 */
export function SettingsPage({
  title,
  back,
  actions,
  children,
}: {
  title: ReactNode /** Parent page of a sub-page; sections omit it. */;
  back?: string;
  actions?: ReactNode;
  children: ReactNode;
}) {
  const wide = useIsWide();
  return (
    <Page title={title} back={back ?? (wide ? undefined : "/settings")} actions={actions}>
      {children}
    </Page>
  );
}

/** A titled block of related settings inside a settings screen. */
export function Group({
  title,
  lead,
  children,
  className,
  flush,
}: {
  title: ReactNode;
  lead?: ReactNode;
  children: ReactNode;
  className?: string /** Rows reach the card's edge (lists); otherwise the card pads its content (forms). */;
  flush?: boolean;
}) {
  return (
    <section className={cn("mt-7 first:mt-0", className)}>
      <h2 className="px-1 text-lg font-black">{title}</h2>
      {lead && <p className="mt-0.5 px-1 text-sm leading-relaxed text-ink-soft">{lead}</p>}
      <div className={cn("sticker mt-3 rounded-card bg-surface", flush ? "p-2" : "p-4")}>
        {children}
      </div>
    </section>
  );
}

const TONES = {
  good: "bg-lime text-[#17122b]",
  warn: "bg-sun text-[#17122b]",
  off: "bg-surface-2 text-ink-soft",
} as const;

/** A small state label: on / off, verified / not, this device. The text carries the meaning, the colour only backs it up. */
export function StateChip({
  tone,
  icon,
  children,
}: {
  tone: keyof typeof TONES;
  icon?: ReactNode;
  children: ReactNode;
}) {
  return (
    <span
      className={cn(
        "inline-flex h-6 shrink-0 items-center gap-1 rounded-full px-2.5 text-xs font-black [&>svg]:size-3.5",
        TONES[tone],
      )}
    >
      {icon}
      {children}
    </span>
  );
}
