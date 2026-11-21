import { admin, type common } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { actionKey, actionTone, caseStatusKey, categoryKey, subjectTypeKey } from "../labels";
import { ToneChip } from "../ui/tone";

/** What kind of thing a case or action is about. */
export function SubjectTypeChip({ type }: { type: common.SubjectType | undefined }) {
  const { t } = useTranslation("admin");
  return <ToneChip tone="brand">{t(subjectTypeKey(type ?? 0))}</ToneChip>;
}

export function CategoryChips({ categories }: { categories: admin.ReportCategory[] }) {
  const { t } = useTranslation("admin");
  return (
    <span className="inline-flex flex-wrap gap-1">
      {categories.map((category) => (
        <ToneChip key={category}>{t(categoryKey(category))}</ToneChip>
      ))}
    </span>
  );
}

export function ActionChip({ kind }: { kind: admin.ModerationActionKind }) {
  const { t } = useTranslation("admin");
  return <ToneChip tone={actionTone(kind)}>{t(actionKey(kind))}</ToneChip>;
}

/** Where a case stands: waiting, or how it was closed. */
export function CaseStateChip({ moderationCase }: { moderationCase: admin.ModerationCase }) {
  const { t } = useTranslation("admin");
  if (moderationCase.status !== admin.CaseStatus.RESOLVED) return <ToneChip tone="warn">{t(caseStatusKey(admin.CaseStatus.OPEN))}</ToneChip>;
  // No resolution: the case closed itself because the owner's account was deleted.
  if (moderationCase.resolution === admin.ModerationActionKind.UNSPECIFIED) return <ToneChip>{t("moderation.autoClosed")}</ToneChip>;
  return <ActionChip kind={moderationCase.resolution} />;
}

/** The title captured with the latest report, or a stand-in when there is none. */
export function useCaseTitle(): (moderationCase: Pick<admin.ModerationCase, "subjectTitle">) => string {
  const { t } = useTranslation("admin");
  return (moderationCase) => moderationCase.subjectTitle || t("moderation.untitled");
}
