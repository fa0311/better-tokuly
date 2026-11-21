import { cn } from "#/lib/cn";

export function Spinner({ className }: { className?: string }) {
  return (
    <svg className={cn("size-5 animate-spin", className)} viewBox="0 0 24 24" fill="none" role="status" aria-label="loading">
      <circle cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="3" opacity="0.25" />
      <path d="M21 12a9 9 0 0 0-9-9" stroke="currentColor" strokeWidth="3" strokeLinecap="round" />
    </svg>
  );
}

/** Centered spinner for a whole pane that is loading. */
export function PaneSpinner() {
  return (
    <div className="grid h-full min-h-40 place-items-center text-ink-faint">
      <Spinner className="size-7" />
    </div>
  );
}
