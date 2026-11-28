import { admin, type common } from "@tokuly/api";
import { useTranslation } from "#/lib/use-translation";
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
  if (moderationCase.status !== admin.CaseStatus.RESOLVED)
    return <ToneChip tone="warn">{t(caseStatusKey(admin.CaseStatus.OPEN))}</ToneChip>;
  // No resolution: the case closed itself because the owner's account was deleted.
  if (moderationCase.resolution === admin.ModerationActionKind.UNSPECIFIED)
    return <ToneChip>{t("moderation.autoClosed")}</ToneChip>;
  return <ActionChip kind={moderationCase.resolution} />;
}

/** The title captured with the latest report, or a stand-in when there is none. */
export function useCaseTitle(): (
  moderationCase: Pick<admin.ModerationCase, "subjectTitle">,
) => string {
  const { t } = useTranslation("admin");
  return (moderationCase) => moderationCase.subjectTitle || t("moderation.untitled");
}

/** An account's moderation record in numbers, as chips. */
export function ModerationRecord({ record }: { record: admin.UserModerationSummary }) {
  const { t } = useTranslation("admin");
  const figures: [string, number, boolean][] = [
    ["reportsReceived", record.reportsReceived, false],
    ["openCases", record.openCases, record.openCases > 0],
    ["warnings", record.warnings, record.warnings > 0],
    ["contentRemovals", record.contentRemovals, record.contentRemovals > 0],
    ["suspensions", record.suspensions, record.suspensions > 0],
    ["reportsFiled", record.reportsFiled, false],
  ];
  return (
    <span className="flex flex-wrap gap-1.5">
      {figures.map(([name, value, notable]) => (
        <ToneChip key={name} tone={notable ? "warn" : "neutral"}>
          {t(`users.record.${name}`)} <span className="font-black tabular-nums">{value}</span>
        </ToneChip>
      ))}
    </span>
  );
}
