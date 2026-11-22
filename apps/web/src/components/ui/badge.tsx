import { cn } from "#/lib/cn";

/** Unread-style counter bubble; renders nothing at zero and caps at 99+. */
export function CountBadge({ count, className }: { count: number; className?: string }) {
  if (count <= 0) return null;
  return (
    <span
      className={cn(
        "inline-grid h-5 min-w-5 place-items-center rounded-full border-2 border-surface bg-pink px-1 text-[11px] font-black leading-none text-white",
        className,
      )}
    >
      {count > 99 ? "99+" : count}
    </span>
  );
}
