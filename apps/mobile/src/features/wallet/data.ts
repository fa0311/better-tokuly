import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { wallet } from "@tokuly/api";
import { uniqueById, walletQueries } from "@tokuly/core/wallet";
import { type Account, currentAccount, isActive } from "#/features/session/account";
import { api } from "#/lib/api";

/**
 * The wallet in the query cache. Keys, query options and cache edits are
 * `@tokuly/core/wallet`; this file wires them to the phone's API client
 * (`api()` when called: whoever is signed in now). The cache belongs to one
 * account: a switch starts the signed-in tree over with an empty one.
 */
export const walletClient = () => api().service(wallet.WalletService);

/** Rows per page of the wallet's lists. */
export const PAGE_SIZE = 30;

/** The balance as the server has it, kept current by pushes (`background.tsx`). */
export function useBalance() {
  const queryClient = useQueryClient();
  return useQuery(walletQueries.balance(walletClient, queryClient));
}

/** The history of one balance (`UNSPECIFIED` = both), newest first, without duplicates. */
export function useLedger(account: wallet.LedgerAccount) {
  const query = useInfiniteQuery(walletQueries.ledger(walletClient, account, PAGE_SIZE));
  const entries = uniqueById(query.data?.pages.flatMap((page) => page.entries) ?? []);
  return { ...query, entries };
}

export const useDailyBonus = () => useQuery(walletQueries.dailyBonus(walletClient));

/** The monthly spending limit and this month's spending. */
export const useSpendingLimit = () => useQuery(walletQueries.spendingLimit(walletClient));

export function usePurchases() {
  const query = useInfiniteQuery(walletQueries.purchases(walletClient, PAGE_SIZE));
  const purchases = uniqueById(query.data?.pages.flatMap((page) => page.purchases) ?? []);
  return { ...query, purchases };
}

/**
 * Sends one request as the account signed in when it was asked, through
 * that account's own client, and hands back its answer — or its refusal —
 * only while that account still is (`undefined` otherwise: whoever took
 * over sees nothing of it; README, "Accounts").
 */
export async function asAccount<T>(send: (account: Account) => Promise<T>): Promise<T | undefined> {
  const account = currentAccount();
  if (!account) return undefined;
  try {
    const answer = await send(account);
    return isActive(account) ? answer : undefined;
  } catch (error) {
    if (isActive(account)) throw error;
    return undefined;
  }
}
