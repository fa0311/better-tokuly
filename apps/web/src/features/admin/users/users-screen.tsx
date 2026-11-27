import { useInfiniteQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { type admin, toMillis } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState } from "#/components/ui/page";
import { compactAge } from "#/lib/format";
import { userAdminClient } from "../api";
import { ACCOUNT_STATES } from "../filter-values";
import { formatDate } from "../format";
import { accountStateKey, roleKey } from "../labels";
import { STAFF_ROLES, type StaffRole } from "../roles";
import { ConsolePage } from "../ui/console-page";
import { type Column, DataTable } from "../ui/data-table";
import { FilterSelect, SearchBox } from "../ui/filters";
import { useSearchText } from "../ui/hooks";
import { ErrorState, RowsSkeleton } from "../ui/states";
import { ToneChip } from "../ui/tone";
import { UserCell } from "../ui/user";
import { AccountStateChip, RoleChips } from "./parts";
import type { UsersSearch } from "./users-search";

/** `/admin/users`: find an account by handle, name, email or ID; without a query, the newest accounts. */
export function UsersScreen({ search }: { search: UsersSearch }) {
  const { t } = useTranslation("admin");
  const navigate = useNavigate();
  // The list follows the typing; the address takes the search over once the typing is done.
  const typed = useSearchText(search.q, (q) =>
    navigate({ to: "/admin/users", search: { ...search, q }, replace: true }),
  );
  const filters: UsersSearch = { ...search, q: typed.query };
  const update = (changes: Partial<UsersSearch>) =>
    navigate({ to: "/admin/users", search: { ...filters, ...changes }, replace: true });

  const users = useInfiniteQuery({
    queryKey: ["admin", "users", "search", filters],
    queryFn: ({ pageParam }) =>
      userAdminClient().searchUsers({
        page: { cursor: pageParam, limit: 30 },
        query: filters.q,
        state: filters.state,
        role: filters.role,
        verifiedOnly: filters.verified,
      }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const accounts = users.data?.pages.flatMap((page) => page.users) ?? [];
  const filtered = Boolean(filters.q || filters.state || filters.role || filters.verified);

  const columns: Column<admin.AdminUser>[] = [
    {
      key: "user",
      header: t("users.columns.user"),
      card: "title",
      cell: (row) => <UserCell user={row.user} />,
    },
    {
      key: "email",
      header: t("users.columns.email"),
      cell: (row) => (
        <span className="inline-flex max-w-full flex-wrap items-center gap-x-2 gap-y-1">
          <span className="break-all">{row.email || "—"}</span>
          {row.email && !row.emailVerified && <ToneChip>{t("users.emailUnverified")}</ToneChip>}
        </span>
      ),
    },
    {
      key: "state",
      header: t("users.columns.state"),
      card: "bare",
      cell: (row) => <AccountStateChip account={row} />,
    },
    {
      key: "roles",
      header: t("users.columns.roles"),
      cell: (row) => <RoleChips roles={row.roles} />,
    },
    {
      key: "created",
      header: t("users.columns.created"),
      className: "whitespace-nowrap",
      cell: (row) => <span className="text-ink-soft">{formatDate(row.createdAt)}</span>,
    },
    {
      key: "lastActive",
      header: t("users.columns.lastActive"),
      className: "whitespace-nowrap",
      cell: (row) => (
        <span className="text-ink-soft">
          {row.lastActiveAt ? compactAge(toMillis(row.lastActiveAt)) : "—"}
        </span>
      ),
    },
    {
      key: "sessions",
      header: t("users.columns.sessions"),
      className: "text-right",
      cell: (row) => <span className="tabular-nums">{row.activeSessionCount}</span>,
    },
  ];

  return (
    <ConsolePage title={t("users.title")}>
      <div className="mb-3 grid grid-cols-2 gap-2 md:grid-cols-[minmax(0,1fr)_10rem_10rem_10rem] md:items-end">
        <SearchBox
          className="col-span-2 md:col-span-1"
          label={t("users.search")}
          placeholder={t("users.searchPlaceholder")}
          value={typed.text}
          onChange={typed.setText}
          onCommit={typed.commit}
        />
        <FilterSelect
          label={t("users.columns.state")}
          value={String(search.state ?? "")}
          onChange={(value) =>
            update({ state: value ? (Number(value) as UsersSearch["state"]) : undefined })
          }
          options={[
            { value: "", label: t("common.all") },
            ...ACCOUNT_STATES.map((state) => ({
              value: String(state),
              label: t(accountStateKey(state)),
            })),
          ]}
        />
        <FilterSelect<StaffRole | "">
          label={t("users.columns.roles")}
          value={search.role ?? ""}
          onChange={(value) => update({ role: value || undefined })}
          options={[
            { value: "", label: t("common.all") },
            ...STAFF_ROLES.map((role) => ({ value: role, label: t(roleKey(role)) })),
          ]}
        />
        <FilterSelect<"all" | "verified">
          label={t("users.verified")}
          value={search.verified ? "verified" : "all"}
          onChange={(value) => update({ verified: value === "verified" ? true : undefined })}
          options={[
            { value: "all", label: t("common.all") },
            { value: "verified", label: t("users.verifiedOnly") },
          ]}
        />
      </div>

      {users.isPending ? (
        <RowsSkeleton />
      ) : users.isError ? (
        <ErrorState error={users.error} onRetry={() => users.refetch()} />
      ) : accounts.length === 0 ? (
        <EmptyState
          emoji="🔍"
          title={t("users.empty")}
          body={filtered ? t("users.emptyBody") : undefined}
        />
      ) : (
        <>
          {!filtered && (
            <p className="mb-2 px-1 text-xs font-bold text-ink-faint">{t("users.newest")}</p>
          )}
          <DataTable
            label={t("users.title")}
            columns={columns}
            rows={accounts}
            rowKey={(row) => row.user?.id ?? ""}
            onRowClick={(row) =>
              row.user && navigate({ to: "/admin/users/$userId", params: { userId: row.user.id } })
            }
          />
          <LoadMore query={users} />
        </>
      )}
    </ConsolePage>
  );
}
