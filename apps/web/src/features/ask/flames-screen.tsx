import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { type ask, toMillis } from "@tokuly/api";
import { Lightbulb, MoreHorizontal, ShieldBan } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Card } from "#/components/ui/card";
import { Coins } from "#/components/ui/coins";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { LoadMore } from "#/components/ui/load-more";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { BalancePill, useBalance } from "#/features/wallet";
import { cn } from "#/lib/cn";
import { compactAge } from "#/lib/format";
import { useMe } from "#/stores/session";
import { askKeys, boxClient, complimentClient, rememberFlame, useFlameSummary, useFlames, useTopFlames } from "./api";
import { localized } from "./model";
import { SectionTitle, toastError } from "./parts";

/** The flames inbox: compliments the viewer received (never from whom), hints for coins, and this week's ranking among friends. */
export function FlamesScreen() {
  const { t, i18n } = useTranslation("ask");
  const queryClient = useQueryClient();
  const me = useMe();
  const flames = useFlames();
  const summary = useFlameSummary().data;
  const top = useTopFlames().data;
  const list = flames.data?.pages.flatMap((page) => page.flames) ?? [];

  // Opening the inbox is seeing the flames: the badge clears, the NEW marks stay for this visit.
  const unseen = summary?.unseen ?? 0;
  useEffect(() => {
    if (unseen <= 0) return;
    complimentClient()
      .markFlamesSeen({ all: true })
      .then((response) => queryClient.setQueryData<ask.FlameSummary>(askKeys.flameSummary, (known) => (known ? { ...known, unseen: response.unseen } : known)))
      .catch(() => {});
  }, [unseen, queryClient]);

  return (
    <Page title={t("flames.title")} back="/ask" wide actions={<BalancePill />}>
      <div className="grid grid-cols-1 items-start gap-x-8 lg:grid-cols-[minmax(0,1fr)_minmax(0,24rem)]">
        <section className="lg:order-2">
          <Card className="bg-sun text-[#17122b]" data-testid="flame-summary">
            <div className="grid grid-cols-2 text-center">
              <div>
                <p className="text-xs font-black opacity-70">{t("flames.total")}</p>
                <p className="text-4xl font-black tabular-nums" data-testid="flame-total">
                  🔥{summary?.total ?? 0}
                </p>
              </div>
              <div>
                <p className="text-xs font-black opacity-70">{t("flames.thisWeek")}</p>
                <p className="text-4xl font-black tabular-nums">{summary?.thisWeek ?? 0}</p>
              </div>
            </div>
            {summary && summary.topQuestions.length > 0 && (
              <div className="mt-4 border-t-2 border-ink/20 pt-3">
                <p className="text-xs font-black opacity-70">{t("flames.top")}</p>
                <ul className="mt-2 flex flex-col gap-1.5">
                  {summary.topQuestions.slice(0, 3).map((entry) => (
                    <li key={entry.question?.id} className="flex items-center gap-2 text-sm font-black">
                      <span aria-hidden>{entry.question?.emoji}</span>
                      <span className="min-w-0 flex-1 truncate">{localized(entry.question?.text, i18n.language)}</span>
                      <span className="tabular-nums opacity-70">{t("flames.times", { count: entry.count })}</span>
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </Card>

          <SectionTitle>{t("flames.ranking")}</SectionTitle>
          <p className="-mt-2 mb-3 px-1 text-xs text-ink-soft">{t("flames.rankingLead")}</p>
          {!top ? (
            <Skeleton className="h-24" />
          ) : top.entries.length === 0 ? (
            <p className="rounded-card bg-surface px-4 py-6 text-center text-sm font-bold text-ink-soft">🏆 {t("flames.rankingEmpty")}</p>
          ) : (
            <ol className="flex flex-col gap-1 rounded-card bg-surface p-2" data-testid="flame-ranking">
              {top.entries.map((entry) => {
                const isMe = entry.user?.id === me?.user?.id;
                return (
                  <li key={entry.user?.id} className={cn("flex items-center gap-3 rounded-2xl px-3 py-2", isMe && "bg-brand-soft")}>
                    <span className="w-6 text-center font-black tabular-nums">{entry.rank <= 3 ? ["🥇", "🥈", "🥉"][entry.rank - 1] : entry.rank}</span>
                    <Avatar user={entry.user} size="sm" />
                    <span className="min-w-0 flex-1 truncate font-bold">{isMe ? t("flames.you") : entry.user?.displayName}</span>
                    <span className="font-black tabular-nums">🔥{entry.count}</span>
                  </li>
                );
              })}
            </ol>
          )}
        </section>

        <section className="lg:order-1">
          <SectionTitle className="lg:mt-0">{t("flames.list")}</SectionTitle>
          {flames.isPending ? (
            <div className="flex flex-col gap-3">
              <Skeleton className="h-24" />
              <Skeleton className="h-24" />
            </div>
          ) : list.length === 0 ? (
            <EmptyState
              emoji="🕯️"
              title={t("flames.empty")}
              body={t("flames.emptyBody")}
              action={
                <Link to="/ask/polls" className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand">
                  {t("flames.play")}
                </Link>
              }
            />
          ) : (
            <div className="flex flex-col gap-3" data-testid="flame-list">
              {list.map((flame) => (
                <FlameRow key={flame.id} flame={flame} />
              ))}
              <LoadMore query={flames} />
            </div>
          )}
        </section>
      </div>
    </Page>
  );
}

function FlameRow({ flame }: { flame: ask.Flame }) {
  const { t, i18n } = useTranslation("ask");
  const queryClient = useQueryClient();
  const [dialog, setDialog] = useState<"buy" | "block" | null>(null);
  const block = useMutation({
    mutationFn: () => boxClient().blockSender({ target: { case: "flameId", value: flame.id } }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: askKeys.blocks });
      toast.success(t("inbox.toast.blocked"));
      setDialog(null);
    },
    onError: toastError,
  });

  return (
    <article className="sticker rounded-card bg-surface px-4 pt-3.5 pb-3" data-testid="flame-row">
      <div className="flex items-start gap-3">
        <span aria-hidden className="grid size-12 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-coral text-2xl">
          {flame.question?.emoji || "🔥"}
        </span>
        <div className="min-w-0 flex-1">
          <p className="leading-snug font-black break-words">{localized(flame.question?.text, i18n.language)}</p>
          <p className="mt-1 flex flex-wrap items-center gap-1.5 text-xs font-bold text-ink-soft">
            {!flame.seen && <span className="rounded-full bg-pink px-2 py-0.5 text-[11px] font-black text-white">{t("flames.new")}</span>}
            <span>{t("flames.from")}</span>
            <time className="text-ink-faint">{compactAge(toMillis(flame.createdAt))}</time>
          </p>
        </div>
        <Menu
          trigger={
            <Button size="icon-sm" variant="ghost" aria-label={t("inbox.actions.more")}>
              <MoreHorizontal className="size-5" />
            </Button>
          }
          actions={[{ label: t("flames.block"), icon: <ShieldBan />, onSelect: () => setDialog("block") }]}
        />
      </div>

      {flame.hints.length > 0 && (
        <ul className="mt-3 flex flex-col gap-1.5" data-testid="flame-hints">
          {flame.hints.map((hint) => (
            <li key={hint.kind} className="flex items-center gap-2 rounded-2xl bg-brand-soft px-3 py-2 text-sm font-bold">
              <Lightbulb className="size-4 shrink-0 text-brand" />
              {localized(hint.text, i18n.language)}
            </li>
          ))}
        </ul>
      )}
      <div className="mt-3 flex items-center gap-2">
        {flame.nextHintPrice > 0n ? (
          <Button size="sm" variant="secondary" icon={<Lightbulb className="size-4" />} onClick={() => setDialog("buy")}>
            {t("flames.buyHint")}
            <Coins amount={flame.nextHintPrice} size="sm" />
          </Button>
        ) : (
          <span className="text-xs font-bold text-ink-faint">{flame.hints.length > 0 ? t("flames.allHints") : t("flames.noHints")}</span>
        )}
      </div>

      {dialog === "buy" && <HintPurchase flame={flame} onClose={() => setDialog(null)} />}
      <ConfirmDialog
        open={dialog === "block"}
        onOpenChange={() => setDialog(null)}
        title={t("inbox.confirmBlock.title")}
        description={t("inbox.confirmBlock.body")}
        confirmLabel={t("inbox.confirmBlock.confirm")}
        danger
        loading={block.isPending}
        onConfirm={() => block.mutate()}
      />
    </article>
  );
}

/** Confirms a hint purchase: the price, the balance before and after; people who are short are sent to the wallet. */
function HintPurchase({ flame, onClose }: { flame: ask.Flame; onClose(): void }) {
  const { t } = useTranslation("ask");
  const queryClient = useQueryClient();
  const balance = useBalance();
  // One key per opened confirmation: a retried tap is charged once.
  const [key] = useState(() => crypto.randomUUID());
  const price = flame.nextHintPrice;
  const short = balance.coins !== undefined && balance.coins < price;

  const buy = useMutation({
    mutationFn: () => complimentClient().buyHint({ flameId: flame.id, idempotencyKey: key }),
    onSuccess: (response) => {
      if (response.flame) rememberFlame(queryClient, response.flame);
      void balance.refresh();
      toast.success(t("flames.purchase.bought"));
      onClose();
    },
    onError: (error) => {
      toastError(error);
      void balance.refresh();
    },
  });

  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("flames.purchase.title")}
      description={t("flames.purchase.body")}
      footer={
        <>
          <Button variant="ghost" onClick={onClose} disabled={buy.isPending}>
            {t("actions.cancel", { ns: "common" })}
          </Button>
          {short ? (
            <Link to="/wallet" className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand">
              {t("flames.purchase.toWallet")}
            </Link>
          ) : (
            <Button loading={buy.isPending} disabled={balance.coins === undefined} onClick={() => buy.mutate()}>
              {t("flames.purchase.confirm")}
            </Button>
          )}
        </>
      }
    >
      <dl className="flex flex-col gap-2 rounded-2xl bg-surface-2 p-4 text-sm font-bold">
        <div className="flex items-center justify-between" data-testid="hint-price">
          <dt>💡 {t("flames.purchase.price")}</dt>
          <dd>
            <Coins amount={price} />
          </dd>
        </div>
        <div className="flex items-center justify-between" data-testid="hint-balance">
          <dt>{t("flames.purchase.balance")}</dt>
          <dd>{balance.coins === undefined ? <Skeleton className="h-5 w-14 rounded-full" /> : <Coins amount={balance.coins} />}</dd>
        </div>
        {balance.coins !== undefined && !short && (
          <div className="flex items-center justify-between border-t-2 border-line pt-2" data-testid="hint-after">
            <dt>{t("flames.purchase.after")}</dt>
            <dd>
              <Coins amount={balance.coins - price} />
            </dd>
          </div>
        )}
      </dl>
      {short && balance.coins !== undefined && (
        <p className="mt-3 text-sm font-black text-danger" role="alert" data-testid="hint-short">
          {t("flames.purchase.short", { count: Number(price - balance.coins) })}
        </p>
      )}
    </Dialog>
  );
}
