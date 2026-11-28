import { Link, useNavigate } from "@tanstack/react-router";
import { type admin, toMillis } from "@tokuly/api";
import { History, RefreshCw, Trash2 } from "lucide-react";
import { useState } from "react";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { compactAge } from "#/lib/format";
import { useTranslation } from "#/lib/use-translation";
import { REPORT_CATEGORIES, SUBJECT_TYPES } from "../filter-values";
import { formatDateTime } from "../format";
import { categoryKey, subjectTypeKey } from "../labels";
import { ConsolePage } from "../ui/console-page";
import { type Column, DataTable } from "../ui/data-table";
import { FilterField, FilterSelect, FilterToken } from "../ui/filters";
import { ErrorState, RowsSkeleton } from "../ui/states";
import { UserCell } from "../ui/user";
import { UserName } from "../ui/user-name";
import { UserPicker } from "../ui/user-picker";
import { CaseStateChip, CategoryChips, SubjectTypeChip, useCaseTitle } from "./parts";
import { useCaseQueue } from "./queue";
import type { QueueSearch } from "./queue-search";
import { TakedownDialog } from "./takedown-dialog";

/** `/admin/moderation`: the reports queue. */
export function ModerationQueueScreen({ search }: { search: QueueSearch }) {
  const { t } = useTranslation("admin");
  const navigate = useNavigate();
  const [takedownOpen, setTakedownOpen] = useState(false);
  const queue = useCaseQueue(search);
  const cases = queue.data?.pages.flatMap((page) => page.cases) ?? [];
  const caseTitle = useCaseTitle();
  const resolved = search.status === "resolved";

  const update = (changes: Partial<QueueSearch>) =>
    navigate({ to: "/admin/moderation", search: { ...search, ...changes }, replace: true });
  const open = (moderationCase: admin.ModerationCase) =>
    navigate({ to: "/admin/moderation/$caseId", params: { caseId: moderationCase.id }, search });

  const columns: Column<admin.ModerationCase>[] = [
    {
      key: "subject",
      header: t("moderation.columns.subject"),
      card: "title",
      cell: (row) => (
        <span className="flex min-w-0 flex-col items-start gap-1">
          <Link
            to="/admin/moderation/$caseId"
            params={{ caseId: row.id }}
            search={search}
            className="line-clamp-2 font-bold hover:underline"
          >
            {caseTitle(row)}
          </Link>
          <SubjectTypeChip type={row.subject?.type} />
        </span>
      ),
    },
    {
      key: "categories",
      header: t("moderation.columns.categories"),
      card: "bare",
      cell: (row) => <CategoryChips categories={row.categories} />,
    },
    {
      key: "reports",
      header: t("moderation.columns.reports"),
      className: "text-right",
      cell: (row) => <span className="font-black tabular-nums">{row.reportCount}</span>,
    },
    // The queue never names the author of an anonymous post; the case does, to a moderator who is not involved.
    {
      key: "owner",
      header: t("moderation.columns.owner"),
      cell: (row) => (
        <UserCell
          user={row.subjectOwner}
          fallback={t(row.ownerWithheld ? "moderation.ownerWithheld" : "moderation.ownerGone")}
        />
      ),
    },
    {
      key: "assignee",
      header: t("moderation.columns.assignee"),
      cell: (row) => <UserCell user={row.assignee} fallback={t("moderation.unassigned")} />,
    },
    resolved
      ? {
          key: "resolved",
          header: t("moderation.columns.resolved"),
          cell: (row) => (
            <span className="flex flex-col items-start gap-1">
              <CaseStateChip moderationCase={row} />
              <span className="text-xs text-ink-soft">{formatDateTime(row.resolvedAt)}</span>
            </span>
          ),
        }
      : {
          key: "lastReported",
          header: t("moderation.columns.lastReported"),
          cell: (row) => (
            <span
              title={formatDateTime(row.lastReportedAt)}
              className="whitespace-nowrap text-ink-soft"
            >
              {compactAge(toMillis(row.lastReportedAt))}
            </span>
          ),
        },
  ];

  return (
    <ConsolePage
      title={t("moderation.title")}
      actions={
        <>
          {/* Reports arrive without a live feed: the queue is re-read on request. On phones the actions are icons, so the page title keeps its room. */}
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={t("common.refresh")}
            loading={queue.isRefetching}
            icon={<RefreshCw className="size-4" />}
            onClick={() => queue.refetch()}
          />
          <Link
            to="/admin/moderation/log"
            aria-label={t("moderation.log.link")}
            className="inline-flex h-9 items-center gap-1.5 rounded-full px-2.5 text-sm font-bold hover:bg-surface-2 sm:px-3"
          >
            <History className="size-4" />
            <span className="hidden sm:inline">{t("moderation.log.link")}</span>
          </Link>
          <Button
            size="sm"
            variant="secondary"
            className="px-3 sm:px-4"
            aria-label={t("moderation.takedown.open")}
            icon={<Trash2 className="size-4" />}
            onClick={() => setTakedownOpen(true)}
          >
            <span className="hidden sm:inline">{t("moderation.takedown.open")}</span>
          </Button>
        </>
      }
    >
      <Tabs<"open" | "resolved">
        className="mb-3"
        value={resolved ? "resolved" : "open"}
        onChange={(status) => update({ status: status === "resolved" ? "resolved" : undefined })}
        items={[
          { value: "open", label: t("enums.caseStatus.OPEN") },
          { value: "resolved", label: t("enums.caseStatus.RESOLVED") },
        ]}
      />
      <QueueFilters search={search} onChange={update} />

      {queue.isPending ? (
        <RowsSkeleton />
      ) : queue.isError ? (
        <ErrorState error={queue.error} onRetry={() => queue.refetch()} />
      ) : cases.length === 0 ? (
        <EmptyState
          emoji={resolved ? "🗂️" : "🕊️"}
          title={t(resolved ? "moderation.emptyResolved" : "moderation.emptyOpen")}
          body={resolved ? undefined : t("moderation.emptyOpenBody")}
        />
      ) : (
        <>
          <DataTable
            label={t("moderation.title")}
            columns={columns}
            rows={cases}
            rowKey={(row) => row.id}
            onRowClick={open}
          />
          <LoadMore query={queue} />
        </>
      )}

      <TakedownDialog open={takedownOpen} onOpenChange={setTakedownOpen} />
    </ConsolePage>
  );
}

type AssigneeChoice = "all" | "mine" | "unassigned" | "someone";

function QueueFilters({
  search,
  onChange,
}: {
  search: QueueSearch;
  onChange(changes: Partial<QueueSearch>): void;
}) {
  const { t } = useTranslation("admin");
  const [picking, setPicking] = useState(false);
  const assigneeId =
    search.assignee && search.assignee !== "mine" && search.assignee !== "unassigned"
      ? search.assignee
      : undefined;
  const assigneeChoice: AssigneeChoice =
    picking || assigneeId ? "someone" : ((search.assignee as AssigneeChoice | undefined) ?? "all");

  return (
    <div className="mb-3 grid grid-cols-2 gap-2 md:grid-cols-3 xl:grid-cols-5">
      <FilterSelect
        label={t("moderation.filters.category")}
        value={String(search.category ?? "")}
        onChange={(value) =>
          onChange({ category: value ? (Number(value) as QueueSearch["category"]) : undefined })
        }
        options={[
          { value: "", label: t("common.all") },
          ...REPORT_CATEGORIES.map((category) => ({
            value: String(category),
            label: t(categoryKey(category)),
          })),
        ]}
      />
      <FilterSelect
        label={t("moderation.filters.subjectType")}
        value={String(search.type ?? "")}
        onChange={(value) =>
          onChange({ type: value ? (Number(value) as QueueSearch["type"]) : undefined })
        }
        options={[
          { value: "", label: t("common.all") },
          ...SUBJECT_TYPES.map((type) => ({ value: String(type), label: t(subjectTypeKey(type)) })),
        ]}
      />
      <FilterSelect<AssigneeChoice>
        label={t("moderation.filters.assignee")}
        value={assigneeChoice}
        onChange={(value) => {
          setPicking(value === "someone");
          if (value !== "someone") onChange({ assignee: value === "all" ? undefined : value });
        }}
        options={[
          { value: "all", label: t("common.all") },
          { value: "mine", label: t("moderation.filters.mine") },
          { value: "unassigned", label: t("moderation.filters.unassigned") },
          { value: "someone", label: t("moderation.filters.someone") },
        ]}
      />
      {assigneeId ? (
        <FilterToken
          label={t("moderation.filters.moderator")}
          clearLabel={t("common.clearFilter")}
          onClear={() => onChange({ assignee: undefined })}
        >
          <UserName userId={assigneeId} />
        </FilterToken>
      ) : (
        picking && (
          <FilterField label={t("moderation.filters.moderator")}>
            <UserPicker
              label={t("moderation.filters.moderator")}
              staffOnly
              onPick={(account) => {
                setPicking(false);
                onChange({ assignee: account.user?.id });
              }}
            />
          </FilterField>
        )
      )}
      <FilterSelect<"newest" | "oldest" | "reports">
        label={t("moderation.filters.sort")}
        value={search.sort ?? "newest"}
        onChange={(value) => onChange({ sort: value === "newest" ? undefined : value })}
        options={[
          { value: "newest", label: t("moderation.filters.sortNewest") },
          { value: "oldest", label: t("moderation.filters.sortOldest") },
          { value: "reports", label: t("moderation.filters.sortReports") },
        ]}
      />
      {search.owner && (
        <FilterToken
          label={t("moderation.filters.owner")}
          clearLabel={t("common.clearFilter")}
          onClear={() => onChange({ owner: undefined })}
        >
          <UserName userId={search.owner} />
        </FilterToken>
      )}
    </div>
  );
}
