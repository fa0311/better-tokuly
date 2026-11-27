import { admin, common } from "@tokuly/api";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { TextArea } from "#/components/ui/field";
import { SwitchRow } from "#/components/ui/switch";
import { cn } from "#/lib/cn";
import { actionKey } from "../labels";
import { NOTE_MAX, REASON_MAX } from "../ui/action-dialog";
import { Section } from "../ui/console-page";
import { DurationSelect, useDurationLabel } from "../ui/duration";

/** What `ResolveCase` is asked to do. */
export interface Resolution {
  action: admin.ModerationActionKind;
  removeContent: boolean;
  reason: string;
  note: string;
  suspensionSeconds: number;
}

const ACTIONS = [
  admin.ModerationActionKind.DISMISS,
  admin.ModerationActionKind.REMOVE_CONTENT,
  admin.ModerationActionKind.WARN,
  admin.ModerationActionKind.SUSPEND,
  admin.ModerationActionKind.BAN,
] as const;

type ResolveAction = (typeof ACTIONS)[number];

const SANCTIONS: readonly ResolveAction[] = [
  admin.ModerationActionKind.WARN,
  admin.ModerationActionKind.SUSPEND,
  admin.ModerationActionKind.BAN,
];

/**
 * Closes an open case: pick what to do, say why, confirm. Actions the API
 * would refuse for this case (taking an account "down", sanctioning an owner
 * who is gone) are not offered.
 */
export function ResolvePanel({
  moderationCase,
  loading,
  onResolve,
}: {
  moderationCase: admin.ModerationCase;
  loading: boolean;
  onResolve(resolution: Resolution): void;
}) {
  const { t } = useTranslation("admin");
  const durationLabel = useDurationLabel();
  const [action, setAction] = useState<ResolveAction | null>(null);
  const [removeContent, setRemoveContent] = useState(false);
  const [seconds, setSeconds] = useState(7 * 86_400);
  const [reason, setReason] = useState("");
  const [note, setNote] = useState("");
  const [reasonMissing, setReasonMissing] = useState(false);
  const [confirming, setConfirming] = useState(false);

  const aboutAccount = moderationCase.subject?.type === common.SubjectType.USER;
  const ownerGone = !moderationCase.subjectOwner || moderationCase.subjectOwner.deleted;
  const unavailable = (candidate: ResolveAction): string | undefined => {
    if (candidate === admin.ModerationActionKind.REMOVE_CONTENT && aboutAccount)
      return t("moderation.resolve.accountNotRemovable");
    if (SANCTIONS.includes(candidate) && ownerGone) return t("moderation.resolve.ownerGone");
    return undefined;
  };

  const isSanction = action !== null && SANCTIONS.includes(action);
  const needsReason = action !== null && action !== admin.ModerationActionKind.DISMISS;
  const removes =
    action === admin.ModerationActionKind.REMOVE_CONTENT ||
    (isSanction && removeContent && !aboutAccount);

  const review = () => {
    if (action === null) return;
    if (needsReason && !reason.trim()) {
      setReasonMissing(true);
      return;
    }
    setConfirming(true);
  };

  const submit = () => {
    if (action === null) return;
    onResolve({
      action,
      removeContent: isSanction && removeContent && !aboutAccount,
      reason: needsReason ? reason.trim() : "",
      note: note.trim(),
      suspensionSeconds: action === admin.ModerationActionKind.SUSPEND ? seconds : 0,
    });
    setConfirming(false);
  };

  return (
    <Section title={t("moderation.resolve.title")}>
      <div className="flex flex-col gap-4 p-4">
        <div
          role="radiogroup"
          aria-label={t("moderation.resolve.title")}
          className="flex flex-col gap-1.5"
        >
          {ACTIONS.map((candidate) => {
            const blocked = unavailable(candidate);
            const selected = action === candidate;
            return (
              // biome-ignore lint/a11y/useSemanticElements: a radio that carries a description line.
              <button
                key={candidate}
                type="button"
                role="radio"
                aria-checked={selected}
                disabled={Boolean(blocked)}
                onClick={() => {
                  setAction(candidate);
                  setReasonMissing(false);
                }}
                className={cn(
                  "flex items-start gap-3 rounded-xl border-2 px-3 py-2 text-left transition-colors disabled:opacity-50",
                  selected ? "border-brand bg-brand-soft" : "border-line hover:border-ink-faint",
                )}
              >
                <span
                  className={cn(
                    "mt-1 grid size-4 shrink-0 place-items-center rounded-full border-2",
                    selected ? "border-brand" : "border-ink-faint",
                  )}
                >
                  {selected && <span className="size-2 rounded-full bg-brand" />}
                </span>
                <span className="min-w-0">
                  <span className="block text-sm font-bold">{t(actionKey(candidate))}</span>
                  <span className="block text-xs text-ink-soft">
                    {blocked ??
                      t(`moderation.resolve.hints.${admin.ModerationActionKind[candidate]}`)}
                  </span>
                </span>
              </button>
            );
          })}
        </div>

        {action === admin.ModerationActionKind.SUSPEND && (
          <DurationSelect value={seconds} onChange={setSeconds} />
        )}
        {isSanction && !aboutAccount && (
          <SwitchRow
            label={t("moderation.resolve.alsoRemove")}
            hint={t("moderation.resolve.alsoRemoveHint")}
            checked={removeContent}
            onChange={setRemoveContent}
          />
        )}
        {needsReason && (
          <TextArea
            label={t("common.reason")}
            hint={t("common.reasonHint")}
            error={reasonMissing ? t("errors:REASON_REQUIRED") : undefined}
            value={reason}
            maxLength={REASON_MAX}
            onChange={(event) => {
              setReason(event.target.value);
              setReasonMissing(false);
            }}
          />
        )}
        {action !== null && (
          <TextArea
            label={t("common.note")}
            hint={t("common.noteHint")}
            value={note}
            maxLength={NOTE_MAX}
            onChange={(event) => setNote(event.target.value)}
          />
        )}

        <Button
          block
          variant={
            action === null || action === admin.ModerationActionKind.DISMISS ? "primary" : "danger"
          }
          disabled={action === null}
          loading={loading}
          onClick={review}
        >
          {t("moderation.resolve.submit")}
        </Button>
      </div>

      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t("moderation.resolve.confirmTitle")}
        confirmLabel={t("moderation.resolve.confirm")}
        danger={action !== admin.ModerationActionKind.DISMISS}
        loading={loading}
        onConfirm={submit}
      >
        {action !== null && (
          <ul className="flex list-disc flex-col gap-1.5 pl-5 text-sm">
            <li className="font-bold">
              {t(actionKey(action))}
              {action === admin.ModerationActionKind.SUSPEND && ` (${durationLabel(seconds)})`}
            </li>
            {removes && <li>{t("moderation.resolve.effects.removes")}</li>}
            {isSanction && moderationCase.subjectOwner && (
              <li>
                {t("moderation.resolve.effects.owner", {
                  handle: moderationCase.subjectOwner.handle,
                })}
              </li>
            )}
            <li>
              {t("moderation.resolve.effects.reporters", { count: moderationCase.reportCount })}
            </li>
          </ul>
        )}
      </ConfirmDialog>
    </Section>
  );
}
