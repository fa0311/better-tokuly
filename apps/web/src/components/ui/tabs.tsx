import { cn } from "#/lib/cn";

export interface TabItem<T extends string> {
  value: T;
  label: string;
  /** Small count shown after the label. */
  badge?: number;
}

/** A pill-shaped segmented control for switching views within a page. */
export function Tabs<T extends string>({
  items,
  value,
  onChange,
  className,
}: {
  items: TabItem<T>[];
  value: T;
  onChange(value: T): void;
  className?: string;
}) {
  return (
    <div
      role="tablist"
      className={cn(
        "scrollbar-none inline-flex max-w-full gap-1 overflow-x-auto rounded-full bg-surface-2 p-1",
        className,
      )}
    >
      {items.map((item) => {
        const active = item.value === value;
        return (
          <button
            key={item.value}
            type="button"
            role="tab"
            aria-selected={active}
            onClick={() => onChange(item.value)}
            className={cn(
              "flex h-9 shrink-0 items-center gap-1.5 rounded-full px-4 text-sm font-bold transition-colors",
              active ? "sticker-sm bg-surface text-ink" : "text-ink-soft hover:text-ink",
            )}
          >
            {item.label}
            {item.badge ? (
              <span className="rounded-full bg-pink px-1.5 text-[11px] font-black text-white">
                {item.badge}
              </span>
            ) : null}
          </button>
        );
      })}
    </div>
  );
}
