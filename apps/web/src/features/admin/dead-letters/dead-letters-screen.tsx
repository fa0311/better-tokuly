import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { type admin, toMillis } from "@tokuly/api";
import { RotateCcw } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { EmptyState } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { compactAge } from "#/lib/format";
import { systemClient } from "../api";
import { formatDateTime } from "../format";
import { ConsolePage } from "../ui/console-page";
import { type Column, DataTable } from "../ui/data-table";
import { SearchBox } from "../ui/filters";
import { useSearchText } from "../ui/hooks";
import { ErrorState, RowsSkeleton } from "../ui/states";
import type { DeadLettersSearch } from "./dead-letters-search";

/** `/admin/dead-letters`: what the event consumers and job workers gave up on, and sending it again. */
export function DeadLettersScreen({ search }: { search: DeadLettersSearch }) {
  const { t } = useTranslation("admin");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const typed = useSearchText(search.consumer, (consumer) =>
    navigate({ to: "/admin/dead-letters", search: { consumer }, replace: true }),
  );
  const prefix = typed.query ?? "";
  const [confirming, setConfirming] = useState(false);

  const letters = useQuery({
    queryKey: ["admin", "deadLetters", prefix],
    queryFn: () => systemClient().listDeadLetters({ consumerPrefix: prefix }),
  });
  const rows = letters.data?.letters ?? [];
  const total = Number(letters.data?.total ?? 0n);

  const redrive = useMutation({
    mutationFn: () => systemClient().redriveDeadLetters({ consumerPrefix: prefix }),
    onSuccess: (done) => {
      const counts = { sent: Number(done.sent), failed: Number(done.failed) };
      if (counts.failed > 0) toast.warning(t("deadLetters.redrive.partly", counts));
      else toast.success(t("deadLetters.redrive.done", counts));
      setConfirming(false);
      return queryClient.invalidateQueries({ queryKey: ["admin"] });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "admin" })),
  });

  const columns: Column<admin.DeadLetter>[] = [
    {
      key: "subject",
      header: t("deadLetters.columns.subject"),
      card: "title",
      cell: (row) => (
        <span className="flex min-w-0 flex-col">
          <code className="font-mono font-bold [overflow-wrap:anywhere]">{row.subject}</code>
          {row.occurredAt && (
            <span className="text-xs font-normal text-ink-soft">
              {t("deadLetters.occurredAt", { time: formatDateTime(row.occurredAt) })}
            </span>
          )}
        </span>
      ),
    },
    {
      key: "consumer",
      header: t("deadLetters.columns.consumer"),
      cell: (row) => (
        <code className="font-mono text-xs [overflow-wrap:anywhere]">{row.consumer}</code>
      ),
    },
    {
      key: "error",
      header: t("deadLetters.columns.error"),
      className: "w-2/5",
      cell: (row) => (
        <span
          className="line-clamp-3 text-xs text-ink-soft [overflow-wrap:anywhere]"
          title={row.error}
        >
          {row.error}
        </span>
      ),
    },
    {
      key: "deliveries",
      header: t("deadLetters.columns.deliveries"),
      className: "whitespace-nowrap",
      cell: (row) => (
        <span className="tabular-nums">
          {t("deadLetters.deliveries", { count: Number(row.deliveries) })}
        </span>
      ),
    },
    {
      key: "diedAt",
      header: t("deadLetters.columns.diedAt"),
      className: "whitespace-nowrap",
      cell: (row) => (
        <span title={formatDateTime(row.diedAt)} className="text-ink-soft">
          {compactAge(toMillis(row.diedAt))}
        </span>
      ),
    },
  ];

  return (
    <ConsolePage
      title={t("deadLetters.title")}
      actions={
        <Button
          size="sm"
          icon={<RotateCcw className="size-4" />}
          disabled={rows.length === 0}
          onClick={() => setConfirming(true)}
        >
          {t("deadLetters.redrive.open")}
        </Button>
      }
    >
      <div className="flex min-w-0 flex-col gap-3">
        <p className="text-sm text-ink-soft">{t("deadLetters.intro")}</p>
        <SearchBox
          label={t("deadLetters.filter")}
          placeholder={t("deadLetters.filterPlaceholder")}
          value={typed.text}
          onChange={typed.setText}
          onCommit={typed.commit}
        />
        {letters.isPending ? (
          <RowsSkeleton />
        ) : letters.isError ? (
          <ErrorState error={letters.error} onRetry={() => letters.refetch()} />
        ) : rows.length === 0 ? (
          <EmptyState
            emoji="📭"
            title={t(prefix ? "deadLetters.emptyFiltered" : "deadLetters.empty")}
            body={prefix ? undefined : t("deadLetters.emptyBody")}
          />
        ) : (
          <>
            {/* The total is of every consumer: beside a filtered list it would mislead. */}
            {!prefix && (
              <p className="text-xs font-bold text-ink-soft">
                {rows.length < total
                  ? t("deadLetters.shown", { shown: rows.length, count: total })
                  : t("deadLetters.total", { count: total })}
              </p>
            )}
            <DataTable
              label={t("deadLetters.title")}
              columns={columns}
              rows={rows}
              rowKey={(row) => String(row.sequence)}
            />
          </>
        )}
      </div>

      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t("deadLetters.redrive.title")}
        description={
          prefix ? t("deadLetters.redrive.filtered", { prefix }) : t("deadLetters.redrive.all")
        }
        confirmLabel={t("deadLetters.redrive.confirm")}
        loading={redrive.isPending}
        onConfirm={() => redrive.mutate()}
      />
    </ConsolePage>
  );
}
