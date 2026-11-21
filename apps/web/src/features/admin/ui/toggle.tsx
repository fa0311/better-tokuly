import { Switch as BaseSwitch } from "@base-ui/react/switch";

/** A bare on/off switch for table cells; `SwitchRow` is the labelled one for forms. */
export function Toggle({ label, checked, onChange, disabled }: { label: string; checked: boolean; onChange(checked: boolean): void; disabled?: boolean }) {
  return (
    <BaseSwitch.Root
      aria-label={label}
      checked={checked}
      onCheckedChange={onChange}
      disabled={disabled}
      className="relative inline-block h-7 w-12 shrink-0 align-middle rounded-full border-2 border-ink bg-surface-3 transition-colors data-[checked]:bg-lime data-[disabled]:opacity-50"
    >
      <BaseSwitch.Thumb className="block size-5 translate-x-0.5 rounded-full border-2 border-ink bg-surface transition-transform duration-200 ease-bounce data-[checked]:translate-x-[1.35rem]" />
    </BaseSwitch.Root>
  );
}
