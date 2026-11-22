import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type common, plans } from "@tokuly/api";
import { Check, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { SelectField } from "#/components/ui/select";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { keys, nameOf, planClient } from "../api";
import { formatMoney, moneyInputValue, parseMoney } from "../money";

type Method = "equal" | "exact" | "shares";
const METHODS: Record<Method, plans.SplitMethod> = { equal: plans.SplitMethod.EQUAL, exact: plans.SplitMethod.EXACT, shares: plans.SplitMethod.SHARES };
const methodOf = (method: plans.SplitMethod): Method => (method === plans.SplitMethod.EXACT ? "exact" : method === plans.SplitMethod.SHARES ? "shares" : "equal");

interface Draft {
  title: string;
  amount: string;
  payerId: string;
  method: Method;
  /** Who the expense was for. */
  included: string[];
  /** Per person: the amount typed (exact) or the weight (shares). */
  exact: Record<string, string>;
  weights: Record<string, string>;
}

/**
 * Records who paid what and how it is divided: equally, by exact amounts, or
 * by weights (2 : 1 : 1). Only people who are going can pay or hold a share.
 */
export function ExpenseDialog({
  plan,
  expense,
  open,
  onOpenChange,
}: {
  plan: plans.Plan;
  /** The expense being changed; unset for a new one. */
  expense?: plans.Expense;
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t, i18n } = useTranslation("plans");
  const me = useMe();
  const queryClient = useQueryClient();
  const currency = expense?.currency || plan.currency || "JPY";
  const myId = me?.user?.id ?? "";

  // Whoever is going, plus anyone already part of this expense (they may stay in it).
  const inExpense = new Set([expense?.paidByUserId ?? "", ...(expense?.shares.map((share) => share.userId) ?? [])]);
  const people = plan.participants.flatMap((participant) => (participant.user && (participant.rsvp === plans.Rsvp.GOING || inExpense.has(participant.user.id)) ? [participant.user] : []));

  const initial = (): Draft =>
    expense
      ? {
          title: expense.title,
          amount: moneyInputValue(expense.amount, currency),
          payerId: expense.paidByUserId,
          method: methodOf(expense.method),
          included: expense.shares.map((share) => share.userId),
          exact: Object.fromEntries(expense.shares.map((share) => [share.userId, moneyInputValue(share.amount, currency)])),
          weights: Object.fromEntries(expense.shares.map((share) => [share.userId, String(share.weight || 1)])),
        }
      : { title: "", amount: "", payerId: people.some((user) => user.id === myId) ? myId : (people[0]?.id ?? ""), method: "equal", included: people.map((user) => user.id), exact: {}, weights: {} };
  const [draft, setDraft] = useState<Draft>(initial);
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) setDraft(initial());
  }
  const set = (patch: Partial<Draft>) => setDraft((current) => ({ ...current, ...patch }));

  const amount = parseMoney(draft.amount, currency);
  const included = people.filter((user) => draft.included.includes(user.id));
  const exactAmounts = included.map((user) => (draft.exact[user.id]?.trim() ? parseMoney(draft.exact[user.id]!, currency) : 0n));
  const exactTotal = exactAmounts.reduce<bigint>((sum, value) => sum + (value ?? 0n), 0n);
  const weights = included.map((user) => Number(draft.weights[user.id] ?? "1"));
  const problem: string | null = !draft.title.trim()
    ? t("split.form.errors.title")
    : amount === null
      ? t("split.form.errors.amount")
      : included.length === 0
        ? t("split.form.errors.people")
        : draft.method === "exact" && (exactAmounts.some((value) => value === null) || exactTotal !== amount)
          ? t("split.form.errors.exact", { rest: formatMoney(amount - exactTotal, currency, i18n.language) })
          : draft.method === "shares" && weights.some((weight) => !Number.isInteger(weight) || weight < 1 || weight > 1000)
            ? t("split.form.errors.weights")
            : null;

  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: keys.expenses(plan.id) });
    void queryClient.invalidateQueries({ queryKey: keys.split(plan.id) });
    void queryClient.invalidateQueries({ queryKey: keys.plan(plan.id) });
  };
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: "plans" }));
  const save = useMutation({
    mutationFn: async () => {
      const input = {
        title: draft.title.trim(),
        amount: amount ?? 0n,
        currency,
        paidByUserId: draft.payerId,
        method: METHODS[draft.method],
        shares: included.map((user, index) => ({ userId: user.id, amount: draft.method === "exact" ? (exactAmounts[index] ?? 0n) : 0n, weight: draft.method === "shares" ? weights[index]! : 0 })),
      };
      if (expense) await planClient().updateExpense({ expenseId: expense.id, expense: input });
      else await planClient().addExpense({ planId: plan.id, expense: input });
    },
    onSuccess: () => {
      refresh();
      onOpenChange(false);
      toast.success(expense ? t("split.saved") : t("split.added"));
    },
    onError,
  });
  const remove = useMutation({
    mutationFn: () => planClient().removeExpense({ expenseId: expense?.id ?? "" }),
    onSuccess: () => {
      refresh();
      onOpenChange(false);
      toast(t("split.removed"));
    },
    onError,
  });

  const toggle = (user: common.UserSummary) => set({ included: draft.included.includes(user.id) ? draft.included.filter((id) => id !== user.id) : [...draft.included, user.id] });
  const totalWeight = weights.reduce((sum, weight) => sum + (Number.isFinite(weight) ? weight : 0), 0);
  const preview = (index: number): string => {
    if (amount === null || included.length === 0) return "";
    if (draft.method === "equal") return formatMoney(amount / BigInt(included.length), currency, i18n.language);
    if (draft.method === "shares" && totalWeight > 0 && Number.isInteger(weights[index]) && weights[index]! > 0) return formatMoney((amount * BigInt(weights[index]!)) / BigInt(totalWeight), currency, i18n.language);
    return "";
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={expense ? t("split.form.editTitle") : t("split.form.title")}
      footer={
        <>
          {expense && (
            <Button variant="ghost" className="mr-auto text-danger" icon={<Trash2 className="size-4" />} loading={remove.isPending} onClick={() => remove.mutate()}>
              {t("split.form.delete")}
            </Button>
          )}
          <Button loading={save.isPending} disabled={Boolean(problem)} onClick={() => save.mutate()}>
            {expense ? t("split.form.save") : t("split.form.add")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <TextField label={t("split.form.what")} placeholder={t("split.form.whatPlaceholder")} value={draft.title} maxLength={80} onChange={(event) => set({ title: event.target.value })} />
        <TextField label={t("split.form.amount", { currency })} inputMode="decimal" placeholder="0" value={draft.amount} onChange={(event) => set({ amount: event.target.value })} />
        <SelectField label={t("split.form.payer")} value={draft.payerId} onChange={(payerId) => set({ payerId })} options={people.map((user) => ({ value: user.id, label: user.id === myId ? t("split.form.me", { name: nameOf(user) }) : nameOf(user) }))} />

        <div className="flex flex-col gap-2">
          <p className="px-1 text-sm font-bold text-ink-soft">{t("split.form.how")}</p>
          <Tabs<Method>
            value={draft.method}
            onChange={(method) => set({ method })}
            items={[
              { value: "equal", label: t("split.form.equal") },
              { value: "exact", label: t("split.form.exact") },
              { value: "shares", label: t("split.form.shares") },
            ]}
          />
          <ul className="flex flex-col gap-1">
            {people.map((user) => {
              const on = draft.included.includes(user.id);
              const index = included.findIndex((other) => other.id === user.id);
              return (
                <li key={user.id} className={cn("flex items-center gap-2 rounded-2xl px-2 py-1.5", on && "bg-surface-2")}>
                  {/* biome-ignore lint/a11y/useSemanticElements: the whole row is the tick box. */}
                  <button type="button" role="checkbox" aria-checked={on} aria-label={nameOf(user)} onClick={() => toggle(user)} className="flex min-w-0 flex-1 items-center gap-2.5 text-left">
                    <span className={cn("grid size-7 shrink-0 place-items-center rounded-lg border-2", on ? "border-ink bg-lime text-[#17122b]" : "border-line")}>{on && <Check className="size-4" strokeWidth={3} />}</span>
                    <Avatar user={user} size="xs" />
                    <span className="truncate font-bold">{nameOf(user)}</span>
                  </button>
                  {on && draft.method === "exact" && (
                    <input
                      aria-label={t("split.form.exactFor", { name: nameOf(user) })}
                      inputMode="decimal"
                      placeholder="0"
                      value={draft.exact[user.id] ?? ""}
                      onChange={(event) => set({ exact: { ...draft.exact, [user.id]: event.target.value } })}
                      className="h-10 w-24 rounded-xl border-2 border-line bg-surface px-3 text-right font-bold focus:border-brand focus:outline-none"
                    />
                  )}
                  {on && draft.method === "shares" && (
                    <input
                      aria-label={t("split.form.weightFor", { name: nameOf(user) })}
                      inputMode="numeric"
                      value={draft.weights[user.id] ?? "1"}
                      onChange={(event) => set({ weights: { ...draft.weights, [user.id]: event.target.value } })}
                      className="h-10 w-14 rounded-xl border-2 border-line bg-surface px-2 text-center font-bold focus:border-brand focus:outline-none"
                    />
                  )}
                  {on && draft.method !== "exact" && <span className="w-20 shrink-0 text-right text-sm font-bold text-ink-soft">{preview(index)}</span>}
                </li>
              );
            })}
          </ul>
          {draft.method === "equal" && amount !== null && included.length > 0 && amount % BigInt(included.length) !== 0n && <p className="px-1 text-xs text-ink-faint">{t("split.form.remainder")}</p>}
          <p className="px-1 text-xs text-ink-faint">{t("split.form.goingOnly")}</p>
        </div>
        {problem && (draft.title || draft.amount) && (
          <p role="alert" className="px-1 text-sm font-bold text-danger">
            {problem}
          </p>
        )}
      </div>
    </Dialog>
  );
}
