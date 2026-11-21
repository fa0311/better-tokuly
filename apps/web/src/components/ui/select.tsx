import { Select as BaseSelect } from "@base-ui/react/select";
import { Check, ChevronDown } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "#/lib/cn";

export interface SelectOption<T extends string> {
  value: T;
  label: string;
  /** Second line explaining the choice. */
  hint?: string;
}

/** A labelled single-choice dropdown for settings with more than a few options. */
export function SelectField<T extends string>({
  label,
  hint,
  value,
  onChange,
  options,
  disabled,
  className,
}: {
  label?: ReactNode;
  hint?: ReactNode;
  value: T;
  onChange(value: T): void;
  options: SelectOption<T>[];
  disabled?: boolean;
  className?: string;
}) {
  const items = options.map((option) => ({ value: option.value, label: option.label }));
  return (
    <div className={cn("flex flex-col gap-1.5", className)}>
      <BaseSelect.Root items={items} value={value} onValueChange={(next) => next !== null && onChange(next as T)} disabled={disabled}>
        {label && <BaseSelect.Label className="px-1 text-sm font-bold">{label}</BaseSelect.Label>}
        <BaseSelect.Trigger className="flex h-12 w-full items-center gap-2 rounded-2xl border-2 border-line bg-surface px-4 text-left font-bold outline-none transition-colors hover:border-ink-faint focus-visible:border-brand data-[popup-open]:border-brand data-[disabled]:opacity-50">
          <BaseSelect.Value className="min-w-0 flex-1 truncate" />
          <BaseSelect.Icon>
            <ChevronDown className="size-5 text-ink-soft" />
          </BaseSelect.Icon>
        </BaseSelect.Trigger>
        <BaseSelect.Portal>
          <BaseSelect.Positioner sideOffset={6} alignItemWithTrigger={false} className="z-50 outline-none">
            <BaseSelect.Popup className="sticker max-h-[min(24rem,var(--available-height))] min-w-[var(--anchor-width)] overflow-y-auto rounded-2xl bg-surface p-1.5 outline-none transition-opacity duration-150 data-[ending-style]:opacity-0 data-[starting-style]:opacity-0">
              {options.map((option) => (
                <BaseSelect.Item
                  key={option.value}
                  value={option.value}
                  className="flex cursor-default select-none items-center gap-2 rounded-xl px-3 py-2.5 outline-none data-[highlighted]:bg-surface-2"
                >
                  <span className="min-w-0 flex-1">
                    <BaseSelect.ItemText className="block font-bold">{option.label}</BaseSelect.ItemText>
                    {option.hint && <span className="block text-sm text-ink-soft">{option.hint}</span>}
                  </span>
                  <BaseSelect.ItemIndicator>
                    <Check className="size-5 text-brand" />
                  </BaseSelect.ItemIndicator>
                </BaseSelect.Item>
              ))}
            </BaseSelect.Popup>
          </BaseSelect.Positioner>
        </BaseSelect.Portal>
      </BaseSelect.Root>
      {hint && <p className="px-1 text-sm text-ink-soft">{hint}</p>}
    </div>
  );
}
