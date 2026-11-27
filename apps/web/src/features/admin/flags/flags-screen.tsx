import {
  type InfiniteData,
  useInfiniteQuery,
  useMutation,
  useQueryClient,
} from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { type admin, toMillis } from "@tokuly/api";
import { Pencil, Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { errorMessage } from "#/lib/errors";
import { compactAge } from "#/lib/format";
import { flagClient } from "../api";
import { formatDateTime } from "../format";
import { ConsolePage } from "../ui/console-page";
import { type Column, DataTable } from "../ui/data-table";
import { SearchBox } from "../ui/filters";
import { useIsWide, useSearchText } from "../ui/hooks";
import { ErrorState, RowsSkeleton } from "../ui/states";
import { Toggle } from "../ui/toggle";
import { EvaluatePanel } from "./evaluate-panel";
import { FlagDialog } from "./flag-dialog";
import type { FlagsSearch } from "./flags-search";

type FlagPages = InfiniteData<admin.ListFlagsResponse>;

/** `/admin/flags`: the feature flags, and a preview of what one account gets. */
export function FlagsScreen({ search }: { search: FlagsSearch }) {
  const { t } = useTranslation("admin");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const wide = useIsWide();
  // The list follows the typing; the address takes the search over once the typing is done.
  const typed = useSearchText(search.q, (q) =>
    navigate({ to: "/admin/flags", search: { ...search, q }, replace: true }),
  );
  const update = (changes: Partial<FlagsSearch>) =>
    navigate({
      to: "/admin/flags",
      search: { ...search, q: typed.query, ...changes },
      replace: true,
    });

  const [tab, setTab] = useState<"flags" | "evaluate">(search.evaluate ? "evaluate" : "flags");
  // The editor is mounted anew for each opening (`session`), so it always starts from the flag as listed.
  const [editor, setEditor] = useState<{
    flag?: admin.FeatureFlag;
    session: number;
    open: boolean;
  }>({ session: 0, open: false });
  const [deleting, setDeleting] = useState<admin.FeatureFlag | null>(null);

  const listKey = ["admin", "flags", "list", typed.query ?? ""];
  const flags = useInfiniteQuery({
    queryKey: listKey,
    queryFn: ({ pageParam }) =>
      flagClient().listFlags({ page: { cursor: pageParam, limit: 50 }, query: typed.query }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const rows = flags.data?.pages.flatMap((page) => page.flags) ?? [];

  // The switch flips at once; the server's answer (or its refusal) follows.
  const toggle = useMutation({
    mutationFn: (input: { name: string; enabled: boolean }) => flagClient().updateFlag(input),
    onMutate: async (input) => {
      await queryClient.cancelQueries({ queryKey: listKey });
      const previous = queryClient.getQueryData<FlagPages>(listKey);
      queryClient.setQueryData<FlagPages>(
        listKey,
        (data) =>
          data && {
            ...data,
            pages: data.pages.map((page) => ({
              ...page,
              flags: page.flags.map((flag) =>
                flag.name === input.name ? { ...flag, enabled: input.enabled } : flag,
              ),
            })),
          },
      );
      return { previous };
    },
    onError: (error, _input, context) => {
      queryClient.setQueryData(listKey, context?.previous);
      toast.error(errorMessage(error));
    },
    onSuccess: (_response, input) =>
      toast.success(
        t(input.enabled ? "flags.toast.enabled" : "flags.toast.disabled", { name: input.name }),
      ),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["admin", "flags"] }),
  });

  const remove = useMutation({
    mutationFn: (name: string) => flagClient().deleteFlag({ name }),
    onSuccess: () => {
      toast.success(t("flags.toast.deleted"));
      setDeleting(null);
      return queryClient.invalidateQueries({ queryKey: ["admin"] });
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  const columns: Column<admin.FeatureFlag>[] = [
    {
      key: "name",
      header: t("flags.columns.name"),
      card: "title",
      className: "w-2/5",
      cell: (row) => (
        <span className="flex min-w-0 flex-col">
          <code className="font-mono font-bold [overflow-wrap:anywhere]">{row.name}</code>
          {row.description && (
            <span className="line-clamp-2 text-xs font-normal text-ink-soft">
              {row.description}
            </span>
          )}
        </span>
      ),
    },
    {
      key: "enabled",
      header: t("flags.columns.enabled"),
      cell: (row) => (
        <Toggle
          label={t("flags.toggle", { name: row.name })}
          checked={row.enabled}
          onChange={(enabled) => toggle.mutate({ name: row.name, enabled })}
        />
      ),
    },
    {
      key: "rollout",
      header: t("flags.columns.rollout"),
      cell: (row) => (
        <span className="flex items-center gap-2">
          <svg className="w-20 shrink-0" height={8} aria-hidden>
            <rect width="100%" height={8} rx={4} className="fill-brand-soft" />
            {row.rolloutPercent > 0 && (
              <rect
                width={`${row.rolloutPercent}%`}
                height={8}
                rx={4}
                className={row.enabled ? "fill-brand" : "fill-ink-faint"}
              />
            )}
          </svg>
          <span className="font-bold tabular-nums">{row.rolloutPercent}%</span>
        </span>
      ),
    },
    {
      key: "allow",
      header: t("flags.columns.allowList"),
      cell: (row) =>
        row.allowUsers.length === 0 ? (
          <span className="text-ink-faint">—</span>
        ) : (
          <span className="flex items-center gap-2">
            <span className="flex -space-x-1.5">
              {row.allowUsers.slice(0, 4).map((user) => (
                <Avatar key={user.id} user={user} size="xs" />
              ))}
            </span>
            <span className="text-xs font-bold whitespace-nowrap text-ink-soft">
              {t("flags.allowCount", { count: row.allowUsers.length })}
            </span>
          </span>
        ),
    },
    {
      key: "updated",
      header: t("flags.columns.updated"),
      className: "whitespace-nowrap",
      cell: (row) => (
        <span title={formatDateTime(row.updatedAt)} className="text-ink-soft">
          {compactAge(toMillis(row.updatedAt))}
        </span>
      ),
    },
    {
      key: "actions",
      header: t("flags.columns.actions"),
      card: "bare",
      className: "text-right whitespace-nowrap",
      cell: (row) => (
        <span className="inline-flex gap-1">
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={t("flags.edit.open", { name: row.name })}
            onClick={() =>
              setEditor((current) => ({ flag: row, session: current.session + 1, open: true }))
            }
          >
            <Pencil className="size-4" />
          </Button>
          <Button
            size="icon-sm"
            variant="ghost"
            className="text-danger"
            aria-label={t("flags.delete.open", { name: row.name })}
            onClick={() => setDeleting(row)}
          >
            <Trash2 className="size-4" />
          </Button>
        </span>
      ),
    },
  ];

  const list = (
    <div className="flex min-w-0 flex-col gap-3">
      <SearchBox
        label={t("flags.search")}
        placeholder={t("flags.search")}
        value={typed.text}
        onChange={typed.setText}
        onCommit={typed.commit}
      />
      {flags.isPending ? (
        <RowsSkeleton />
      ) : flags.isError ? (
        <ErrorState error={flags.error} onRetry={() => flags.refetch()} />
      ) : rows.length === 0 ? (
        <EmptyState
          emoji="🚩"
          title={t(typed.query ? "flags.emptySearch" : "flags.empty")}
          body={typed.query ? undefined : t("flags.emptyBody")}
        />
      ) : (
        <>
          <DataTable
            label={t("flags.title")}
            columns={columns}
            rows={rows}
            rowKey={(row) => row.name}
          />
          <LoadMore query={flags} />
        </>
      )}
    </div>
  );
  const evaluate = (
    <EvaluatePanel userId={search.evaluate} onChange={(userId) => update({ evaluate: userId })} />
  );

  return (
    <ConsolePage
      title={t("flags.title")}
      actions={
        <Button
          size="sm"
          icon={<Plus className="size-4" />}
          onClick={() => setEditor((current) => ({ session: current.session + 1, open: true }))}
        >
          {t("flags.create.open")}
        </Button>
      }
    >
      {wide ? (
        <div className="grid items-start gap-4 lg:grid-cols-[minmax(0,1fr)_21rem]">
          {list}
          {evaluate}
        </div>
      ) : (
        <>
          <Tabs<"flags" | "evaluate">
            className="mb-3"
            value={tab}
            onChange={setTab}
            items={[
              { value: "flags", label: t("flags.tabs.flags") },
              { value: "evaluate", label: t("flags.tabs.evaluate") },
            ]}
          />
          {tab === "flags" ? list : evaluate}
        </>
      )}

      <FlagDialog
        key={editor.session}
        flag={editor.flag}
        open={editor.open}
        onOpenChange={(open) => setEditor((current) => ({ ...current, open }))}
      />
      <ConfirmDialog
        open={deleting !== null}
        onOpenChange={(open) => !open && setDeleting(null)}
        title={t("flags.delete.title", { name: deleting?.name })}
        description={t("flags.delete.description")}
        confirmLabel={t("flags.delete.confirm")}
        danger
        loading={remove.isPending}
        onConfirm={() => deleting && remove.mutate(deleting.name)}
      />
    </ConsolePage>
  );
}
