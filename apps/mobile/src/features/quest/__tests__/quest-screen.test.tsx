import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { create, quest } from "@tokuly/api";
import type { ReactNode } from "react";
import { Alert } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { currentAccount, isActive } from "#/features/session/account";
import { api } from "#/lib/api";
import { QuestScreen } from "../quest-screen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("#/features/session/account", () => ({ currentAccount: jest.fn(), isActive: jest.fn() }));
jest.mock("expo-router", () => ({ router: { push: jest.fn() }, Stack: { Screen: () => null } }));

const M = quest.MissionState;
const later = { seconds: BigInt(Math.floor(Date.now() / 1000) + 3600), nanos: 0 };

/**
 * The quest and wallet domains as the server answers them: one mission done
 * and waiting, the claims it was sent (answered when `answer()` is called),
 * and a balance that the claim raises on the server.
 */
function server() {
  let state = M.COMPLETED;
  let coins = 100n;
  let switched = false;
  const claims: string[] = [];
  let answer = () => {};
  let lose = () => {};
  const mission = () =>
    create(quest.MissionSchema, {
      id: "m1",
      key: "stamps_5",
      title: { ja: "スタンプを5こ送る", en: "Send 5 stamps" },
      progress: 5n,
      target: 5n,
      state,
      reward: { coins: 10, xp: 20 },
      endsAt: later,
    });
  const client = {
    service: () => ({
      getProgress: async () => ({
        progress: create(quest.ProgressSchema, {
          level: { level: 3, totalXp: 120n, xpIntoLevel: 20n, xpForNextLevel: 100n },
          xpToday: 20,
          badgeCount: 2,
        }),
      }),
      listMissions: async () =>
        create(quest.ListMissionsResponseSchema, {
          daily: [mission()],
          rerollsLeft: 1,
          dailyBonus: { claimed: 0, required: 3, reward: { coins: 30 } },
        }),
      getBalance: async () => ({ balance: { coins } }),
      claimMission: ({ missionId }: { missionId: string }) => {
        claims.push(missionId);
        return new Promise((resolve, reject) => {
          lose = () => {
            state = M.CLAIMED;
            coins += 10n;
            reject(new Error("the connection dropped before the answer arrived"));
          };
          answer = () => {
            state = M.CLAIMED;
            coins += 10n;
            resolve(
              create(quest.ClaimMissionResponseSchema, {
                mission: mission(),
                reward: { coins: 10, xp: 20 },
                granted: true,
              }),
            );
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
  };
}

async function show(children: ReactNode) {
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={new QueryClient()}>{children}</QueryClientProvider>
    </SafeAreaProvider>,
  );
}

describe("the quest screen", () => {
  beforeEach(() => jest.spyOn(Alert, "alert").mockImplementation(() => {}));
  afterEach(() => jest.clearAllMocks());

  it("shows the level, the coins and today's missions", async () => {
    server();
    await show(<QuestScreen />);
    expect((await screen.findByTestId("quest-level")).props.children).toBe(3);
    expect(await screen.findByText("スタンプを5こ送る")).toBeTruthy();
    await waitFor(() => expect(screen.getByTestId("quest-coins").props.children).toBe("🪙 100"));
    expect(screen.getByText("受け取り 0 / 3")).toBeTruthy();
  });

  it("sends a claim once however often it is tapped, and then shows the server's balance", async () => {
    const api = server();
    await show(<QuestScreen />);
    const claim = await screen.findByTestId("mission-claim-stamps_5");
    await fireEvent.press(claim);
    await fireEvent.press(claim);
    await fireEvent.press(claim);
    expect(api.claims).toEqual(["m1"]);
    api.answer();
    await waitFor(() => expect(screen.getByText("受け取りずみ")).toBeTruthy());
    await waitFor(() => expect(screen.getByTestId("quest-coins").props.children).toBe("🪙 110"));
    expect(api.claims).toEqual(["m1"]);
    expect(Alert.alert).toHaveBeenCalledWith("ミッション クリア!", "10コインと 20 XPをゲット!");
  });

  it("reads the balance and the mission again when a claim's answer is lost", async () => {
    const api = server();
    await show(<QuestScreen />);
    await waitFor(() => expect(screen.getByTestId("quest-coins").props.children).toBe("🪙 100"));
    await fireEvent.press(await screen.findByTestId("mission-claim-stamps_5"));
    api.loseAnswer();
    await waitFor(() => expect(screen.getByText("受け取りずみ")).toBeTruthy());
    await waitFor(() => expect(screen.getByTestId("quest-coins").props.children).toBe("🪙 110"));
    expect(api.claims).toEqual(["m1"]);
  });

  it("drops a claim answered after an account switch", async () => {
    const api = server();
    await show(<QuestScreen />);
    await fireEvent.press(await screen.findByTestId("mission-claim-stamps_5"));
    api.switchAccount();
    api.answer();
    await waitFor(() => expect(api.claims).toEqual(["m1"]));
    expect(Alert.alert).not.toHaveBeenCalled();
    expect(screen.queryByText("受け取りずみ")).toBeNull();
  });
});
