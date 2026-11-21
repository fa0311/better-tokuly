import { useInfiniteQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import type { admin } from "@tokuly/api";
import { Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState } from "#/components/ui/page";
import { moderationClient } from "../api";
import { formatDateTime } from "../format";
import { ACTION_KINDS, actionKey } from "../labels";
import { parseEnum, parseText } from "../params";
import { ConsolePage } from "../ui/console-page";
import { type Column, DataTable } from "../ui/data-table";
import { FilterField, FilterSelect, FilterToken } from "../ui/filters";
import { ErrorState, RowsSkeleton } from "../ui/states";
import { UserCell } from "../ui/user";
import { UserName } from "../ui/user-name";
import { UserPicker } from "../ui/user-picker";
import { ActionChip, SubjectTypeChip } from "./parts";
import { TakedownDialog } from "./takedown-dialog";

/** The moderation log's filters, kept in the address bar. */
export interface LogSearch {
  /** Actions against this account: its moderation history. */
  user?: string;
  /** Actions taken by this staff member. */
  moderator?: string;
  kind?: (typeof ACTION_KINDS)[number];
  /** Actions belonging to this case. */
  case?: string;
}

export function parseLogSearch(search: Record<string, unknown>): LogSearch {
  return { user: parseText(search.user), moderator: parseText(search.moderator), kind: parseEnum(search.kind, ACTION_KINDS), case: parseText(search.case) };
}

/** `/admin/moderation/log`: every action staff (or the system) took, newest first. */
export function ModerationLogScreen({ search }: { search: LogSearch }) {
  const { t } = useTranslation("admin");
  const navigate = useNavigate();
  const [takedownOpen, setTakedownOpen] = useState(false);
  const log = useInfiniteQuery({
    queryKey: ["admin", "actions", search],
    queryFn: ({ pageParam }) => moderationClient().listModerationActions({ page: { cursor: pageParam, limit: 50 }, userId: search.user, moderatorId: search.moderator, kind: search.kind, caseId: search.case }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const actions = log.data?.pages.flatMap((page) => page.actions) ?? [];
  const update = (changes: Partial<LogSearch>) => navigate({ to: "/admin/moderation/log", search: { ...search, ...changes }, replace: true });

  const columns: Column<admin.ModerationAction>[] = [
    { key: "time", header: t("moderation.log.columns.time"), className: "whitespace-nowrap", cell: (row) => <span className="text-ink-soft">{formatDateTime(row.createdAt)}</span> },
    { key: "kind", header: t("moderation.log.columns.action"), card: "bare", cell: (row) => <ActionChip kind={row.kind} /> },
    { key: "moderator", header: t("moderation.log.columns.moderator"), cell: (row) => <UserCell user={row.moderator} fallback={t("common.system")} /> },
    { key: "target", header: t("moderation.log.columns.target"), card: "title", cell: (row) => <UserCell user={row.targetUser} /> },
    {
      key: "subject",
      header: t("moderation.log.columns.subject"),
      cell: (row) =>
        row.subject ? (
          <span className="flex min-w-0 flex-col items-start gap-1">
            <SubjectTypeChip type={row.subject.type} />
            <span className="line-clamp-1 break-all text-xs text-ink-soft">{row.subjectTitle || row.subject.id}</span>
          </span>
        ) : (
          <span className="text-ink-faint">—</span>
        ),
    },
    {
      key: "reason",
      header: t("moderation.log.columns.reason"),
      className: "w-1/4",
      cell: (row) => (
        <span className="flex flex-col gap-1">
          {row.reason && <span className="line-clamp-3 whitespace-pre-wrap break-words">{row.reason}</span>}
          {row.note && <span className="line-clamp-2 whitespace-pre-wrap break-words text-xs text-ink-soft">{row.note}</span>}
          {row.expiresAt && <span className="text-xs text-ink-soft">{t("moderation.log.until", { time: formatDateTime(row.expiresAt) })}</span>}
          {!row.reason && !row.note && !row.expiresAt && <span className="text-ink-faint">—</span>}
        </span>
      ),
    },
    {
      key: "case",
      header: t("moderation.log.columns.case"),
      cell: (row) =>
        row.caseId ? (
          <Link to="/admin/moderation/$caseId" params={{ caseId: row.caseId }} className="text-xs font-bold whitespace-nowrap text-brand hover:underline">
            {t("moderation.log.viewCase")}
          </Link>
        ) : (
          <span className="text-xs text-ink-faint">{t("moderation.log.direct")}</span>
        ),
    },
  ];

  return (
    <ConsolePage
      title={t("moderation.log.title")}
      back="/admin/moderation"
      actions={
        <Button size="sm" variant="secondary" className="px-3 sm:px-4" aria-label={t("moderation.takedown.open")} icon={<Trash2 className="size-4" />} onClick={() => setTakedownOpen(true)}>
          <span className="hidden sm:inline">{t("moderation.takedown.open")}</span>
        </Button>
      }
    >
      <div className="mb-3 grid grid-cols-2 gap-2 md:grid-cols-4">
        <FilterSelect
          label={t("moderation.log.columns.action")}
          value={String(search.kind ?? "")}
          onChange={(value) => update({ kind: value ? (Number(value) as LogSearch["kind"]) : undefined })}
          options={[{ value: "", label: t("common.all") }, ...ACTION_KINDS.map((kind) => ({ value: String(kind), label: t(actionKey(kind)) }))]}
        />
        {search.moderator ? (
          <FilterToken label={t("moderation.log.columns.moderator")} clearLabel={t("common.clearFilter")} onClear={() => update({ moderator: undefined })}>
            <UserName userId={search.moderator} />
          </FilterToken>
        ) : (
          <FilterField label={t("moderation.log.columns.moderator")}>
            <UserPicker label={t("moderation.log.columns.moderator")} staffOnly onPick={(account) => update({ moderator: account.user?.id })} />
          </FilterField>
        )}
        {search.user ? (
          <FilterToken label={t("moderation.log.columns.target")} clearLabel={t("common.clearFilter")} onClear={() => update({ user: undefined })}>
            <UserName userId={search.user} />
          </FilterToken>
        ) : (
          <FilterField label={t("moderation.log.columns.target")}>
            <UserPicker label={t("moderation.log.columns.target")} onPick={(account) => update({ user: account.user?.id })} />
          </FilterField>
        )}
        {search.case && (
          <FilterToken label={t("moderation.log.columns.case")} clearLabel={t("common.clearFilter")} onClear={() => update({ case: undefined })}>
            <code className="font-mono text-xs">{search.case}</code>
          </FilterToken>
        )}
      </div>

      {log.isPending ? (
        <RowsSkeleton />
      ) : log.isError ? (
        <ErrorState error={log.error} onRetry={() => log.refetch()} />
      ) : actions.length === 0 ? (
        <EmptyState emoji="📒" title={t("moderation.log.empty")} body={t("moderation.log.emptyBody")} />
      ) : (
        <>
          <DataTable label={t("moderation.log.title")} columns={columns} rows={actions} rowKey={(row) => row.id} />
          <LoadMore query={log} />
        </>
      )}
      <TakedownDialog open={takedownOpen} onOpenChange={setTakedownOpen} />
    </ConsolePage>
  );
}
