import { useInfiniteQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { wallet } from "@tokuly/api";
import { uniqueById, walletQueries } from "@tokuly/core/wallet";
import {
  ChevronRight,
  History,
  type LucideIcon,
  PiggyBank,
  ReceiptText,
  ShoppingBag,
} from "lucide-react";
import { Card } from "#/components/ui/card";
import { CoinIcon, Coins } from "#/components/ui/coins";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { PAGE_SIZE, walletClient } from "./api";
import { useCountUp } from "./celebration";
import { DailyBonusCard } from "./daily-bonus-card";
import { LedgerList } from "./ledger";
import { LinkButton, SectionTitle } from "./parts";
import { SpendingLimitBanner } from "./spending-limit";
import { useBalance } from "./use-balance";

/** How many of the latest movements the home page shows. */
const RECENT = 6;

/** The wallet's front page: what you have, today's bonus, and the ways onward. */
export function WalletHomeScreen() {
  const { t } = useTranslation("wallet");
  return (
    <Page title={t("title")} back="/me" wide>
      <div className="grid items-start gap-5 lg:grid-cols-2">
        <div className="flex min-w-0 flex-col gap-5">
          <BalanceHero />
          <SpendingLimitBanner />
          <DailyBonusCard />
        </div>
        <div className="flex min-w-0 flex-col gap-5">
          <Shortcuts />
          <RecentActivity />
        </div>
      </div>
    </Page>
  );
}

/** The balance, front and centre; it rolls to its new value whenever coins arrive or leave. */
function BalanceHero() {
  const { t } = useTranslation("wallet");
  const { coins, earnings, isError, refresh } = useBalance();
  const shown = useCountUp(coins);

  return (
    <section
      className="sticker-lg relative overflow-hidden rounded-card bg-brand p-5 text-on-brand"
      aria-label={t("home.balance")}
    >
      {/* Coins scattered in the corner: decoration only. */}
      <CoinIcon className="absolute -top-5 -right-4 size-24 rotate-12 text-5xl opacity-90" />
      <CoinIcon className="absolute top-14 right-16 size-10 -rotate-12 text-xl opacity-80" />
      <p className="text-sm font-black opacity-80">{t("home.balance")}</p>
      <div className="mt-1 flex min-h-14 items-center">
        {shown === undefined ? (
          isError ? (
            <button
              type="button"
              onClick={() => void refresh()}
              className="font-bold underline decoration-2 underline-offset-2"
            >
              {t("home.balanceError")}
            </button>
          ) : (
            <Skeleton className="h-11 w-40 opacity-40" />
          )
        ) : (
          <Coins amount={shown} size="lg" className="text-5xl [&>span]:size-11 [&>span]:text-2xl" />
        )}
      </div>
      {earnings !== undefined && earnings > 0n && (
        <Link
          to="/wallet/earnings"
          className="mt-2 inline-flex items-center gap-1.5 rounded-full bg-on-brand/15 py-1 pr-2 pl-3 text-sm font-black"
        >
          {t("home.earnings")}
          <Coins amount={earnings} size="sm" />
          <ChevronRight className="size-4" />
        </Link>
      )}
      <div className="mt-5 flex flex-wrap gap-2.5">
        <LinkButton to="/wallet/buy" variant="lime" icon={<ShoppingBag className="size-4" />}>
          {t("home.buy")}
        </LinkButton>
        <LinkButton to="/wallet/history" variant="secondary" icon={<History className="size-4" />}>
          {t("home.history")}
        </LinkButton>
      </div>
    </section>
  );
}

interface Shortcut {
  to: "/wallet/earnings" | "/wallet/purchases";
  labelKey: string;
  hintKey: string;
  icon: LucideIcon;
  tint: string;
}

/** The places the hero's two buttons do not already lead to. */
const SHORTCUTS: Shortcut[] = [
  {
    to: "/wallet/earnings",
    labelKey: "home.shortcuts.earnings",
    hintKey: "home.shortcuts.earningsHint",
    icon: PiggyBank,
    tint: "bg-pink",
  },
  {
    to: "/wallet/purchases",
    labelKey: "home.shortcuts.purchases",
    hintKey: "home.shortcuts.purchasesHint",
    icon: ReceiptText,
    tint: "bg-sun",
  },
];

function Shortcuts() {
  const { t } = useTranslation("wallet");
  return (
    <nav aria-label={t("home.shortcuts.title")}>
      <ul className="grid grid-cols-2 gap-3">
        {SHORTCUTS.map((shortcut) => (
          <li key={shortcut.to}>
            <Link
              to={shortcut.to}
              className="pressable sticker flex h-full flex-col gap-3 rounded-card bg-surface p-4"
            >
              <span
                className={cn(
                  "grid size-11 shrink-0 place-items-center rounded-2xl border-2 border-ink text-[#17122b]",
                  shortcut.tint,
                )}
              >
                <shortcut.icon className="size-5" />
              </span>
              <span className="min-w-0">
                <span className="block font-black">{t(shortcut.labelKey)}</span>
                <span className="block text-xs leading-snug font-bold text-ink-soft">
                  {t(shortcut.hintKey)}
                </span>
              </span>
            </Link>
          </li>
        ))}
      </ul>
    </nav>
  );
}

/** The latest movements; the full list is one tap away. Shares its data with the history page. */
function RecentActivity() {
  const { t } = useTranslation("wallet");
  const query = useInfiniteQuery(
    walletQueries.ledger(walletClient, wallet.LedgerAccount.UNSPECIFIED, PAGE_SIZE),
  );
  const entries = uniqueById(query.data?.pages.flatMap((page) => page.entries) ?? []).slice(
    0,
    RECENT,
  );

  return (
    <section>
      <SectionTitle
        action={
          entries.length > 0 && (
            <Link to="/wallet/history" className="flex items-center text-sm font-black text-brand">
              {t("home.seeAll")}
              <ChevronRight className="size-4" />
            </Link>
          )
        }
      >
        {t("home.recent")}
      </SectionTitle>
      <Card className="mt-1 p-1.5">
        {query.isPending ? (
          <div className="flex flex-col gap-2 p-1.5">
            {[0, 1, 2].map((row) => (
              <Skeleton key={row} className="h-14" />
            ))}
          </div>
        ) : entries.length > 0 ? (
          <LedgerList entries={entries} />
        ) : (
          <EmptyState emoji="🪙" title={t("home.recentEmpty")} body={t("home.recentEmptyBody")} />
        )}
      </Card>
    </section>
  );
}
