import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { feed } from "@tokuly/api";
import { CircleAlert, MoreHorizontal, Pencil, Plus, RefreshCw, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { useStaffAccess } from "#/features/admin";
import { cn } from "#/lib/cn";
import { newsAdminClient, newsKeys } from "../api";
import { SourceIcon } from "../parts";
import { formatDateTime, Pane, StaffPage, staffErrorMessage, ToneChip } from "./parts";
import { RemoveArticle } from "./remove-article";
import { SourceDialog } from "./source-dialog";

/** How long the page keeps asking for fresh fetch states after a refresh was queued. */
const WATCH_MS = 45_000;
const WATCH_INTERVAL_MS = 2000;

/**
 * `/admin/news`: the feeds articles come from (add, change, switch off,
 * remove, fetch now) and taking a single article down. Sources need the
 * `admin` role; moderators can take articles down.
 */
export function AdminNewsScreen() {
  const { t } = useTranslation("news");
  const access = useStaffAccess();
  const [adding, setAdding] = useState(false);
  const [watchUntil, setWatchUntil] = useState(0);
  const queryClient = useQueryClient();
  const refreshAll = useMutation({
    mutationFn: () => newsAdminClient().refreshNewsSources({}),
    onSuccess: (response) => {
      toast.success(t("staff.sources.refreshQueued", { count: response.queuedCount }));
      setWatchUntil(Date.now() + WATCH_MS);
      return queryClient.invalidateQueries({ queryKey: newsKeys.admin.sources });
    },
    onError: (error) => toast.error(staffErrorMessage(error)),
  });

  if (!access.moderator) {
    return (
      <StaffPage title={t("staff.title")}>
        <EmptyState emoji="🔐" title={t("staff.gate.title")} body={t("staff.gate.body")} />
      </StaffPage>
    );
  }
  return (
    <StaffPage
      title={t("staff.title")}
      actions={
        access.admin && (
          <>
            <Button
              size="sm"
              variant="secondary"
              icon={<RefreshCw className="size-4" />}
              loading={refreshAll.isPending}
              onClick={() => refreshAll.mutate()}
            >
              <span className="hidden sm:inline">{t("staff.sources.refreshAll")}</span>
              <span className="sm:hidden">{t("staff.sources.refreshAllShort")}</span>
            </Button>
            <Button size="sm" icon={<Plus className="size-4" />} onClick={() => setAdding(true)}>
              {t("staff.sources.add")}
            </Button>
          </>
        )
      }
    >
      <div className="grid grid-cols-1 items-start gap-4 2xl:grid-cols-[minmax(0,1fr)_24rem]">
        {access.admin && (
          <Sources
            watchUntil={watchUntil}
            onWatch={() => setWatchUntil(Date.now() + WATCH_MS)}
            onAdd={() => setAdding(true)}
          />
        )}
        <RemoveArticle />
      </div>
      {adding && (
        <SourceDialog
          onClose={() => setAdding(false)}
          onSaved={() => setWatchUntil(Date.now() + WATCH_MS)}
        />
      )}
    </StaffPage>
  );
}

function Sources({
  watchUntil,
  onWatch,
  onAdd,
}: {
  watchUntil: number;
  onWatch(): void;
  onAdd(): void;
}) {
  const { t } = useTranslation("news");
  const queryClient = useQueryClient();
  const [editing, setEditing] = useState<feed.ManagedNewsSource>();
  const [deleting, setDeleting] = useState<feed.ManagedNewsSource>();
  const sources = useQuery({
    queryKey: newsKeys.admin.sources,
    queryFn: () => newsAdminClient().listManagedNewsSources({}),
    // Fetches run in the background and announce nothing: follow them for a while after one was queued.
    refetchInterval: () => (Date.now() < watchUntil ? WATCH_INTERVAL_MS : false),
  });
  const refreshLists = () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: newsKeys.admin.sources }),
      queryClient.invalidateQueries({ queryKey: newsKeys.sources }),
      queryClient.invalidateQueries({ queryKey: newsKeys.lists }),
    ]);
  const refresh = useMutation({
    mutationFn: (source: feed.ManagedNewsSource) =>
      newsAdminClient().refreshNewsSources({ sourceId: source.id }),
    onSuccess: (_response, source) => {
      toast.success(t("staff.sources.refreshOne", { name: source.name }));
      onWatch();
      return queryClient.invalidateQueries({ queryKey: newsKeys.admin.sources });
    },
    onError: (error) => toast.error(staffErrorMessage(error)),
  });
  const toggle = useMutation({
    mutationFn: (source: feed.ManagedNewsSource) =>
      newsAdminClient().updateNewsSource({ sourceId: source.id, enabled: !source.enabled }),
    onSuccess: (response) => {
      toast.success(
        t(response.source?.enabled ? "staff.sources.switchedOn" : "staff.sources.switchedOff"),
      );
      if (response.source?.enabled) onWatch();
      return refreshLists();
    },
    onError: (error) => toast.error(staffErrorMessage(error)),
  });
  const remove = useMutation({
    mutationFn: (source: feed.ManagedNewsSource) =>
      newsAdminClient().deleteNewsSource({ sourceId: source.id }),
    onSuccess: () => {
      toast.success(t("staff.sources.deleted"));
      return refreshLists();
    },
    onError: (error) => toast.error(staffErrorMessage(error)),
    onSettled: () => setDeleting(undefined),
  });
  const list = sources.data?.sources ?? [];

  return (
    <Pane
      title={t("staff.sources.title")}
      aside={
        list.length > 0 && (
          <span className="text-xs font-bold text-ink-faint tabular-nums">
            {t("staff.sources.count", { count: list.length })}
          </span>
        )
      }
    >
      {sources.isPending ? (
        <div className="flex flex-col gap-2 p-4" aria-busy>
          <Skeleton className="h-20" />
          <Skeleton className="h-20" />
        </div>
      ) : sources.isError ? (
        <EmptyState emoji="🌀" title={staffErrorMessage(sources.error)} />
      ) : list.length === 0 ? (
        <EmptyState
          emoji="📡"
          title={t("staff.sources.empty")}
          body={t("staff.sources.emptyBody")}
          action={
            <Button icon={<Plus className="size-4" />} onClick={onAdd}>
              {t("staff.sources.add")}
            </Button>
          }
        />
      ) : (
        <ul aria-label={t("staff.sources.title")} className="divide-y-2 divide-line">
          {list.map((source) => (
            <li
              key={source.id}
              data-testid="news-source"
              className={cn("flex flex-col gap-2 px-4 py-3", !source.enabled && "bg-surface-2/50")}
            >
              <div className="flex items-center gap-3">
                <span className="grid size-10 shrink-0 place-items-center rounded-xl bg-surface-2">
                  <SourceIcon source={source} className="size-5" />
                </span>
                <div className="min-w-0 flex-1">
                  <p className="flex min-w-0 items-center gap-2">
                    <span className="truncate font-black">{source.name}</span>
                    {source.language && (
                      <span className="shrink-0 rounded bg-surface-2 px-1 text-[11px] font-bold text-ink-soft uppercase">
                        {source.language}
                      </span>
                    )}
                    {source.fromConfig && (
                      <span className="shrink-0 rounded bg-surface-2 px-1 text-[11px] font-bold text-ink-soft">
                        {t("staff.sources.fromConfig")}
                      </span>
                    )}
                  </p>
                  <p className="truncate font-mono text-xs text-ink-faint">{source.feedUrl}</p>
                </div>
                <FetchState source={source} />
                <Menu
                  trigger={
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      aria-label={t("staff.sources.menu", { name: source.name })}
                    >
                      <MoreHorizontal className="size-5" />
                    </Button>
                  }
                  actions={[
                    {
                      label: t("staff.sources.refreshNow"),
                      icon: <RefreshCw />,
                      disabled: !source.enabled,
                      onSelect: () => refresh.mutate(source),
                    },
                    {
                      label: t("staff.sources.edit"),
                      icon: <Pencil />,
                      onSelect: () => setEditing(source),
                    },
                    {
                      label: t(
                        source.enabled ? "staff.sources.switchOff" : "staff.sources.switchOn",
                      ),
                      icon: <span />,
                      onSelect: () => toggle.mutate(source),
                    },
                    {
                      label: t("staff.sources.delete"),
                      icon: <Trash2 />,
                      danger: true,
                      onSelect: () => setDeleting(source),
                    },
                  ]}
                />
              </div>
              <dl className="flex flex-wrap gap-x-5 gap-y-1 pl-[3.25rem] text-xs text-ink-soft">
                <div className="flex gap-1.5">
                  <dt className="font-bold text-ink-faint">{t("staff.sources.articles")}</dt>
                  <dd className="font-bold tabular-nums">{source.articleCount.toLocaleString()}</dd>
                </div>
                <div className="flex gap-1.5">
                  <dt className="font-bold text-ink-faint">{t("staff.sources.lastSuccess")}</dt>
                  <dd>{formatDateTime(source.lastSuccessAt)}</dd>
                </div>
                {source.enabled && (
                  <div className="flex gap-1.5">
                    <dt className="font-bold text-ink-faint">{t("staff.sources.nextFetch")}</dt>
                    <dd>{formatDateTime(source.nextFetchAt)}</dd>
                  </div>
                )}
                {source.defaultTags.length > 0 && (
                  <div className="flex min-w-0 gap-1.5">
                    <dt className="font-bold text-ink-faint">{t("staff.sources.tags")}</dt>
                    <dd className="truncate">
                      {source.defaultTags.map((tag) => `#${tag}`).join(" ")}
                    </dd>
                  </div>
                )}
              </dl>
              {source.lastError && (
                <p className="ml-[3.25rem] flex items-start gap-1.5 rounded-xl bg-danger/10 px-2.5 py-1.5 text-xs font-bold break-words text-danger">
                  <CircleAlert className="mt-px size-3.5 shrink-0" />
                  <span className="min-w-0">
                    {source.lastError}
                    {source.consecutiveFailures > 1 &&
                      ` (${t("staff.sources.failures", { count: source.consecutiveFailures })})`}
                  </span>
                </p>
              )}
            </li>
          ))}
        </ul>
      )}
      {editing && (
        <SourceDialog
          key={editing.id}
          source={editing}
          onClose={() => setEditing(undefined)}
          onSaved={onWatch}
        />
      )}
      <ConfirmDialog
        open={deleting !== undefined}
        onOpenChange={(open) => !open && setDeleting(undefined)}
        title={t("staff.sources.deleteTitle", { name: deleting?.name ?? "" })}
        description={t("staff.sources.deleteBody", { count: deleting?.articleCount ?? 0 })}
        confirmLabel={t("staff.sources.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => deleting && remove.mutate(deleting)}
      />
    </Pane>
  );
}

/** Where a source stands: switched off, never fetched, failing, or fine. */
function FetchState({ source }: { source: feed.ManagedNewsSource }) {
  const { t } = useTranslation("news");
  if (!source.enabled) return <ToneChip>{t("staff.sources.state.off")}</ToneChip>;
  if (source.lastError)
    return <ToneChip tone="danger">{t("staff.sources.state.failing")}</ToneChip>;
  if (!source.lastFetchedAt)
    return <ToneChip tone="info">{t("staff.sources.state.waiting")}</ToneChip>;
  return <ToneChip tone="good">{t("staff.sources.state.ok")}</ToneChip>;
}
