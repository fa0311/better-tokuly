import type { QueryClient } from "@tanstack/query-core";
import { create, wallet } from "@tokuly/api";
import { type ClientSource, nextCursor } from "../query";
import { walletKeys } from "./keys";

type Wallet = ClientSource<typeof wallet.WalletService>;

/** Query options of the wallet. Each app adds its hooks around them. */
export const walletQueries = {
  /**
   * The balance as the server has it. The answer still loses against a newer
   * snapshot pushed while it was on its way (`rememberBalance`).
   */
  balance: (client: Wallet, queryClient: QueryClient) => ({
    queryKey: walletKeys.balance,
    queryFn: async (): Promise<wallet.Balance> => {
      const fetched = (await client().getBalance({})).balance ?? create(wallet.BalanceSchema);
      const known = queryClient.getQueryData<wallet.Balance>(walletKeys.balance);
      return known && known.revision > fetched.revision ? known : fetched;
    },
  }),

  /** The history of one balance (`UNSPECIFIED` = both), newest first, page by page. */
  ledger: (client: Wallet, account: wallet.LedgerAccount, pageSize: number) => ({
    queryKey: walletKeys.ledger(account),
    queryFn: ({ pageParam }: { pageParam: string }) =>
      client().listLedgerEntries({ account, page: { limit: pageSize, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  }),

  /** The daily login bonus; `null` where the server has none. */
  dailyBonus: (client: Wallet) => ({
    queryKey: walletKeys.dailyBonus,
    queryFn: async () => (await client().getDailyBonus({})).bonus ?? null,
  }),

  /** The caller's purchases, newest first, page by page. */
  purchases: (client: Wallet, pageSize: number) => ({
    queryKey: walletKeys.purchases,
    queryFn: ({ pageParam }: { pageParam: string }) =>
      client().listPurchases({ page: { limit: pageSize, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  }),
};
