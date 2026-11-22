import { engagement, toDate } from "@tokuly/api";
import { ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Chip } from "#/components/ui/card";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { useMyReports } from "./api";
import { useOpenPath } from "./open-path";
import { REPORT_CATEGORIES, reportCategoryKey } from "./report-dialog";
import { subjectTypeKey } from "./subject-card";

const STATUS: Record<engagement.ReportStatus, { key: string; tone: string }> = {
  [engagement.ReportStatus.UNSPECIFIED]: { key: "submitted", tone: "bg-sun text-[#17122b]" },
  [engagement.ReportStatus.SUBMITTED]: { key: "submitted", tone: "bg-sun text-[#17122b]" },
  [engagement.ReportStatus.ACTION_TAKEN]: { key: "actionTaken", tone: "bg-mint text-[#17122b]" },
  [engagement.ReportStatus.NO_ACTION]: { key: "noAction", tone: "bg-surface-3 text-ink-soft" },
  [engagement.ReportStatus.SUPERSEDED]: { key: "superseded", tone: "bg-surface-3 text-ink-soft" },
};

/** "My reports": what the viewer reported, newest first, each with where it stands. */
export function ReportsList() {
  const { t, i18n } = useTranslation("engagement");
  const query = useMyReports();
  const open = useOpenPath();
  const reports = query.data?.pages.flatMap((page) => page.reports) ?? [];

  if (query.isPending) {
    return (
      <div className="flex flex-col gap-3">
        <Skeleton className="h-24" />
        <Skeleton className="h-24" />
      </div>
    );
  }
  if (reports.length === 0)
    return (
      <EmptyState emoji="🕊️" title={t("report.mine.empty")} body={t("report.mine.emptyBody")} />
    );

  return (
    <ul className="flex flex-col gap-3">
      {reports.map((report) => {
        const status = STATUS[report.status] ?? STATUS[engagement.ReportStatus.SUBMITTED];
        const category = REPORT_CATEGORIES.find((entry) => entry.category === report.category);
        const created = toDate(report.createdAt);
        const resolved = toDate(report.resolvedAt);
        const date = (value: Date) =>
          value.toLocaleDateString(i18n.language, {
            year: "numeric",
            month: "short",
            day: "numeric",
          });
        return (
          <li key={report.id} data-testid="report" className="sticker rounded-card bg-surface p-4">
            <div className="flex flex-wrap items-center gap-2">
              <Chip className={cn("border-2 border-ink font-black", status.tone)}>
                {t(`report.status.${status.key}`)}
              </Chip>
              <Chip>
                <span aria-hidden>{category?.emoji}</span>
                {t(`report.categories.${reportCategoryKey(report.category)}`)}
              </Chip>
              <span className="ml-auto text-xs font-bold text-ink-faint">
                {created && date(created)}
              </span>
            </div>
            <button
              type="button"
              disabled={!report.subjectPath}
              onClick={() => open(report.subjectPath)}
              className="mt-3 flex w-full items-center gap-2 rounded-2xl bg-surface-2 px-3 py-2.5 text-left enabled:hover:bg-surface-3"
            >
              <span className="min-w-0 flex-1">
                <span className="block text-xs font-black text-brand">
                  {t(`subjects.${subjectTypeKey(report.subject?.type)}`)}
                </span>
                <span className="line-clamp-2 text-[15px] font-bold leading-snug">
                  {report.subjectTitle || t("subjects.untitled")}
                </span>
              </span>
              {report.subjectPath && <ChevronRight className="size-5 shrink-0 text-ink-faint" />}
            </button>
            {report.text && (
              <p className="mt-3 text-sm leading-relaxed whitespace-pre-wrap text-ink-soft">
                {report.text}
              </p>
            )}
            {resolved && (
              <p className="mt-2 text-xs font-bold text-ink-faint">
                {t("report.mine.resolvedOn", { date: date(resolved) })}
              </p>
            )}
          </li>
        );
      })}
      <LoadMore query={query} showEnd />
    </ul>
  );
}
