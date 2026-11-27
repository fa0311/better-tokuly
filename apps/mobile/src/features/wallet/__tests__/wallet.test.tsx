import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { create, wallet } from "@tokuly/api";
import { walletKeys } from "@tokuly/core/wallet";
import type { ReactNode } from "react";
import { Alert } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { currentAccount, isActive } from "#/features/session/account";
import { api } from "#/lib/api";
import { HistoryScreen } from "../history-screen";
import links from "../links";
import { WalletScreen } from "../wallet-screen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("#/features/session/account", () => ({ currentAccount: jest.fn(), isActive: jest.fn() }));
jest.mock("expo-router", () => ({ router: { push: jest.fn(), back: jest.fn() } }));
jest.mock("#/lib/realtime", () => ({ realtime: () => ({ onFrame: () => () => {} }) }));

const CYCLE = [10n, 10n, 15n, 15n, 20n, 20n, 50n];
const at = (date: Date) => ({ seconds: BigInt(Math.floor(date.getTime() / 1000)), nanos: 0 });

/**
 * The wallet as the server answers it: 100 coins, today's bonus waiting,
 * the claims it was sent (answered when `answer()` is called) and a ledger
 * of two movements.
 */
function server() {
  let coins = 100n;
  let revision = 1n;
  let claimed = false;
  let switched = false;
  /** How long until the next bonus unlocks, by the phone's clock. */
  let unlocksIn = 3 * 3_600_000;
  const claims: number[] = [];
  let answer = () => {};
  let lose = () => {};
  const bonus = () =>
    create(wallet.DailyBonusSchema, {
      claimable: !claimed,
      streak: claimed ? 3 : 2,
      nextCoins: claimed ? 15n : 15n,
      nextCycleDay: claimed ? 4 : 3,
      cycle: CYCLE,
      nextClaimAt: claimed ? at(new Date(Date.now() + unlocksIn)) : undefined,
    });
  const balance = () => create(wallet.BalanceSchema, { coins, earnings: 0n, revision });
  const grant = () => {
    claimed = true;
    coins += 15n;
    revision += 1n;
  };
  const client = {
    service: () => ({
      getBalance: async () => ({ balance: balance() }),
      getDailyBonus: async () => ({ bonus: bonus() }),
      listLedgerEntries: async ({ account }: { account: wallet.LedgerAccount }) =>
        create(wallet.ListLedgerEntriesResponseSchema, {
          entries: [
            create(wallet.LedgerEntrySchema, {
              id: "e2",
              kind: wallet.EntryKind.TRANSFER,
              reason: "live.gift",
              description: "ライブギフトを送りました",
              coinsDelta: -30n,
              amount: 30n,
              createdAt: at(new Date()),
            }),
            ...(account === wallet.LedgerAccount.EARNINGS
              ? []
              : [
                  create(wallet.LedgerEntrySchema, {
                    id: "e1",
                    kind: wallet.EntryKind.GRANT,
                    reason: "daily_bonus",
                    description: "ログインボーナス",
                    coinsDelta: 10n,
                    amount: 10n,
                    createdAt: at(new Date(Date.now() - 86_400_000)),
                  }),
                ]),
          ],
        }),
      claimDailyBonus: () => {
        claims.push(claims.length);
        return new Promise((resolve, reject) => {
          answer = () => {
            grant();
            resolve(
              create(wallet.ClaimDailyBonusResponseSchema, {
                coins: 15n,
                bonus: bonus(),
                balance: balance(),
              }),
            );
          };
          lose = () => {
            grant();
            reject(new Error("the connection dropped before the answer arrived"));
          };
        });
      },
    }),
  };
  jest.mocked(api).mockReturnValue(client as never);
  jest
    .mocked(currentAccount)
    .mockReturnValue({ sessionId: "s1", userId: "u1", api: client as never });
  jest.mocked(isActive).mockImplementation(() => !switched);
  return {
    claims,
    answer: () => answer(),
    /** The server grants the claim, but its answer never reaches the phone. */
    loseAnswer: () => lose(),
    switchAccount: () => {
      switched = true;
    },
    /** The bonus is claimed on another phone. */
    claimElsewhere: grant,
    /** Midnight passes on the server: the next bonus waits. */
    nextDay: () => {
      claimed = false;
    },
    /** The phone's clock runs ahead of the server's: it is past midnight here, not yet there. */
    clockAhead: () => {
      unlocksIn = -1000;
    },
  };
}

async function show(children: ReactNode, queryClient = new QueryClient()) {
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    </SafeAreaProvider>,
  );
}

const coinsShown = () => screen.getByTestId("wallet-coins").props.children;

describe("the wallet screen", () => {
  beforeEach(() => jest.spyOn(Alert, "alert").mockImplementation(() => {}));
  afterEach(() => jest.clearAllMocks());

  it("shows the server's balance, today's bonus and the latest movements", async () => {
    server();
    await show(<WalletScreen />);
    await waitFor(() => expect(coinsShown()).toBe("🪙 100"));
    expect(await screen.findByText("受け取る！ 🪙 +15")).toBeTruthy();
    expect(await screen.findByText("ライブギフトを送りました")).toBeTruthy();
    expect(screen.getByText("🪙 −30")).toBeTruthy();
  });

  it("claims the bonus once however often it is tapped, and shows what the server granted", async () => {
    const api = server();
    await show(<WalletScreen />);
    const claim = await screen.findByTestId("wallet-bonus-claim");
    await fireEvent.press(claim);
    await fireEvent.press(claim);
    await fireEvent.press(claim);
    expect(api.claims).toHaveLength(1);
    expect(screen.queryByText(/ゲット/)).toBeNull();
    api.answer();
    await waitFor(() => expect(screen.getByText("ゲット！ 🪙 +15")).toBeTruthy());
    await waitFor(() => expect(coinsShown()).toBe("🪙 115"));
    expect(api.claims).toHaveLength(1);
  });

  it("sends one claim when the wallet is open twice and both are tapped", async () => {
    const api = server();
    await show(
      <>
        <WalletScreen />
        <WalletScreen />
      </>,
    );
    await waitFor(() => expect(screen.getAllByTestId("wallet-bonus-claim")).toHaveLength(2));
    for (const claim of screen.getAllByTestId("wallet-bonus-claim")) await fireEvent.press(claim);
    expect(api.claims).toHaveLength(1);
    api.answer();
    await waitFor(() => expect(screen.getByText("ゲット！ 🪙 +15")).toBeTruthy());
    expect(screen.getByText("今日のぶんは受け取ったよ")).toBeTruthy();
    expect(Alert.alert).not.toHaveBeenCalled();
  });

  it("shows a claim as won only until the next bonus, not for one made elsewhere later", async () => {
    const api = server();
    const queryClient = new QueryClient();
    const readBonus = () => queryClient.invalidateQueries({ queryKey: walletKeys.dailyBonus });
    await show(<WalletScreen />, queryClient);
    await fireEvent.press(await screen.findByTestId("wallet-bonus-claim"));
    api.answer();
    await waitFor(() => expect(screen.getByText("ゲット！ 🪙 +15")).toBeTruthy());
    api.nextDay();
    await act(readBonus);
    expect(await screen.findByTestId("wallet-bonus-claim")).toBeTruthy();
    api.claimElsewhere();
    await act(readBonus);
    await waitFor(() => expect(screen.getByText("今日のぶんは受け取ったよ")).toBeTruthy());
    expect(screen.queryByText(/ゲット/)).toBeNull();
  });

  it("keeps asking for the next bonus while the phone's clock is ahead of the server's", async () => {
    jest.useFakeTimers();
    try {
      const api = server();
      api.claimElsewhere();
      api.clockAhead();
      await show(<WalletScreen />);
      expect(await screen.findByTestId("wallet-bonus-claimed")).toBeTruthy();
      api.nextDay();
      await act(() => jest.advanceTimersByTimeAsync(21_000));
      expect(await screen.findByTestId("wallet-bonus-claim")).toBeTruthy();
    } finally {
      jest.useRealTimers();
    }
  });

  it("never shows a claim whose answer was lost as won, and reads the server again", async () => {
    const api = server();
    await show(<WalletScreen />);
    await waitFor(() => expect(coinsShown()).toBe("🪙 100"));
    await fireEvent.press(await screen.findByTestId("wallet-bonus-claim"));
    api.loseAnswer();
    await waitFor(() => expect(screen.getByText("今日のぶんは受け取ったよ")).toBeTruthy());
    await waitFor(() => expect(coinsShown()).toBe("🪙 115"));
    expect(screen.queryByText(/ゲット/)).toBeNull();
    expect(Alert.alert).toHaveBeenCalledTimes(1);
  });

  it("drops a claim answered after an account switch", async () => {
    const api = server();
    await show(<WalletScreen />);
    await waitFor(() => expect(coinsShown()).toBe("🪙 100"));
    await fireEvent.press(await screen.findByTestId("wallet-bonus-claim"));
    api.switchAccount();
    api.answer();
    await waitFor(() => expect(api.claims).toHaveLength(1));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.queryByText(/ゲット/)).toBeNull();
    expect(coinsShown()).toBe("🪙 100");
    expect(Alert.alert).not.toHaveBeenCalled();
  });
});

describe("the history screen", () => {
  afterEach(() => jest.clearAllMocks());

  it("lists the movements by day and opens one", async () => {
    server();
    await show(<HistoryScreen />);
    expect(await screen.findByText("今日")).toBeTruthy();
    expect(screen.getByText("昨日")).toBeTruthy();
    await fireEvent.press(screen.getByTestId("wallet-entry-e2"));
    expect(await screen.findByTestId("wallet-entry-sheet")).toBeTruthy();
    expect(screen.getByText("e2")).toBeTruthy();
  });

  it("filters to the earnings", async () => {
    server();
    await show(<HistoryScreen />);
    await screen.findByText("昨日");
    await fireEvent.press(screen.getByTestId("wallet-history-filter-earnings"));
    await waitFor(() => expect(screen.queryByText("昨日")).toBeNull());
  });
});

describe("wallet links", () => {
  const screenFor = (path: string) => {
    for (const target of links) {
      const match = path.match(target.path);
      if (match) return target.screen(match, path);
    }
    return null;
  };

  it("open the front page, the history and the purchases here, the rest on the web", () => {
    expect(screenFor("/wallet")).toBe("/wallet");
    expect(screenFor("/wallet/history")).toBe("/wallet/history");
    expect(screenFor("/wallet/purchases")).toBe("/wallet/purchases");
    expect(screenFor("/wallet/purchases/p1")).toContain("/web?path=");
    expect(screenFor("/wallet/buy")).toContain("/web?path=");
  });
});
