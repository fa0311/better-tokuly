import { QueryClient } from "@tanstack/query-core";
import { create, wallet } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { applyWalletEvent, rememberBalance } from "./cache";
import { walletKeys } from "./keys";
import { walletQueries } from "./queries";

const balance = (coins: bigint, revision: bigint) =>
  create(wallet.BalanceSchema, { coins, earnings: 0n, revision });

const ledgerPage = (...ids: string[]) =>
  create(wallet.ListLedgerEntriesResponseSchema, {
    entries: ids.map((id) => create(wallet.LedgerEntrySchema, { id, coinsDelta: -1n })),
  });

describe("rememberBalance", () => {
  it("keeps the snapshot with the highest revision, whatever order they arrive in", () => {
    const queryClient = new QueryClient();
    rememberBalance(queryClient, balance(50n, 7n));
    rememberBalance(queryClient, balance(80n, 5n));
    expect(queryClient.getQueryData<wallet.Balance>(walletKeys.balance)?.coins).toBe(50n);
    rememberBalance(queryClient, balance(20n, 8n));
    expect(queryClient.getQueryData<wallet.Balance>(walletKeys.balance)?.coins).toBe(20n);
  });
});

describe("walletQueries.balance", () => {
  it("lets a snapshot pushed while the request was on its way win over its older answer", async () => {
    const queryClient = new QueryClient();
    const client = () =>
      ({
        getBalance: async () => {
          rememberBalance(queryClient, balance(30n, 9n));
          return create(wallet.GetBalanceResponseSchema, { balance: balance(100n, 4n) });
        },
      }) as never;
    const answer = await walletQueries.balance(client, queryClient).queryFn();
    expect(answer.coins).toBe(30n);
  });
});

describe("applyWalletEvent", () => {
  it("puts a pushed movement on top of the loaded history once, with its balance", () => {
    const queryClient = new QueryClient();
    const key = walletKeys.ledger(wallet.LedgerAccount.UNSPECIFIED);
    queryClient.setQueryData(key, { pages: [ledgerPage("old")], pageParams: [""] });
    const event = create(wallet.WalletEventSchema, {
      event: {
        case: "balanceChanged",
        value: {
          balance: balance(9n, 3n),
          entry: create(wallet.LedgerEntrySchema, { id: "new", coinsDelta: -1n }),
        },
      },
    });

    applyWalletEvent(queryClient, event);
    applyWalletEvent(queryClient, event);

    const pages = queryClient.getQueryData<{ pages: wallet.ListLedgerEntriesResponse[] }>(key);
    expect(pages?.pages[0]?.entries.map((entry) => entry.id)).toEqual(["new", "old"]);
    expect(queryClient.getQueryData<wallet.Balance>(walletKeys.balance)?.coins).toBe(9n);
  });
});
