import { Menu as BaseMenu } from "@base-ui/react/menu";
import type { ReactElement, ReactNode } from "react";
import { cn } from "#/lib/cn";

export interface MenuAction {
  label: string;
  icon?: ReactNode;
  /** Destructive actions are shown in the danger colour. */
  danger?: boolean;
  disabled?: boolean;
  onSelect(): void;
}

/**
 * A dropdown of actions hanging off a trigger (usually a "more" icon button).
 * `trigger` is the element that opens it; it must forward its props and ref,
 * which `Button` and plain DOM elements do.
 */
export function Menu({ trigger, actions, align = "end" }: { trigger: ReactElement<Record<string, unknown>>; actions: MenuAction[]; align?: "start" | "center" | "end" }) {
  return (
    <BaseMenu.Root>
      <BaseMenu.Trigger render={trigger} />
      <BaseMenu.Portal>
        <BaseMenu.Positioner align={align} sideOffset={8} className="z-50 outline-none">
          <BaseMenu.Popup
            className={cn(
              "sticker min-w-44 origin-[var(--transform-origin)] rounded-2xl bg-surface p-1.5 outline-none",
              "transition-[transform,opacity] duration-150 data-[ending-style]:scale-95 data-[ending-style]:opacity-0 data-[starting-style]:scale-95 data-[starting-style]:opacity-0",
            )}
          >
            {actions.map((action) => (
              <BaseMenu.Item
                key={action.label}
                disabled={action.disabled}
                onClick={action.onSelect}
                className={cn(
                  "flex cursor-default select-none items-center gap-2.5 rounded-xl px-3 py-2.5 text-[15px] font-bold outline-none",
                  "data-[highlighted]:bg-surface-2 data-[disabled]:opacity-40",
                  action.danger && "text-danger",
                )}
              >
                {action.icon && <span className="grid size-5 place-items-center [&>svg]:size-5">{action.icon}</span>}
                {action.label}
              </BaseMenu.Item>
            ))}
          </BaseMenu.Popup>
        </BaseMenu.Positioner>
      </BaseMenu.Portal>
    </BaseMenu.Root>
  );
}
