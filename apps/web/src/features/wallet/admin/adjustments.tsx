import { useInfiniteQuery } from "@tanstack/react-query";
import { toDate, wallet } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { Coins } from "#/components/ui/coins";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { PAGE_SIZE, walletAdminClient, walletKeys } from "../api";
import { uniqueById } from "../ledger";
import { Cell, ColumnHeads, Pane, RECORD_ROW } from "./parts";

/** The log of every manual balance correction, newest first (admins only). */
export function AdjustmentsLog({ onOpenUser }: { onOpenUser(userId: string): void }) {
  const { t } = useTranslation("wallet");
  const query = useInfiniteQuery({
    queryKey: walletKeys.admin.adjustments(""),
    queryFn: ({ pageParam }) => walletAdminClient().listAdjustments({ page: { limit: PAGE_SIZE, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const adjustments = uniqueById(query.data?.pages.flatMap((page) => page.adjustments) ?? []);

  return (
    <Pane title={t("staff.adjustments.title")}>
      {query.isPending ? (
        <div className="flex flex-col gap-2 p-3">
          {[0, 1, 2].map((row) => (
            <Skeleton key={row} className="h-12" />
          ))}
        </div>
      ) : query.isError ? (
        <EmptyState emoji="😵‍💫" title={errorMessage(query.error)} />
      ) : adjustments.length > 0 ? (
        <>
          <AdjustmentRows adjustments={adjustments} showUser onOpenUser={onOpenUser} />
          <LoadMore query={query} />
        </>
      ) : (
        <EmptyState emoji="🧾" title={t("staff.adjustments.empty")} body={t("staff.adjustments.emptyBody")} />
      )}
    </Pane>
  );
}

/** Corrections as a table on wide screens and as labelled cards on narrow ones (one set of rows, two layouts). */
export function AdjustmentRows({ adjustments, showUser, onOpenUser }: { adjustments: wallet.Adjustment[]; /** Off inside one user's wallet, where every row is theirs. */ showUser: boolean; onOpenUser?(userId: string): void }) {
  const { t, i18n } = useTranslation("wallet");
  const columns = showUser ? "9.5rem minmax(0,1.2fr) 4.5rem 7rem minmax(0,2fr) minmax(0,1fr)" : "9.5rem 4.5rem 7rem minmax(0,2fr) minmax(0,1fr)";
  const heads = [t("staff.adjustments.when"), ...(showUser ? [t("staff.adjustments.user")] : []), t("staff.adjustments.account"), t("staff.adjustments.delta"), t("staff.adjustments.reason"), t("staff.adjustments.admin")];

  return (
    <div>
      <ColumnHeads columns={columns} heads={heads} />
      <ul className="divide-y-2 divide-line">
        {adjustments.map((adjustment) => (
          <li key={adjustment.id} className={RECORD_ROW} style={{ gridTemplateColumns: columns }}>
            <Cell label={t("staff.adjustments.when")}>{toDate(adjustment.createdAt)?.toLocaleString(i18n.language, { dateStyle: "short", timeStyle: "short" })}</Cell>
            {showUser && (
              <Cell label={t("staff.adjustments.user")}>
                {adjustment.user && !adjustment.user.deleted ? (
                  <button type="button" className="max-w-full truncate font-black underline decoration-2 underline-offset-2" onClick={() => adjustment.user && onOpenUser?.(adjustment.user.id)}>
                    @{adjustment.user.handle}
                  </button>
                ) : (
                  <span className="text-ink-soft">{t("staff.deletedUser")}</span>
                )}
              </Cell>
            )}
            <Cell label={t("staff.adjustments.account")}>{t(adjustment.account === wallet.LedgerAccount.EARNINGS ? "history.filters.earnings" : "history.filters.coins")}</Cell>
            <Cell label={t("staff.adjustments.delta")}>
              <Coins amount={adjustment.delta} signed size="sm" />
            </Cell>
            <Cell full label={t("staff.adjustments.reason")}>
              <span className="whitespace-pre-wrap">{adjustment.reason}</span>
            </Cell>
            <Cell label={t("staff.adjustments.admin")}>
              <span className="text-ink-soft">{adjustment.admin ? `@${adjustment.admin.handle}` : "—"}</span>
            </Cell>
          </li>
        ))}
      </ul>
    </div>
  );
}
