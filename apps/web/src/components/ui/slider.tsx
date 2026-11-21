import { Slider as BaseSlider } from "@base-ui/react/slider";
import { cn } from "#/lib/cn";

/**
 * A single-thumb slider for picking a number in a range (a level, a zoom).
 * `onChange` fires while dragging; `onCommit` once the value is let go, which
 * is the moment to save it.
 */
export function Slider({
  value,
  onChange,
  onCommit,
  min = 0,
  max = 100,
  step = 1,
  label,
  valueText,
  disabled,
  className,
}: {
  value: number;
  onChange(value: number): void;
  onCommit?(value: number): void;
  min?: number;
  max?: number;
  step?: number;
  /** Accessible name of the thumb. */
  label: string;
  /** What a screen reader announces for the current value (defaults to the number). */
  valueText?: string;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <BaseSlider.Root
      value={value}
      onValueChange={(next) => onChange(Array.isArray(next) ? (next[0] ?? min) : next)}
      onValueCommitted={(next) => onCommit?.(Array.isArray(next) ? (next[0] ?? min) : next)}
      min={min}
      max={max}
      step={step}
      disabled={disabled}
      thumbAlignment="edge"
      className={cn("w-full", className)}
    >
      <BaseSlider.Control className="flex h-11 w-full touch-none select-none items-center data-[disabled]:opacity-50">
        <BaseSlider.Track className="h-4 w-full rounded-full border-2 border-ink bg-surface-3">
          <BaseSlider.Indicator className="rounded-full bg-lime" />
          <BaseSlider.Thumb
            aria-label={label}
            getAriaValueText={valueText === undefined ? undefined : () => valueText}
            className="size-8 rounded-full border-[2.5px] border-ink bg-surface shadow-pop-sm transition-transform duration-150 ease-bounce has-[:focus-visible]:outline-[3px] has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-brand data-[dragging]:scale-110"
          />
        </BaseSlider.Track>
      </BaseSlider.Control>
    </BaseSlider.Root>
  );
}
