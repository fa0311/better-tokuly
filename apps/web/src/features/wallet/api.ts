import type { InfiniteData, QueryClient } from "@tanstack/react-query";
import { create, wallet } from "@tokuly/api";
import { api } from "#/lib/api";

export const walletClient = () => api().service(wallet.WalletService);
export const earningsClient = () => api().service(wallet.EarningsService);
export const walletAdminClient = () => api().service(wallet.WalletAdminService);

/** Rows per page of the wallet's lists. */
export const PAGE_SIZE = 30;

/**
 * Query keys of the wallet. Everything the signed-in user sees about their
 * own coins lives under `["wallet", …]`; the staff tools under
 * `["wallet", "admin", …]`.
 */
export const walletKeys = {
  all: ["wallet"] as const,
  balance: ["wallet", "balance"] as const,
  ledgers: ["wallet", "ledger"] as const,
  ledger: (account: wallet.LedgerAccount) => ["wallet", "ledger", account] as const,
  packages: ["wallet", "packages"] as const,
  purchases: ["wallet", "purchases"] as const,
  purchase: (purchaseId: string) => ["wallet", "purchase", purchaseId] as const,
  dailyBonus: ["wallet", "dailyBonus"] as const,
  earnings: ["wallet", "earnings"] as const,
  earningsSummary: (range: string) => ["wallet", "earnings", range] as const,
  payouts: ["wallet", "payouts"] as const,
  payout: (payoutId: string) => ["wallet", "payouts", "one", payoutId] as const,
  admin: {
    all: ["wallet", "admin"] as const,
    queues: ["wallet", "admin", "queue"] as const,
    queue: (status: wallet.PayoutStatus) => ["wallet", "admin", "queue", status] as const,
    pendingCount: ["wallet", "admin", "queue", "pendingCount"] as const,
    payout: (payoutId: string) => ["wallet", "admin", "payout", payoutId] as const,
    userSearch: (query: string) => ["wallet", "admin", "userSearch", query] as const,
    userWallet: (userId: string) => ["wallet", "admin", "user", userId] as const,
    userLedger: (userId: string, account: wallet.LedgerAccount) =>
      ["wallet", "admin", "userLedger", userId, account] as const,
    adjustments: (userId: string) => ["wallet", "admin", "adjustments", userId] as const,
    allAdjustments: ["wallet", "admin", "adjustments"] as const,
    packages: ["wallet", "admin", "packages"] as const,
  },
};

type Paged<T> = InfiniteData<T, string>;

/**
 * Stores a balance snapshot unless a newer one is already known. Snapshots
 * arrive from RPC responses and realtime pushes in any order; the one with
 * the highest revision is the truth.
 */
export function rememberBalance(
  queryClient: QueryClient,
  balance: wallet.Balance | undefined,
): void {
  if (!balance) return;
  queryClient.setQueryData<wallet.Balance>(walletKeys.balance, (known) =>
    known && known.revision >= balance.revision ? known : balance,
  );
}

/** Asks the server for the balance; the answer still loses against a newer snapshot pushed meanwhile. */
export async function fetchBalance(queryClient: QueryClient): Promise<wallet.Balance> {
  const fetched = (await walletClient().getBalance({})).balance ?? create(wallet.BalanceSchema);
  const known = queryClient.getQueryData<wallet.Balance>(walletKeys.balance);
  return known && known.revision > fetched.revision ? known : fetched;
}

/** Puts a new movement at the top of every loaded history list it belongs to. */
export function rememberEntry(queryClient: QueryClient, entry: wallet.LedgerEntry): void {
  for (const account of [
    wallet.LedgerAccount.UNSPECIFIED,
    wallet.LedgerAccount.COINS,
    wallet.LedgerAccount.EARNINGS,
  ]) {
    const touches =
      account === wallet.LedgerAccount.UNSPECIFIED ||
      (account === wallet.LedgerAccount.COINS
        ? entry.coinsDelta !== 0n
        : entry.earningsDelta !== 0n);
    if (!touches) continue;
    queryClient.setQueryData<Paged<wallet.ListLedgerEntriesResponse>>(
      walletKeys.ledger(account),
      (data) => {
        const first = data?.pages[0];
        if (
          !data ||
          !first ||
          data.pages.some((page) => page.entries.some((known) => known.id === entry.id))
        )
          return data;
        return {
          ...data,
          pages: [{ ...first, entries: [entry, ...first.entries] }, ...data.pages.slice(1)],
        };
      },
    );
  }
}

/** Stores the latest state of a purchase wherever it is cached. */
export function rememberPurchase(
  queryClient: QueryClient,
  purchase: wallet.Purchase | undefined,
): void {
  if (!purchase) return;
  queryClient.setQueryData(walletKeys.purchase(purchase.id), purchase);
  let listed = false;
  queryClient.setQueryData<Paged<wallet.ListPurchasesResponse>>(walletKeys.purchases, (data) => {
    if (!data) return data;
    const pages = data.pages.map((page) => ({
      ...page,
      purchases: page.purchases.map((known) => {
        if (known.id !== purchase.id) return known;
        listed = true;
        return purchase;
      }),
    }));
    return { ...data, pages };
  });
  // A purchase the list has not seen yet: let it load again from the top.
  if (!listed) void queryClient.invalidateQueries({ queryKey: walletKeys.purchases });
}

/** Stores the latest state of one of the user's payout requests and refreshes what depends on it. */
export function rememberPayout(
  queryClient: QueryClient,
  payout: wallet.PayoutRequest | undefined,
): void {
  if (!payout) return;
  queryClient.setQueryData(walletKeys.payout(payout.id), payout);
  let listed = false;
  queryClient.setQueryData<Paged<wallet.ListPayoutRequestsResponse>>(walletKeys.payouts, (data) => {
    if (!data) return data;
    const pages = data.pages.map((page) => ({
      ...page,
      payouts: page.payouts.map((known) => {
        if (known.id !== payout.id) return known;
        listed = true;
        return payout;
      }),
    }));
    return { ...data, pages };
  });
  if (!listed) void queryClient.invalidateQueries({ queryKey: walletKeys.payouts, exact: true });
  // Available / locked earnings move with every status change.
  void queryClient.invalidateQueries({ queryKey: walletKeys.earnings });
}

/** Applies one realtime wallet event to the query cache. */
export function applyWalletEvent(queryClient: QueryClient, event: wallet.WalletEvent): void {
  switch (event.event.case) {
    case "balanceChanged": {
      const { balance, entry } = event.event.value;
      rememberBalance(queryClient, balance);
      if (!entry) return;
      rememberEntry(queryClient, entry);
      if (entry.earningsDelta !== 0n)
        void queryClient.invalidateQueries({ queryKey: walletKeys.earnings });
      // Claimed on another device: the bonus card must stop offering it here.
      if (
        entry.reason === "daily_bonus" &&
        queryClient.getQueryData<wallet.DailyBonus>(walletKeys.dailyBonus)?.claimable
      ) {
        void queryClient.invalidateQueries({ queryKey: walletKeys.dailyBonus });
      }
      return;
    }
    case "purchaseUpdated":
      rememberPurchase(queryClient, event.event.value);
      return;
    case "payoutUpdated":
      rememberPayout(queryClient, event.event.value);
      return;
    default:
      return;
  }
}
