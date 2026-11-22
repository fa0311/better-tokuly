import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { search } from "@tokuly/api";
import { RefreshCw, RotateCw } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { searchAdminClient, searchKeys } from "../api";
import { isKind, kindSlug } from "../kinds";
import { formatBytes, formatDateTime, Pane, staffErrorMessage, type Tone, ToneChip } from "./parts";

/** How long the page keeps asking for fresh numbers after a rebuild was started. */
const WATCH_MS = 90_000;
const WATCH_INTERVAL_MS = 2000;

type Stats = Awaited<ReturnType<ReturnType<typeof searchAdminClient>["getIndexStats"]>>;

/** Something is still being written to an index. */
const busy = (stats: Stats | undefined) =>
  stats?.indexes.some((index) => index.indexing || index.pendingDocuments > 0n) ?? false;

/**
 * Every index next to what it should contain, and the button that rebuilds
 * one. The search domain sends no events, so while a rebuild is under way the
 * numbers are asked for again every couple of seconds — and only then.
 */
export function Indexes() {
  const { t } = useTranslation("search");
  const queryClient = useQueryClient();
  const [watchUntil, setWatchUntil] = useState(0);
  const [rebuilding, setRebuilding] = useState<search.IndexStats>();
  const stats = useQuery({
    queryKey: searchKeys.admin.stats,
    queryFn: () => searchAdminClient().getIndexStats({}),
    refetchInterval: (query) =>
      busy(query.state.data) || Date.now() < watchUntil ? WATCH_INTERVAL_MS : false,
  });
  const reindex = useMutation({
    mutationFn: (kind: search.SearchKind) => searchAdminClient().reindex({ kind }),
    onSuccess: (response) => {
      toast.success(t("staff.indexes.queued", { count: Number(response.queuedDocuments) }));
      setWatchUntil(Date.now() + WATCH_MS);
      return queryClient.invalidateQueries({ queryKey: searchKeys.admin.stats });
    },
    onError: (error) => toast.error(staffErrorMessage(error)),
    onSettled: () => setRebuilding(undefined),
  });

  if (stats.isPending) {
    return (
      <div className="flex flex-col gap-2" aria-busy>
        <Skeleton className="h-16" />
        <Skeleton className="h-64" />
      </div>
    );
  }
  if (stats.isError) return <EmptyState emoji="🌀" title={staffErrorMessage(stats.error)} />;
  const data = stats.data;
  const kindName = (index: search.IndexStats) =>
    isKind(index.kind) ? t(`kinds.${kindSlug(index.kind)}`) : index.indexUid;

  return (
    <div className="flex flex-col gap-4">
      <Pane>
        <dl className="flex flex-wrap items-center gap-x-6 gap-y-2 px-4 py-3 text-sm">
          <div className="flex items-center gap-2">
            <dt className="text-xs font-bold text-ink-faint">{t("staff.indexes.engine")}</dt>
            <dd>
              <ToneChip tone={data.healthy ? "good" : "danger"}>
                {t(data.healthy ? "staff.indexes.healthy" : "staff.indexes.unhealthy")}
              </ToneChip>
            </dd>
          </div>
          {data.engineVersion && (
            <div className="flex items-center gap-2">
              <dt className="text-xs font-bold text-ink-faint">{t("staff.indexes.version")}</dt>
              <dd className="font-mono text-xs">{data.engineVersion}</dd>
            </div>
          )}
          {data.healthy && (
            <div className="flex items-center gap-2">
              <dt className="text-xs font-bold text-ink-faint">{t("staff.indexes.size")}</dt>
              <dd className="font-bold tabular-nums">{formatBytes(data.databaseSizeBytes)}</dd>
            </div>
          )}
          <Button
            className="ml-auto"
            size="sm"
            variant="secondary"
            icon={<RefreshCw className={cn("size-4", stats.isFetching && "animate-spin")} />}
            onClick={() => void stats.refetch()}
          >
            {t("staff.indexes.refresh")}
          </Button>
        </dl>
        {!data.healthy && (
          <p className="border-t-2 border-line px-4 py-3 text-sm font-bold text-danger">
            {t("staff.indexes.unhealthyHint")}
          </p>
        )}
      </Pane>

      <Pane title={t("staff.indexes.title")}>
        <ul aria-label={t("staff.indexes.title")} className="divide-y-2 divide-line">
          {data.indexes.map((index) => {
            const registered = Number(index.registeredDocuments);
            const indexed = Number(index.indexedDocuments);
            const pending = Number(index.pendingDocuments);
            const working = index.indexing || pending > 0;
            const [tone, state]: [Tone, string] = !data.healthy
              ? ["neutral", "unknown"]
              : !index.exists
                ? ["warn", "missing"]
                : working
                  ? ["info", "working"]
                  : ["good", "synced"];
            const done =
              registered > 0 ? Math.min(1, Math.max(0, (registered - pending) / registered)) : 1;
            return (
              <li
                key={index.indexUid}
                data-testid="index-row"
                className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3 text-sm lg:grid lg:grid-cols-[minmax(0,1.2fr)_minmax(0,1.4fr)_7rem_minmax(0,1fr)_auto] lg:gap-3"
              >
                <div className="min-w-0 basis-full lg:basis-auto">
                  <p className="truncate font-black">{kindName(index)}</p>
                  <p className="truncate font-mono text-xs text-ink-faint">{index.indexUid}</p>
                </div>
                <div className="min-w-0 flex-1 basis-40">
                  <p className="font-bold tabular-nums">
                    {t("staff.indexes.documents", {
                      indexed: data.healthy ? indexed.toLocaleString() : "—",
                      registered: registered.toLocaleString(),
                    })}
                    {pending > 0 && (
                      <span className="ml-2 text-xs text-ink-soft">
                        {t("staff.indexes.pending", { count: pending })}
                      </span>
                    )}
                  </p>
                  <div
                    className="mt-1 h-2 overflow-hidden rounded-full bg-surface-3"
                    role="progressbar"
                    aria-label={t("staff.indexes.progress", { kind: kindName(index) })}
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={Math.round(done * 100)}
                  >
                    <div
                      className={cn(
                        "h-full rounded-full transition-[width] duration-500",
                        working ? "bg-sky" : "bg-lime",
                      )}
                      style={{ width: `${done * 100}%` }}
                    />
                  </div>
                </div>
                <ToneChip tone={tone} className="justify-self-start">
                  {t(`staff.indexes.state.${state}`)}
                </ToneChip>
                <p className="min-w-0 text-xs text-ink-soft">
                  <span className="lg:hidden">{t("staff.indexes.lastChanged")}: </span>
                  {formatDateTime(index.lastChangedAt)}
                </p>
                <Button
                  size="sm"
                  variant="secondary"
                  icon={<RotateCw className="size-4" />}
                  onClick={() => setRebuilding(index)}
                  aria-label={t("staff.indexes.reindexKind", { kind: kindName(index) })}
                >
                  {t("staff.indexes.reindex")}
                </Button>
              </li>
            );
          })}
        </ul>
      </Pane>

      <ConfirmDialog
        open={rebuilding !== undefined}
        onOpenChange={(open) => !open && setRebuilding(undefined)}
        title={t("staff.indexes.reindexTitle", { kind: rebuilding ? kindName(rebuilding) : "" })}
        description={t("staff.indexes.reindexBody")}
        confirmLabel={t("staff.indexes.reindex")}
        loading={reindex.isPending}
        onConfirm={() => rebuilding && reindex.mutate(rebuilding.kind)}
      />
    </div>
  );
}
