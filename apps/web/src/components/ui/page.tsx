import { Link } from "@tanstack/react-router";
import { ChevronLeft } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "#/lib/cn";

/**
 * Standard scrolling page inside the app shell: a sticky header and a
 * centered column. Use for every non-map screen so they line up.
 */
export function Page({ title, back, actions, children, wide }: { title: ReactNode; back?: string; actions?: ReactNode; children: ReactNode; wide?: boolean }) {
  return (
    <div className="h-full overflow-y-auto">
      <header className="glass sticky top-0 z-20 border-b-2 border-line">
        <div className={cn("mx-auto flex h-16 items-center gap-3 px-4", wide ? "max-w-6xl" : "max-w-2xl")}>
          {back && (
            <Link to={back} aria-label="back" className="grid size-9 place-items-center rounded-full hover:bg-surface-2">
              <ChevronLeft className="size-6" />
            </Link>
          )}
          <h1 className="min-w-0 flex-1 truncate text-xl font-black">{title}</h1>
          {actions}
        </div>
      </header>
      <div className={cn("mx-auto px-4 pt-4 pb-32 md:pb-10", wide ? "max-w-6xl" : "max-w-2xl")}>{children}</div>
    </div>
  );
}

/** Friendly placeholder for lists with nothing in them. */
export function EmptyState({ emoji, title, body, action }: { emoji: string; title: string; body?: string; action?: ReactNode }) {
  return (
    <div className="flex flex-col items-center gap-2 px-6 py-14 text-center">
      <div className="animate-float text-6xl">{emoji}</div>
      <p className="mt-2 text-lg font-black">{title}</p>
      {body && <p className="max-w-xs text-sm leading-relaxed text-ink-soft">{body}</p>}
      {action && <div className="mt-3">{action}</div>}
    </div>
  );
}

/** A tappable row in a list: leading visual, text, trailing controls. */
export function ListRow({ leading, title, subtitle, trailing, onClick, className }: { leading?: ReactNode; title: ReactNode; subtitle?: ReactNode; trailing?: ReactNode; onClick?(): void; className?: string }) {
  const body = (
    <>
      {leading}
      <div className="min-w-0 flex-1 text-left">
        <div className="truncate font-bold">{title}</div>
        {subtitle && <div className="truncate text-sm text-ink-soft">{subtitle}</div>}
      </div>
      {trailing}
    </>
  );
  const classes = cn("flex w-full items-center gap-3 rounded-2xl px-3 py-2.5", onClick && "transition-colors hover:bg-surface-2", className);
  return onClick ? (
    <button type="button" onClick={onClick} className={classes}>
      {body}
    </button>
  ) : (
    <div className={classes}>{body}</div>
  );
}

/** Grey shimmer block shown while content loads. */
export function Skeleton({ className }: { className?: string }) {
  return <div className={cn("animate-pulse rounded-2xl bg-surface-3", className)} />;
}
