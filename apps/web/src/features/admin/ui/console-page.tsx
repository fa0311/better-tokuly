import { Link } from "@tanstack/react-router";
import { ChevronLeft } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "#/lib/cn";

/**
 * A page of the staff console: a compact sticky header and a full-width body.
 * Denser than the app's `Page`, and it never caps the width — tables and
 * split views use whatever the screen offers.
 */
export function ConsolePage({
  title,
  back,
  backSearch,
  actions,
  children,
}: {
  title: ReactNode;
  /** Path of the page one level up; shows a back arrow. */
  back?: string;
  /** Search parameters for the back link (filters to return to). */
  backSearch?: object;
  actions?: ReactNode;
  children: ReactNode;
}) {
  const { t } = useTranslation("common");
  return (
    <div className="h-full overflow-y-auto">
      <header className="glass sticky top-0 z-20 border-b-2 border-line">
        <div className="flex h-14 items-center gap-2 px-4">
          {back && (
            <Link
              to={back}
              search={backSearch as never}
              aria-label={t("actions.back")}
              className="-ml-2 grid size-9 shrink-0 place-items-center rounded-full hover:bg-surface-2"
            >
              <ChevronLeft className="size-6" />
            </Link>
          )}
          <h1 className="min-w-0 flex-1 truncate text-lg font-black">{title}</h1>
          {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
        </div>
      </header>
      <div className="px-4 pt-4 pb-32 md:pb-10">{children}</div>
    </div>
  );
}

/** A titled block inside a console page; a region named after its title (or `label`) for assistive technology. */
export function Section({
  title,
  label,
  aside,
  children,
  className,
}: {
  title?: ReactNode /** Name of the region when there is no text title. */;
  label?: string;
  aside?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section
      aria-label={label ?? (typeof title === "string" ? title : undefined)}
      className={cn("rounded-2xl border-2 border-line bg-surface", className)}
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

/** A label–value line for the fact sheets of cases and accounts. */
export function Fact({ label, children }: { label: ReactNode; children: ReactNode }) {
  return (
    <div className="flex items-baseline gap-3 px-4 py-2 text-sm">
      <dt className="w-28 shrink-0 text-xs font-bold text-ink-faint">{label}</dt>
      <dd className="min-w-0 flex-1 break-words">{children}</dd>
    </div>
  );
}
