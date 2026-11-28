import { type ReactNode, useEffect, useState } from "react";
import { ConfirmDialog } from "#/components/ui/confirm";
import { TextArea } from "#/components/ui/field";
import { useTranslation } from "#/lib/use-translation";

/** Limits of the API for the texts staff attach to an action. */
export const REASON_MAX = 500;
export const NOTE_MAX = 2000;

export interface ActionTexts {
  /** Shown to the affected user. */
  reason: string;
  /** Internal, for the logs. */
  note: string;
}

/**
 * The confirmation step of every staff action: says what is about to happen
 * and collects the reason for the user and/or the internal note the API takes
 * for it.
 */
export function ActionDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  danger,
  loading,
  requireReason,
  note = true,
  noteLabel,
  validate,
  children,
  onConfirm,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  title: ReactNode;
  description?: ReactNode;
  confirmLabel: string;
  danger?: boolean;
  loading?: boolean;
  /** Ask for the text shown to the user; the action cannot be confirmed without it. */
  requireReason?: boolean;
  /** Ask for an internal note (default on). */
  note?: boolean;
  noteLabel?: string;
  /** Checks the caller's own fields (`children`); returning false keeps the dialog open. */
  validate?(): boolean;
  /** Extra fields between the description and the texts (a duration, a switch). */
  children?: ReactNode;
  onConfirm(texts: ActionTexts): void;
}) {
  const { t } = useTranslation("admin");
  const [reasonText, setReasonText] = useState("");
  const [noteText, setNoteText] = useState("");
  const [missing, setMissing] = useState(false);

  // Every opening starts from empty fields.
  useEffect(() => {
    if (!open) return;
    setReasonText("");
    setNoteText("");
    setMissing(false);
  }, [open]);

  const confirm = () => {
    const ownFieldsOk = validate?.() ?? true;
    if (requireReason && !reasonText.trim()) {
      setMissing(true);
      return;
    }
    if (!ownFieldsOk) return;
    onConfirm({ reason: reasonText.trim(), note: noteText.trim() });
  };

  return (
    <ConfirmDialog
      open={open}
      onOpenChange={onOpenChange}
      title={title}
      description={description}
      confirmLabel={confirmLabel}
      danger={danger}
      loading={loading}
      onConfirm={confirm}
    >
      <div className="flex flex-col gap-4">
        {children}
        {requireReason && (
          <TextArea
            label={t("common.reason")}
            hint={t("common.reasonHint")}
            error={missing ? t("errors:REASON_REQUIRED") : undefined}
            value={reasonText}
            maxLength={REASON_MAX}
            onChange={(event) => {
              setReasonText(event.target.value);
              setMissing(false);
            }}
          />
        )}
        {note && (
          <TextArea
            label={noteLabel ?? t("common.note")}
            hint={t("common.noteHint")}
            value={noteText}
            maxLength={NOTE_MAX}
            onChange={(event) => setNoteText(event.target.value)}
          />
        )}
      </div>
    </ConfirmDialog>
  );
}
