import { useInfiniteQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { type admin, fromDate } from "@tokuly/api";
import { ChevronDown, ChevronRight, FilterX } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState } from "#/components/ui/page";
import { systemClient } from "../api";
import { formatDateTime, fromLocalInput } from "../format";
import { SUBJECT_TYPES, subjectTypeKey } from "../labels";
import { parseChoice, parseText } from "../params";
import { ConsolePage } from "../ui/console-page";
import { type Column, DataTable } from "../ui/data-table";
import { FilterField, FilterInput, FilterSelect, FilterTextInput, FilterToken } from "../ui/filters";
import { ErrorState, RowsSkeleton } from "../ui/states";
import { ToneChip } from "../ui/tone";
import { UserCell } from "../ui/user";
import { UserName } from "../ui/user-name";
import { UserPicker } from "../ui/user-picker";
import { JsonDiff } from "./json-diff";

/** The kinds of thing an audit entry can be about. */
const TARGET_TYPES = ["user", "case", "subject", "flag", "announcement"] as const;
type TargetType = (typeof TARGET_TYPES)[number];

/** Every action the admin domain records, grouped by what it acts on (`docs/events/admin.md`). */
const ACTIONS: Record<string, string[]> = {
  "case.": ["case.assigned", "case.unassigned", "case.note_added", "case.resolved", "case.reopened"],
  "content.": ["content.removed"],
  "user.": ["user.warned", "user.suspended", "user.unsuspended", "user.suspension_expired", "user.verified_changed", "user.roles_changed", "user.sessions_revoked"],
  "flag.": ["flag.created", "flag.updated", "flag.deleted"],
  "announcement.": ["announcement.created", "announcement.updated", "announcement.published", "announcement.unpublished", "announcement.deleted"],
};

/** The audit log's filters, kept in the address bar. Times are local `YYYY-MM-DDTHH:mm`. */
export interface AuditSearch {
  actor?: string;
  /** An exact action (`user.suspended`) or a prefix ending in a dot (`user.`). */
  action?: string;
  targetType?: TargetType;
  targetId?: string;
  targetUser?: string;
  since?: string;
  until?: string;
}

export function parseAuditSearch(search: Record<string, unknown>): AuditSearch {
  return {
    actor: parseText(search.actor),
    action: parseText(search.action),
    targetType: parseChoice(search.targetType, TARGET_TYPES),
    targetId: parseText(search.targetId),
    targetUser: parseText(search.targetUser),
    since: parseText(search.since),
    until: parseText(search.until),
  };
}

/** `/admin/audit`: who did what, to what, and why. */
export function AuditScreen({ search }: { search: AuditSearch }) {
  const { t } = useTranslation("admin");
  const navigate = useNavigate();
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const update = (changes: Partial<AuditSearch>) => navigate({ to: "/admin/audit", search: { ...search, ...changes }, replace: true });

  const since = fromLocalInput(search.since ?? "");
  const until = fromLocalInput(search.until ?? "");
  const log = useInfiniteQuery({
    queryKey: ["admin", "audit", search],
    queryFn: ({ pageParam }) =>
      systemClient().listAuditLog({
        page: { cursor: pageParam, limit: 50 },
        actorId: search.actor,
        action: search.action,
        targetType: search.targetType,
        targetId: search.targetId,
        targetUserId: search.targetUser,
        since: since && fromDate(since),
        until: until && fromDate(until),
      }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const entries = log.data?.pages.flatMap((page) => page.entries) ?? [];
  const filtered = Object.values(search).some(Boolean);
  const toggle = (id: string) =>
    setExpanded((current) => {
      const next = new Set(current);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  const columns: Column<admin.AuditEntry>[] = [
    {
      key: "expand",
      header: "",
      card: "bare",
      className: "w-10",
      cell: (row) => (
        <button type="button" aria-expanded={expanded.has(row.id)} aria-label={t("audit.toggleDetails", { action: row.action })} onClick={() => toggle(row.id)} className="grid size-8 place-items-center rounded-full hover:bg-surface-2">
          {expanded.has(row.id) ? <ChevronDown className="size-4" /> : <ChevronRight className="size-4" />}
        </button>
      ),
    },
    { key: "time", header: t("audit.columns.time"), className: "whitespace-nowrap", cell: (row) => <span className="text-ink-soft">{formatDateTime(row.createdAt)}</span> },
    { key: "actor", header: t("audit.columns.actor"), cell: (row) => <UserCell user={row.actor} fallback={t("common.system")} /> },
    { key: "action", header: t("audit.columns.action"), card: "title", cell: (row) => <ActionName action={row.action} /> },
    { key: "target", header: t("audit.columns.target"), cell: (row) => <Target entry={row} /> },
    { key: "reason", header: t("audit.columns.reason"), className: "w-1/4", cell: (row) => (row.reason ? <span className="line-clamp-2 whitespace-pre-wrap break-words">{row.reason}</span> : <span className="text-ink-faint">—</span>) },
  ];

  return (
    <ConsolePage
      title={t("audit.title")}
      actions={
        filtered && (
          <Button size="sm" variant="ghost" icon={<FilterX className="size-4" />} onClick={() => navigate({ to: "/admin/audit", search: {}, replace: true })}>
            {t("audit.clearFilters")}
          </Button>
        )
      }
    >
      <AuditFilters search={search} onChange={update} />
      {log.isPending ? (
        <RowsSkeleton />
      ) : log.isError ? (
        <ErrorState error={log.error} onRetry={() => log.refetch()} />
      ) : entries.length === 0 ? (
        <EmptyState emoji="📜" title={t(filtered ? "audit.emptyFiltered" : "audit.empty")} />
      ) : (
        <>
          <DataTable
            label={t("audit.title")}
            columns={columns}
            rows={entries}
            rowKey={(row) => row.id}
            onRowClick={(row) => toggle(row.id)}
            expanded={(row) => expanded.has(row.id) && <EntryDetails entry={row} />}
          />
          <LoadMore query={log} />
        </>
      )}
    </ConsolePage>
  );
}

/** A readable name for an action with its dotted identifier underneath. */
function ActionName({ action }: { action: string }) {
  const { t, i18n } = useTranslation("admin");
  const known = i18n.exists(`admin:audit.actions.${action}`);
  return (
    <span className="flex flex-col">
      {known && <span className="font-bold">{t(`audit.actions.${action}`)}</span>}
      <code className={known ? "font-mono text-xs font-normal text-ink-soft" : "font-mono font-bold"}>{action}</code>
    </span>
  );
}

/** The first block of a UUID, enough to tell rows apart; the details show it whole. */
const shortId = (id: string) => (/^[0-9a-f]{8}-/.test(id) ? `${id.slice(0, 8)}…` : id);

/** What an entry acted on (linked to its page in the console where there is one) and the account it affected. */
function Target({ entry }: { entry: admin.AuditEntry }) {
  const { t } = useTranslation("admin");
  const type = parseChoice(entry.targetType, TARGET_TYPES);
  const linkClass = "font-mono text-xs font-bold break-all text-brand hover:underline";
  let reference: ReactNode = null;
  if (type === "case" && entry.targetId) {
    reference = (
      <Link to="/admin/moderation/$caseId" params={{ caseId: entry.targetId }} title={entry.targetId} className={linkClass}>
        {shortId(entry.targetId)}
      </Link>
    );
  } else if (type === "flag" && entry.targetId) {
    reference = (
      <Link to="/admin/flags" search={{ q: entry.targetId }} className={linkClass}>
        {entry.targetId}
      </Link>
    );
  } else if (type === "announcement" && entry.targetId) {
    reference = (
      <Link to="/admin/announcements/$announcementId" params={{ announcementId: entry.targetId }} title={entry.targetId} className={linkClass}>
        {shortId(entry.targetId)}
      </Link>
    );
  } else if (type === "subject") {
    // `<subject type number>:<id>`
    const [typeNumber, id] = entry.targetId.split(":");
    const subjectType = SUBJECT_TYPES.find((candidate) => candidate === Number(typeNumber));
    reference = (
      <span className="text-xs">
        {subjectType !== undefined && <span className="font-bold">{t(subjectTypeKey(subjectType))} </span>}
        <code title={id} className="font-mono text-ink-soft">
          {shortId(id ?? entry.targetId)}
        </code>
      </span>
    );
  } else if (type !== "user" && entry.targetId) {
    reference = <code className="font-mono text-xs break-all text-ink-soft">{entry.targetId}</code>;
  }
  return (
    <span className="flex min-w-0 flex-col items-start gap-1">
      <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <ToneChip>{type ? t(`audit.targetTypes.${type}`) : entry.targetType || "—"}</ToneChip>
        {reference}
      </span>
      {/* For an account, the person is the reference. */}
      {entry.targetUser ? <UserCell user={entry.targetUser} /> : type === "user" && <code className="font-mono text-xs break-all text-ink-soft">{entry.targetId}</code>}
    </span>
  );
}

/** Everything else an entry recorded: full IDs, the reason in full, the request's address and the snapshots. */
function EntryDetails({ entry }: { entry: admin.AuditEntry }) {
  const { t } = useTranslation("admin");
  return (
    <div className="flex flex-col gap-3">
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-xs">
        <dt className="font-bold text-ink-faint">{t("audit.filters.targetId")}</dt>
        <dd className="font-mono break-all">{entry.targetId || "—"}</dd>
        <dt className="font-bold text-ink-faint">{t("audit.columns.ip")}</dt>
        <dd className="font-mono">{entry.ip || "—"}</dd>
        {entry.reason && (
          <>
            <dt className="font-bold text-ink-faint">{t("audit.columns.reason")}</dt>
            <dd className="whitespace-pre-wrap break-words">{entry.reason}</dd>
          </>
        )}
      </dl>
      {(entry.beforeJson || entry.afterJson) && <JsonDiff beforeJson={entry.beforeJson} afterJson={entry.afterJson} />}
    </div>
  );
}

function AuditFilters({ search, onChange }: { search: AuditSearch; onChange(changes: Partial<AuditSearch>): void }) {
  const { t } = useTranslation("admin");
  const actionOptions = [
    { value: "", label: t("common.all") },
    ...Object.entries(ACTIONS).flatMap(([prefix, actions]) => [{ value: prefix, label: t("audit.actionGroup", { group: t(`audit.groups.${prefix.slice(0, -1)}`) }) }, ...actions.map((action) => ({ value: action, label: `　${t(`audit.actions.${action}`)}` }))]),
  ];
  // An action from a link that this list does not know is still shown as selected.
  if (search.action && !actionOptions.some((option) => option.value === search.action)) actionOptions.push({ value: search.action, label: search.action });

  return (
    <div className="mb-3 grid grid-cols-2 gap-2 md:grid-cols-4 2xl:grid-cols-7">
      <FilterSelect label={t("audit.columns.action")} value={search.action ?? ""} onChange={(value) => onChange({ action: value || undefined })} options={actionOptions} />
      {search.actor ? (
        <FilterToken label={t("audit.columns.actor")} clearLabel={t("common.clearFilter")} onClear={() => onChange({ actor: undefined })}>
          <UserName userId={search.actor} />
        </FilterToken>
      ) : (
        <FilterField label={t("audit.columns.actor")}>
          <UserPicker label={t("audit.columns.actor")} staffOnly onPick={(account) => onChange({ actor: account.user?.id })} />
        </FilterField>
      )}
      {search.targetUser ? (
        <FilterToken label={t("audit.columns.targetUser")} clearLabel={t("common.clearFilter")} onClear={() => onChange({ targetUser: undefined })}>
          <UserName userId={search.targetUser} />
        </FilterToken>
      ) : (
        <FilterField label={t("audit.columns.targetUser")}>
          <UserPicker label={t("audit.columns.targetUser")} onPick={(account) => onChange({ targetUser: account.user?.id })} />
        </FilterField>
      )}
      <FilterSelect<TargetType | "">
        label={t("audit.filters.targetType")}
        value={search.targetType ?? ""}
        onChange={(value) => onChange({ targetType: value || undefined })}
        options={[{ value: "", label: t("common.all") }, ...TARGET_TYPES.map((type) => ({ value: type, label: t(`audit.targetTypes.${type}`) }))]}
      />
      <FilterTextInput label={t("audit.filters.targetId")} value={search.targetId ?? ""} onCommit={(targetId) => onChange({ targetId: targetId || undefined })} placeholder={t("audit.filters.targetIdPlaceholder")} />
      <FilterInput type="datetime-local" label={t("audit.filters.since")} value={search.since ?? ""} onChange={(value) => onChange({ since: value || undefined })} />
      <FilterInput type="datetime-local" label={t("audit.filters.until")} value={search.until ?? ""} onChange={(value) => onChange({ until: value || undefined })} />
    </div>
  );
}
