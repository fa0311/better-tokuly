import { Dialog as BaseDialog } from "@base-ui/react/dialog";
import { X } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "#/lib/cn";

export interface DialogProps {
  open: boolean;
  onOpenChange(open: boolean): void;
  title: ReactNode;
  description?: ReactNode;
  children: ReactNode;
  /** Buttons row at the bottom. */
  footer?: ReactNode;
  className?: string;
}

/**
 * A modal. On phones it rises from the bottom as a sheet; on wider screens it
 * is a centered card.
 */
export function Dialog({
  open,
  onOpenChange,
  title,
  description,
  children,
  footer,
  className,
}: DialogProps) {
  return (
    <BaseDialog.Root open={open} onOpenChange={onOpenChange}>
      <BaseDialog.Portal>
        <BaseDialog.Backdrop className="fixed inset-0 z-50 bg-overlay transition-opacity duration-200 data-[ending-style]:opacity-0 data-[starting-style]:opacity-0" />
        <BaseDialog.Popup
          className={cn(
            "sticker-lg fixed inset-x-0 bottom-0 z-50 flex max-h-[92dvh] flex-col rounded-t-[2rem] bg-surface outline-none",
            "transition-all duration-300 ease-out-soft data-[ending-style]:translate-y-full data-[starting-style]:translate-y-full",
            "sm:inset-x-auto sm:top-1/2 sm:bottom-auto sm:left-1/2 sm:w-[min(32rem,calc(100vw-2rem))] sm:-translate-x-1/2 sm:-translate-y-1/2 sm:rounded-[2rem]",
            "sm:data-[ending-style]:-translate-y-[45%] sm:data-[ending-style]:opacity-0 sm:data-[starting-style]:-translate-y-[45%] sm:data-[starting-style]:opacity-0",
            className,
          )}
        >
          <header className="flex items-start gap-3 px-6 pt-6">
            <div className="min-w-0 flex-1">
              <BaseDialog.Title className="text-xl font-black leading-snug">
                {title}
              </BaseDialog.Title>
              {description && (
                <BaseDialog.Description className="mt-1 text-sm text-ink-soft">
                  {description}
                </BaseDialog.Description>
              )}
            </div>
            <BaseDialog.Close
              aria-label="close"
              className="grid size-9 shrink-0 place-items-center rounded-full hover:bg-surface-2"
            >
              <X className="size-5" />
            </BaseDialog.Close>
          </header>
          <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">{children}</div>
          {footer && <footer className="flex justify-end gap-2 px-6 pb-6 pb-safe">{footer}</footer>}
        </BaseDialog.Popup>
      </BaseDialog.Portal>
    </BaseDialog.Root>
  );
}
