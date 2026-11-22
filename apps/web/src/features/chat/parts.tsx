import { ContextMenu } from "@base-ui/react/context-menu";
import { Search, X } from "lucide-react";
import { type ReactElement, type ReactNode, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import type { MenuAction } from "#/components/ui/menu";
import { cn } from "#/lib/cn";

/** Small building blocks shared by the chat screens. */

export function SearchField({
  value,
  onChange,
  placeholder,
  autoFocus,
  className,
}: {
  value: string;
  onChange(value: string): void;
  placeholder: string;
  autoFocus?: boolean;
  className?: string;
}) {
  const { t } = useTranslation("chat");
  return (
    <label
      className={cn(
        "flex h-11 items-center gap-2 rounded-full border-2 border-line bg-surface px-4 focus-within:border-brand",
        className,
      )}
    >
      <Search className="size-[18px] shrink-0 text-ink-faint" />
      <input
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        aria-label={placeholder}
        // biome-ignore lint/a11y/noAutofocus: a search the person just opened is where they want to type.
        autoFocus={autoFocus}
        className="min-w-0 flex-1 bg-transparent text-[15px] outline-none placeholder:text-ink-faint"
        autoCapitalize="none"
        autoComplete="off"
        spellCheck={false}
        enterKeyHint="search"
      />
      {value && (
        <button
          type="button"
          aria-label={t("actions.clear")}
          onClick={() => onChange("")}
          className="grid size-6 shrink-0 place-items-center rounded-full text-ink-faint hover:bg-surface-2 hover:text-ink"
        >
          <X className="size-4" />
        </button>
      )}
    </label>
  );
}

/**
 * Gives `children` a menu of actions on right-click (and on long-press on
 * touch screens), looking like the `Menu` primitive. `children` must be one
 * element that forwards its props and ref.
 */
export function ContextActions({
  actions,
  quick,
  children,
}: {
  actions: MenuAction[];
  /** A row of one-tap choices above the actions (the quick reactions of a message). */
  quick?: {
    label: string;
    items: { key: string; label: string; content: ReactNode; onSelect(): void }[];
  };
  children: ReactElement<Record<string, unknown>>;
}) {
  if (actions.length === 0 && !quick) return children;
  return (
    <ContextMenu.Root>
      <ContextMenu.Trigger render={children} />
      <ContextMenu.Portal>
        <ContextMenu.Positioner className="z-50 outline-none">
          <ContextMenu.Popup className="sticker min-w-44 origin-[var(--transform-origin)] rounded-2xl bg-surface p-1.5 outline-none transition-[transform,opacity] duration-150 data-[ending-style]:scale-95 data-[ending-style]:opacity-0 data-[starting-style]:scale-95 data-[starting-style]:opacity-0">
            {quick && (
              <ContextMenu.Group
                aria-label={quick.label}
                className="mb-1 flex items-center gap-0.5 border-b-2 border-line pb-1.5"
              >
                {quick.items.map((item) => (
                  <ContextMenu.Item
                    key={item.key}
                    aria-label={item.label}
                    onClick={item.onSelect}
                    className="grid size-10 cursor-default select-none place-items-center rounded-xl text-2xl outline-none data-[highlighted]:bg-surface-2 [&>svg]:size-5"
                  >
                    {item.content}
                  </ContextMenu.Item>
                ))}
              </ContextMenu.Group>
            )}
            {actions.map((action) => (
              <ContextMenu.Item
                key={action.label}
                disabled={action.disabled}
                onClick={action.onSelect}
                className={cn(
                  "flex cursor-default select-none items-center gap-2.5 rounded-xl px-3 py-2.5 text-[15px] font-bold outline-none data-[disabled]:opacity-40 data-[highlighted]:bg-surface-2",
                  action.danger && "text-danger",
                )}
              >
                {action.icon && (
                  <span className="grid size-5 place-items-center [&>svg]:size-5">
                    {action.icon}
                  </span>
                )}
                {action.label}
              </ContextMenu.Item>
            ))}
          </ContextMenu.Popup>
        </ContextMenu.Positioner>
      </ContextMenu.Portal>
    </ContextMenu.Root>
  );
}

/** Heading of a group of rows. */
export function SectionTitle({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <h2 className={cn("px-3 pt-3 pb-1 text-xs font-black text-ink-faint", className)}>
      {children}
    </h2>
  );
}

/** Whether a CSS media query matches, kept current. */
export function useMediaQuery(query: string): boolean {
  return useSyncExternalStore(
    (notify) => {
      const list = window.matchMedia(query);
      list.addEventListener("change", notify);
      return () => list.removeEventListener("change", notify);
    },
    () => window.matchMedia(query).matches,
    () => false,
  );
}

/** Wide enough for the two-pane layout (the `md` breakpoint). */
export function useWide(): boolean {
  return useMediaQuery("(min-width: 768px)");
}

/** A pointer that can hover: quick actions appear next to the message instead of in a sheet. */
export function useHover(): boolean {
  return useMediaQuery("(hover: hover) and (pointer: fine)");
}
