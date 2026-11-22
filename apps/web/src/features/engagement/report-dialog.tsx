import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { engagement } from "@tokuly/api";
import { Flag } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextArea } from "#/components/ui/field";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { engagementKeys, reportClient } from "./api";
import type { Subject } from "./model";

/** The longest report text the server takes. */
const TEXT_MAX = 1000;

/** The categories in the order they are offered, each with the emoji that leads its row. */
export const REPORT_CATEGORIES: { category: engagement.ReportCategory; key: string; emoji: string }[] = [
  { category: engagement.ReportCategory.SPAM, key: "SPAM", emoji: "📮" },
  { category: engagement.ReportCategory.HARASSMENT, key: "HARASSMENT", emoji: "😣" },
  { category: engagement.ReportCategory.HATE, key: "HATE", emoji: "🚫" },
  { category: engagement.ReportCategory.SEXUAL, key: "SEXUAL", emoji: "🔞" },
  { category: engagement.ReportCategory.VIOLENCE, key: "VIOLENCE", emoji: "💢" },
  { category: engagement.ReportCategory.SELF_HARM, key: "SELF_HARM", emoji: "🫂" },
  { category: engagement.ReportCategory.ILLEGAL, key: "ILLEGAL", emoji: "⚖️" },
  { category: engagement.ReportCategory.IMPERSONATION, key: "IMPERSONATION", emoji: "🎭" },
  { category: engagement.ReportCategory.COPYRIGHT, key: "COPYRIGHT", emoji: "©️" },
  { category: engagement.ReportCategory.OTHER, key: "OTHER", emoji: "💬" },
];

/** Key of a category's name in the `engagement` strings: `report.categories.<key>`. */
export function reportCategoryKey(category: engagement.ReportCategory): string {
  return REPORT_CATEGORIES.find((entry) => entry.category === category)?.key ?? "OTHER";
}

export interface ReportDialogProps {
  /** What is reported: content, a comment, or a user (`SubjectType.USER` with the UUID or the handle). */
  subject: Subject;
  open: boolean;
  onOpenChange(open: boolean): void;
  /** What the person is reporting, in words: "@handle のノート". Shown under the title. */
  label?: string;
}

/**
 * Reports a subject to the moderation team: pick what is wrong, optionally
 * say more. The report then shows up under "my reports" with its status.
 */
export function ReportDialog({ subject, open, onOpenChange, label }: ReportDialogProps) {
  const { t } = useTranslation("engagement");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [category, setCategory] = useState<engagement.ReportCategory>();
  const [text, setText] = useState("");

  const close = () => {
    onOpenChange(false);
    setCategory(undefined);
    setText("");
  };
  const send = useMutation({
    mutationFn: () => reportClient().createReport({ subject, category, text: text.trim() }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: engagementKeys.reports });
      toast.success(t("report.sent"), { action: { label: t("report.viewMine"), onClick: () => void navigate({ to: "/activity", search: { tab: "reports" } }) } });
      close();
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => (next ? onOpenChange(true) : close())}
      title={t("report.title")}
      description={label ? t("report.leadFor", { label }) : t("report.lead")}
      footer={
        <>
          <Button variant="ghost" onClick={close} disabled={send.isPending}>
            {t("report.cancel")}
          </Button>
          <Button variant="danger" disabled={category === undefined} loading={send.isPending} onClick={() => send.mutate()}>
            {t("report.send")}
          </Button>
        </>
      }
    >
      <div role="radiogroup" aria-label={t("report.reason")} className="grid grid-cols-1 gap-1.5 sm:grid-cols-2">
        {REPORT_CATEGORIES.map((entry) => {
          const selected = entry.category === category;
          return (
            // biome-ignore lint/a11y/useSemanticElements: a radio drawn as a tile with an emoji, not a bare radio input.
            <button
              key={entry.key}
              type="button"
              role="radio"
              aria-checked={selected}
              onClick={() => setCategory(entry.category)}
              className={cn(
                "flex items-center gap-2.5 rounded-2xl border-2 px-3 py-2.5 text-left text-[15px] font-bold transition-colors",
                selected ? "border-ink bg-brand-soft" : "border-line hover:border-ink-faint",
              )}
            >
              <span aria-hidden className="text-xl leading-none">
                {entry.emoji}
              </span>
              {t(`report.categories.${entry.key}`)}
            </button>
          );
        })}
      </div>
      <TextArea
        className="mt-4"
        label={t("report.details")}
        placeholder={t("report.detailsPlaceholder")}
        value={text}
        maxLength={TEXT_MAX}
        onChange={(event) => setText(event.target.value)}
        hint={t("report.detailsHint", { count: TEXT_MAX - text.length })}
      />
    </Dialog>
  );
}

/**
 * A "report" button with its dialog, for screens that have no menu to put
 * the action in (a profile's action row).
 */
export function ReportButton({ subject, label }: Pick<ReportDialogProps, "subject" | "label">) {
  const { t } = useTranslation("engagement");
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button variant="ghost" icon={<Flag className="size-4" />} onClick={() => setOpen(true)}>
        {t("report.action")}
      </Button>
      <ReportDialog subject={subject} label={label} open={open} onOpenChange={setOpen} />
    </>
  );
}
