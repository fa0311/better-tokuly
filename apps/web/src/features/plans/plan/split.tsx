import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type common, plans, toMillis } from "@tokuly/api";
import { ArrowRight, ChevronDown, HandCoins, Plus, Undo2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { compactAge } from "#/lib/format";
import { useMe } from "#/stores/session";
import { keys, nameOf, planClient } from "../api";
import { formatMoney } from "../money";
import { SectionTitle } from "../parts";
import { ExpenseDialog } from "./expense-dialog";
import { canWork, isHostLike } from "./model";

/** Every expense of a plan (they come in pages; a plan holds at most 500). */
async function loadExpenses(planId: string): Promise<plans.Expense[]> {
  const expenses: plans.Expense[] = [];
  let cursor = "";
  do {
    const page = await planClient().listExpenses({ planId, page: { limit: 100, cursor } });
    expenses.push(...page.expenses);
    cursor = page.page?.nextCursor ?? "";
  } while (cursor);
  return expenses;
}

/**
 * 割り勘: who paid what, where everyone stands, and the fewest transfers
 * that settle it. A transfer is marked as received by the person who got the
 * money — nobody declares their own debt paid — and can be taken back by them.
 */
export function CostSplit({ plan }: { plan: plans.Plan }) {
  const { t, i18n } = useTranslation("plans");
  const me = useMe();
  const myId = me?.user?.id ?? "";
  const queryClient = useQueryClient();
  const expenses = useQuery({
    queryKey: keys.expenses(plan.id),
    queryFn: () => loadExpenses(plan.id),
  });
  const split = useQuery({
    queryKey: keys.split(plan.id),
    queryFn: () => planClient().getSplit({ planId: plan.id }),
  });
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<plans.Expense>();
  const [settling, setSettling] = useState<{ transfer: plans.Transfer; currency: string }>();
  const [undoing, setUndoing] = useState<plans.Settlement>();
  const [showBalances, setShowBalances] = useState(false);
  const [showHistory, setShowHistory] = useState(false);

  const people = new Map<string, common.UserSummary | undefined>(
    plan.participants.map((participant) => [participant.user?.id ?? "", participant.user]),
  );
  const name = (userId: string) => (userId === myId ? t("split.you") : nameOf(people.get(userId)));
  const money = (amount: bigint, currency: string) => formatMoney(amount, currency, i18n.language);
  const allowed = canWork(plan);
  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: keys.split(plan.id) });
  };
  const onError = (error: unknown) => {
    toast.error(errorMessage(error, { namespace: "plans" }));
    // What is owed may have changed under the button.
    refresh();
  };

  const settle = useMutation({
    mutationFn: ({ transfer, currency }: { transfer: plans.Transfer; currency: string }) =>
      planClient().settleTransfer({
        planId: plan.id,
        fromUserId: transfer.fromUserId,
        toUserId: transfer.toUserId,
        amount: transfer.amount,
        currency,
      }),
    onSuccess: () => {
      refresh();
      setSettling(undefined);
      toast.success(t("split.settled"));
    },
    onError: (error) => {
      setSettling(undefined);
      onError(error);
    },
  });
  const undo = useMutation({
    mutationFn: (settlement: plans.Settlement) =>
      planClient().removeSettlement({ settlementId: settlement.id }),
    onSuccess: () => {
      refresh();
      setUndoing(undefined);
      toast(t("split.undone"));
    },
    onError: (error) => {
      setUndoing(undefined);
      onError(error);
    },
  });

  const splits = split.data?.splits ?? [];
  const settlements = split.data?.settlements ?? [];
  const removed = split.data?.removedSettlements ?? [];
  const canEdit = (expense: plans.Expense) =>
    plan.status === plans.PlanStatus.ACTIVE &&
    (expense.createdByUserId === myId || expense.paidByUserId === myId || isHostLike(plan));

  return (
    <section>
      <SectionTitle
        action={
          allowed && (
            <Button
              variant="secondary"
              size="sm"
              icon={<Plus className="size-4" />}
              onClick={() => setAdding(true)}
            >
              {t("split.add")}
            </Button>
          )
        }
      >
        {t("split.title")}
      </SectionTitle>

      {expenses.isPending || split.isPending ? (
        <Skeleton className="h-28 rounded-card" />
      ) : (expenses.data?.length ?? 0) === 0 && settlements.length === 0 ? (
        <div className="rounded-card bg-surface px-4 py-6 text-center">
          <p className="text-3xl">🧾</p>
          <p className="mt-1 font-black">{t("split.empty")}</p>
          <p className="text-sm text-ink-soft">
            {allowed ? t("split.emptyBody") : t("split.goingOnly")}
          </p>
        </div>
      ) : (
        <div className="flex flex-col gap-3">
          {splits.map((currencySplit) => {
            const mine = currencySplit.balances.find((balance) => balance.userId === myId);
            const net = mine?.net ?? 0n;
            return (
              <div
                key={currencySplit.currency}
                className="sticker rounded-card bg-surface p-4"
                data-testid="split-summary"
              >
                <div className="flex items-baseline justify-between gap-2">
                  <p className="text-sm font-bold text-ink-soft">{t("split.total")}</p>
                  <p className="text-2xl font-black">
                    {money(currencySplit.total, currencySplit.currency)}
                  </p>
                </div>
                <p
                  className={cn(
                    "mt-1 rounded-2xl px-3 py-2 text-center font-black",
                    net > 0n
                      ? "bg-lime text-[#17122b]"
                      : net < 0n
                        ? "bg-sun text-[#17122b]"
                        : "bg-surface-2 text-ink-soft",
                  )}
                  data-testid="my-balance"
                >
                  {net > 0n
                    ? t("split.youGet", { amount: money(net, currencySplit.currency) })
                    : net < 0n
                      ? t("split.youOwe", { amount: money(-net, currencySplit.currency) })
                      : t("split.even")}
                </p>

                {currencySplit.transfers.length > 0 ? (
                  <ul className="mt-3 flex flex-col gap-2">
                    {currencySplit.transfers.map((transfer) => (
                      <li
                        key={`${transfer.fromUserId}-${transfer.toUserId}`}
                        className="flex flex-wrap items-center gap-x-2 gap-y-1.5 rounded-2xl bg-surface-2 px-3 py-2.5"
                        data-testid="transfer"
                      >
                        <span className="flex min-w-0 flex-1 items-center gap-1.5">
                          <Avatar user={people.get(transfer.fromUserId)} size="xs" />
                          <span className="truncate text-sm font-bold">
                            {name(transfer.fromUserId)}
                          </span>
                          <ArrowRight
                            className="size-4 shrink-0 text-ink-faint"
                            aria-label={t("split.to")}
                          />
                          <Avatar user={people.get(transfer.toUserId)} size="xs" />
                          <span className="truncate text-sm font-bold">
                            {name(transfer.toUserId)}
                          </span>
                        </span>
                        <span className="shrink-0 font-black">
                          {money(transfer.amount, currencySplit.currency)}
                        </span>
                        {transfer.toUserId === myId ? (
                          <Button
                            variant="lime"
                            size="sm"
                            className="w-full sm:w-auto"
                            icon={<HandCoins className="size-4" />}
                            onClick={() =>
                              setSettling({ transfer, currency: currencySplit.currency })
                            }
                          >
                            {t("split.received")}
                          </Button>
                        ) : (
                          transfer.fromUserId === myId && (
                            <span className="w-full text-xs font-bold text-ink-soft">
                              {t("split.payHint", { name: name(transfer.toUserId) })}
                            </span>
                          )
                        )}
                      </li>
                    ))}
                  </ul>
                ) : (
                  currencySplit.total > 0n && (
                    <p className="mt-3 text-center text-sm font-black text-success">
                      🎉 {t("split.allSettled")}
                    </p>
                  )
                )}

                <button
                  type="button"
                  aria-expanded={showBalances}
                  onClick={() => setShowBalances(!showBalances)}
                  className="mt-3 flex items-center gap-1 text-sm font-bold text-ink-soft hover:text-ink"
                >
                  {t("split.balances")}
                  <ChevronDown
                    className={cn("size-4 transition-transform", showBalances && "rotate-180")}
                  />
                </button>
                {showBalances && (
                  <ul className="mt-2 flex flex-col gap-1.5 text-sm">
                    {currencySplit.balances.map((balance) => (
                      <li key={balance.userId} className="flex items-center gap-2">
                        <Avatar user={people.get(balance.userId)} size="xs" />
                        <span className="min-w-0 flex-1 truncate font-bold">
                          {name(balance.userId)}
                        </span>
                        <span className="text-xs text-ink-soft">
                          {t("split.paidOwed", {
                            paid: money(balance.paid, currencySplit.currency),
                            owed: money(balance.owed, currencySplit.currency),
                          })}
                        </span>
                        <span
                          className={cn(
                            "w-20 shrink-0 text-right font-black",
                            balance.net > 0n
                              ? "text-success"
                              : balance.net < 0n
                                ? "text-danger"
                                : "text-ink-faint",
                          )}
                        >
                          {balance.net > 0n ? "+" : ""}
                          {money(balance.net, currencySplit.currency)}
                        </span>
                      </li>
                    ))}
                  </ul>
                )}
              </div>
            );
          })}

          <ul className="rounded-card bg-surface p-1.5">
            {(expenses.data ?? []).map((expense) => {
              const row = (
                <>
                  <span
                    className="grid size-10 shrink-0 place-items-center rounded-xl bg-surface-2 text-xl"
                    aria-hidden
                  >
                    🧾
                  </span>
                  <span className="min-w-0 flex-1 text-left">
                    <span className="block truncate font-bold">{expense.title}</span>
                    <span className="block truncate text-sm text-ink-soft">
                      {t("split.paidBy", {
                        name: name(expense.paidByUserId),
                        count: expense.shares.length,
                      })}
                    </span>
                  </span>
                  <span className="shrink-0 font-black">
                    {money(expense.amount, expense.currency)}
                  </span>
                </>
              );
              return (
                <li key={expense.id} data-testid="expense">
                  {canEdit(expense) ? (
                    <button
                      type="button"
                      onClick={() => setEditing(expense)}
                      className="flex w-full items-center gap-3 rounded-2xl px-2.5 py-2 transition-colors hover:bg-surface-2"
                    >
                      {row}
                    </button>
                  ) : (
                    <div className="flex items-center gap-3 px-2.5 py-2">{row}</div>
                  )}
                </li>
              );
            })}
          </ul>

          {(settlements.length > 0 || removed.length > 0) && (
            <div className="rounded-card bg-surface p-3">
              <button
                type="button"
                aria-expanded={showHistory}
                onClick={() => setShowHistory(!showHistory)}
                className="flex w-full items-center gap-1 text-sm font-bold text-ink-soft hover:text-ink"
              >
                {t("split.history", { count: settlements.length })}
                <ChevronDown
                  className={cn("size-4 transition-transform", showHistory && "rotate-180")}
                />
              </button>
              {showHistory && (
                <ul className="mt-2 flex flex-col gap-2 text-sm">
                  {settlements.map((settlement) => (
                    <li
                      key={settlement.id}
                      className="flex items-center gap-2"
                      data-testid="settlement"
                    >
                      <span className="min-w-0 flex-1">
                        <span className="font-bold">
                          ✅{" "}
                          {t("split.settlement", {
                            to: name(settlement.toUserId),
                            from: name(settlement.fromUserId),
                            amount: money(settlement.amount, settlement.currency),
                          })}
                        </span>
                        <span className="ml-1.5 text-xs text-ink-faint">
                          {compactAge(toMillis(settlement.createdAt))}
                        </span>
                      </span>
                      {settlement.toUserId === myId && (
                        <Button
                          variant="ghost"
                          size="sm"
                          className="shrink-0"
                          icon={<Undo2 className="size-4" />}
                          onClick={() => setUndoing(settlement)}
                        >
                          {t("split.undo")}
                        </Button>
                      )}
                    </li>
                  ))}
                  {removed.map((settlement) => (
                    <li
                      key={settlement.id}
                      className="text-ink-faint"
                      data-testid="removed-settlement"
                    >
                      <span className="line-through">
                        {t("split.settlement", {
                          to: name(settlement.toUserId),
                          from: name(settlement.fromUserId),
                          amount: money(settlement.amount, settlement.currency),
                        })}
                      </span>
                      <span className="ml-1.5 text-xs">
                        {t("split.undoneBy", { name: name(settlement.removedByUserId) })}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}
        </div>
      )}

      <ExpenseDialog plan={plan} open={adding} onOpenChange={setAdding} />
      <ExpenseDialog
        plan={plan}
        expense={editing}
        open={Boolean(editing)}
        onOpenChange={(open) => !open && setEditing(undefined)}
      />
      <ConfirmDialog
        open={Boolean(settling)}
        onOpenChange={(open) => !open && setSettling(undefined)}
        title={t("split.receivedTitle")}
        description={
          settling
            ? t("split.receivedBody", {
                name: name(settling.transfer.fromUserId),
                amount: money(settling.transfer.amount, settling.currency),
              })
            : undefined
        }
        confirmLabel={t("split.received")}
        loading={settle.isPending}
        onConfirm={() => settling && settle.mutate(settling)}
      />
      <ConfirmDialog
        open={Boolean(undoing)}
        onOpenChange={(open) => !open && setUndoing(undefined)}
        title={t("split.undoTitle")}
        description={
          undoing
            ? t("split.undoBody", {
                name: name(undoing.fromUserId),
                amount: money(undoing.amount, undoing.currency),
              })
            : undefined
        }
        confirmLabel={t("split.undo")}
        danger
        loading={undo.isPending}
        onConfirm={() => undoing && undo.mutate(undoing)}
      />
    </section>
  );
}
