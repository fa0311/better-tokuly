import { Switch } from "@base-ui/react/switch";
import { cn } from "#/lib/cn";

/**
 * A bare on/off switch for dense layouts (a grid of settings), named by
 * `label` for assistive technology. A setting with its own line of text uses
 * `SwitchRow` from the design system instead; the two look the same.
 */
export function Toggle({
  label,
  checked,
  onChange,
  disabled,
  className,
}: {
  label: string;
  checked: boolean;
  onChange(checked: boolean): void;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <Switch.Root
      aria-label={label}
      checked={checked}
      onCheckedChange={onChange}
      disabled={disabled}
      className={cn(
        "relative h-8 w-14 shrink-0 rounded-full border-2 border-ink bg-surface-3 transition-colors data-[checked]:bg-lime data-[disabled]:opacity-50",
        className,
      )}
    >
      <Switch.Thumb className="block size-6 translate-x-0.5 rounded-full border-2 border-ink bg-surface transition-transform duration-200 ease-bounce data-[checked]:translate-x-[1.6rem]" />
    </Switch.Root>
  );
}
