import { cn } from "#/lib/cn";

/** A row of choices where one is on: the order of a grid, the range of a chart. */
export function ChoiceChips<T extends string | number>({
  options,
  value,
  onChange,
  label,
  className,
}: {
  options: readonly { value: T; label: string }[];
  value: T;
  onChange(value: T): void;
  /** What is being chosen, for screen readers. */
  label: string;
  className?: string;
}) {
  return (
    <fieldset className={cn("scrollbar-none flex gap-2 overflow-x-auto", className)}>
      <legend className="sr-only">{label}</legend>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          aria-pressed={value === option.value}
          onClick={() => onChange(option.value)}
          className={cn(
            "h-9 shrink-0 rounded-full border-2 px-3.5 text-sm font-bold",
            value === option.value
              ? "border-ink bg-ink text-bg"
              : "border-line bg-surface text-ink-soft hover:text-ink",
          )}
        >
          {option.label}
        </button>
      ))}
    </fieldset>
  );
}
