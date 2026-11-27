import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { Code, ConnectError, create, fromDate, geo } from "@tokuly/api";
import type { ReactNode } from "react";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { currentAccount, isActive } from "#/features/session/account";
import { api } from "#/lib/api";
import { FootprintsScreen } from "../footprints-screen";
import links from "../links";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("#/features/session/account", () => ({ currentAccount: jest.fn(), isActive: jest.fn() }));
jest.mock("#/features/session/store", () => ({
  useMe: () => ({ timeZone: "Asia/Tokyo", user: { id: "me" } }),
}));
jest.mock("expo-router", () => ({
  router: { push: jest.fn(), back: jest.fn(), navigate: jest.fn() },
  Stack: { Screen: () => null },
}));

const tokyo = create(geo.RegionSchema, {
  id: "JP-13",
  kind: geo.RegionKind.ADMIN1,
  name: "Tokyo",
  localName: "東京都",
  countryCode: "JP",
});

/**
 * One account's footprint service; `switchOnSeen` lets another account take
 * over while the recap is marked seen, `boardDown` fails the ranking.
 */
function footprintsOf({ cells = 125, switchOnSeen = false, boardDown = false } = {}) {
  let switched = false;
  let seen = false;
  const asked: unknown[] = [];
  const client = {
    service: () => ({
      getStats: async (request: unknown) => {
        asked.push(request);
        return create(geo.GetFootprintStatsResponseSchema, {
          totalCellCount: BigInt(cells),
          countryCount: 1,
          world: { exploredKm2: 3.2 },
          regions: cells
            ? [{ region: tokyo, percent: 0.24, cellCount: BigInt(cells), exploredKm2: 3.2 }]
            : [],
        });
      },
      getLeaderboard: async () => {
        if (boardDown) throw new ConnectError("no network", Code.Unavailable);
        return create(geo.GetLeaderboardResponseSchema, {
          entries: [
            {
              rank: 1,
              user: { id: "u2", handle: "mio", displayName: "みお" },
              percent: 0.5,
              cellCount: 300n,
            },
            {
              rank: 2,
              user: { id: "me", handle: "me", displayName: "わたし" },
              percent: 0.24,
              cellCount: 125n,
            },
          ],
        });
      },
      getRecap: async ({ markSeen }: { markSeen: boolean }) => {
        asked.push({ markSeen });
        if (markSeen && switchOnSeen) switched = true;
        const fresh = !seen;
        if (markSeen) seen = true;
        return create(geo.GetRecapResponseSchema, {
          newCellCount: fresh ? 12n : 0n,
          distanceMeters: 1500,
          // 2 May, 05:00 in Tokyo.
          since: fromDate(new Date("2026-05-01T20:00:00Z")),
        });
      },
    }),
  };
  jest.mocked(api).mockReturnValue(client as never);
  jest
    .mocked(currentAccount)
    .mockReturnValue({ sessionId: "s1", userId: "me", api: client as never });
  jest.mocked(isActive).mockImplementation(() => !switched);
  return asked;
}

async function show(children: ReactNode) {
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider
        client={
          new QueryClient({
            defaultOptions: {
              queries: { retry: false, gcTime: Infinity },
              mutations: { gcTime: Infinity },
            },
          })
        }
      >
        {children}
      </QueryClientProvider>
    </SafeAreaProvider>,
  );
}

afterEach(() => jest.clearAllMocks());

describe("the footprints screen", () => {
  it("shows the account's own level, regions and ranking", async () => {
    const asked = footprintsOf();
    await show(<FootprintsScreen />);
    // 125 cells: Lv.6 (5·25 = 125), a townie.
    expect(await screen.findByText("6")).toBeTruthy();
    expect(screen.getByText("まちの探検家")).toBeTruthy();
    expect(screen.getByText("東京都を 0.24% 開拓した!")).toBeTruthy();
    expect(await screen.findByTestId("footprints-rank-u2")).toBeTruthy();
    // Only the account's own numbers are asked for.
    expect(asked).toContainEqual({ userId: "" });
  });

  it("starts in the fog before anything is uncovered", async () => {
    footprintsOf({ cells: 0 });
    await show(<FootprintsScreen />);
    expect(await screen.findByText("まだ霧の中…!")).toBeTruthy();
  });

  it("dates the recap on the account's calendar, not the phone's", async () => {
    footprintsOf();
    await show(<FootprintsScreen />);
    await screen.findByTestId("footprints-recap");
    expect(screen.getByText(/5月2日 から/)).toBeTruthy();
  });

  it("says when the ranking cannot be read, instead of calling the account alone", async () => {
    footprintsOf({ boardDown: true });
    await show(<FootprintsScreen />);
    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(screen.queryByText("まだ誰もいないよ")).toBeNull();
    expect(screen.queryByText("ともだちを追加")).toBeNull();
  });

  it("marks the recap seen when the person says so", async () => {
    const asked = footprintsOf();
    await show(<FootprintsScreen />);
    await fireEvent.press(await screen.findByTestId("footprints-recap-seen"));
    await waitFor(() => expect(screen.queryByTestId("footprints-recap")).toBeNull());
    expect(asked).toContainEqual({ markSeen: true });
  });

  it("leaves the recap alone when another account took over meanwhile", async () => {
    footprintsOf({ switchOnSeen: true });
    await show(<FootprintsScreen />);
    await fireEvent.press(await screen.findByTestId("footprints-recap-seen"));
    await waitFor(() => expect(currentAccount).toHaveBeenCalled());
    expect(screen.getByTestId("footprints-recap")).toBeTruthy();
  });
});

describe("footprint links", () => {
  it("open the own footprints here and someone else's on the web", () => {
    const [own] = links;
    expect(own?.screen("/footprints".match(own.path)!, "/footprints")).toBe("/footprints");
    expect(own?.screen("/footprints".match(own.path)!, "/footprints?user=u2")).toMatch(/^\/web/);
  });
});
