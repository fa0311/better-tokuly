import { Switch as BaseSwitch } from "@base-ui/react/switch";
import type { ReactNode } from "react";

/** A labelled on/off setting row. */
export function SwitchRow({ label, hint, checked, onChange, disabled }: { label: ReactNode; hint?: ReactNode; checked: boolean; onChange(checked: boolean): void; disabled?: boolean }) {
  return (
    // biome-ignore lint/a11y/noLabelWithoutControl: the switch renders the control inside this label.
    <label className="flex items-center gap-4 rounded-2xl px-3 py-3 hover:bg-surface-2">
      <span className="min-w-0 flex-1">
        <span className="block font-bold">{label}</span>
        {hint && <span className="block text-sm text-ink-soft">{hint}</span>}
      </span>
      <BaseSwitch.Root
        checked={checked}
        onCheckedChange={onChange}
        disabled={disabled}
        className="relative h-8 w-14 shrink-0 rounded-full border-2 border-ink bg-surface-3 transition-colors data-[checked]:bg-lime data-[disabled]:opacity-50"
      >
        <BaseSwitch.Thumb className="block size-6 translate-x-0.5 rounded-full border-2 border-ink bg-surface transition-transform duration-200 ease-bounce data-[checked]:translate-x-[1.6rem]" />
      </BaseSwitch.Root>
    </label>
  );
}
