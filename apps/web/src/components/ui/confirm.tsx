import type { ReactNode } from "react";
import { useTranslation } from "#/lib/use-translation";
import { Button } from "./button";
import { Dialog } from "./dialog";

/**
 * Asks before doing something that cannot be undone (or costs coins).
 * The caller owns `open` and closes it when `onConfirm` has finished.
 */
export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  danger,
  loading,
  confirmDisabled,
  onConfirm,
  children,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  title: ReactNode;
  description?: ReactNode;
  confirmLabel: string;
  /** Styles the confirm button as destructive. */
  danger?: boolean;
  loading?: boolean;
  /** Keeps the confirm button from being pressed (say why in `children`). */
  confirmDisabled?: boolean;
  onConfirm(): void;
  /** Extra content between the description and the buttons (a summary, a reason field). */
  children?: ReactNode;
}) {
  const { t } = useTranslation("common");
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={title}
      description={description}
      footer={
        <>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={loading}>
            {t("actions.cancel")}
          </Button>
          <Button
            variant={danger ? "danger" : "primary"}
            onClick={onConfirm}
            loading={loading}
            disabled={confirmDisabled}
          >
            {confirmLabel}
          </Button>
        </>
      }
    >
      {children}
    </Dialog>
  );
}
