import type { Timestamp } from "@bufbuild/protobuf/wkt";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { create, fromDate, geo, toDate } from "@tokuly/api";
import { router } from "expo-router";
import type { ReactNode } from "react";
import { Alert } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { currentAccount, isActive } from "#/features/session/account";
import { api } from "#/lib/api";
import { DayScreen } from "../day-screen";
import { DeleteHistoryScreen } from "../delete-screen";
import { HistoryScreen } from "../history-screen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("#/features/session/account", () => ({ currentAccount: jest.fn(), isActive: jest.fn() }));
let mockTimeZone = "Asia/Tokyo";
jest.mock("#/features/session/store", () => ({ useMe: () => ({ timeZone: mockTimeZone }) }));
let mockRecordHistory = true;
jest.mock("#/features/settings/data", () => ({
  useSettings: () => ({
    data: { privacy: { recordHistory: mockRecordHistory } },
  }),
}));
jest.mock("expo-router", () => ({
  router: { push: jest.fn(), back: jest.fn(), replace: jest.fn() },
  Stack: { Screen: () => null },
}));
// The map is the device's.
jest.mock("@maplibre/maplibre-react-native", () => {
  const { View } = require("react-native");
  const Stub = ({ children }: { children?: ReactNode }) =>
    require("react").createElement(View, null, children);
  return { Map: Stub, Camera: () => null, Marker: Stub, GeoJSONSource: Stub, Layer: () => null };
});

// Sunday 22 November 2026, 12:00 in Tokyo.
const NOW = Date.parse("2026-11-22T03:00:00Z");
const at = (iso: string) => fromDate(new Date(iso));

const cafe = create(geo.VisitSchema, {
  id: "v1",
  name: "カフェ",
  center: { lat: 35.66, lng: 139.7 },
  arrivedAt: at("2026-11-22T01:00:00Z"),
  departedAt: at("2026-11-22T02:00:00Z"),
  durationSeconds: 3600n,
});
const school = create(geo.VisitSchema, {
  id: "v2",
  name: "がっこう",
  center: { lat: 35.68, lng: 139.72 },
  arrivedAt: at("2026-11-21T23:00:00Z"),
  departedAt: at("2026-11-22T00:30:00Z"),
  durationSeconds: 5400n,
});

interface Deletion {
  from?: Timestamp;
  to?: Timestamp;
  includeFootprints: boolean;
}

/**
 * One account's history service. `switchOnWrite` answers every write only
 * after the person switched to another account. What was written is listed.
 */
function historyOf({ switchOnWrite = false, emptyDays = false, holdWrites = false } = {}) {
  let switched = false;
  const written: { name: string; request: unknown; answer(): void }[] = [];
  const write = <T,>(name: string, request: unknown, answer: T) =>
    new Promise<T>((resolve) => {
      written.push({ name, request, answer: () => resolve(answer) });
      if (switchOnWrite) switched = true;
      if (!holdWrites) resolve(answer);
    });
  const routes: { from?: Timestamp; to?: Timestamp }[] = [];
  const client = {
    service: () => ({
      listActiveDays: async () => ({
        days: [
          create(geo.DaySummarySchema, { date: "2026-11-21", distanceMeters: 3200, visitCount: 2 }),
        ],
      }),
      getDay: async ({ date }: { date: string }) =>
        create(
          geo.GetDayResponseSchema,
          emptyDays
            ? {}
            : {
                summary: { date, distanceMeters: 3200, visitCount: 2, newCellCount: 4 },
                visits: [cafe, school],
                friendsMet: [{ id: "u2", handle: "mio", displayName: "みお" }],
                memo: { text: "たのしかった", photos: [{ assetId: "p1" }] },
                pathPolyline: "_p~iF~ps|U_ulLnnqC",
              },
        ),
      getRoute: async (request: { from?: Timestamp; to?: Timestamp }) => {
        routes.push(request);
        return { points: [] };
      },
      listVisits: async () => ({ visits: [cafe], page: { nextCursor: "" } }),
      listNights: async () => ({ nights: [], page: { nextCursor: "" } }),
      getStats: async () => create(geo.GetStatsResponseSchema, {}),
      listMyPlaces: async () => ({ places: [] }),
      setDayMemo: (request: unknown) =>
        write("memo", request, { memo: { text: "あめだった", photos: [{ assetId: "p1" }] } }),
      deleteHistory: (request: Deletion) => write("delete", request, {}),
    }),
  };
  jest.mocked(api).mockReturnValue(client as never);
  jest
    .mocked(currentAccount)
    .mockReturnValue({ sessionId: "s1", userId: "u1", api: client as never });
  jest.mocked(isActive).mockImplementation(() => !switched);
  return Object.assign(written, { routes });
}

let queryClient: QueryClient;

async function show(children: ReactNode) {
  queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: Infinity },
      mutations: { gcTime: Infinity },
    },
  });
  const tree = (content: ReactNode) => (
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={queryClient}>{content}</QueryClientProvider>
    </SafeAreaProvider>
  );
  const shown = await render(tree(children));
  return {
    close: () => shown.unmount(),
    again: (content: ReactNode) => shown.rerender(tree(content)),
  };
}

beforeEach(() => {
  jest.spyOn(Alert, "alert").mockImplementation(() => {});
  jest.spyOn(Date, "now").mockReturnValue(NOW);
});
afterEach(() => {
  jest.clearAllMocks();
  jest.restoreAllMocks();
  mockRecordHistory = true;
  mockTimeZone = "Asia/Tokyo";
});

describe("the history screen", () => {
  it("marks the days with something on the account's calendar and opens them", async () => {
    historyOf();
    await show(<HistoryScreen />);
    expect(await screen.findByTestId("history-month-totals")).toBeTruthy();
    await fireEvent.press(screen.getByTestId("history-day-2026-11-21"));
    expect(router.push).toHaveBeenCalledWith("/history/2026-11-21");
    expect(screen.queryByTestId("history-recording-off")).toBeNull();
  });

  it("says when the account keeps no history", async () => {
    mockRecordHistory = false;
    historyOf();
    await show(<HistoryScreen />);
    expect(screen.getByTestId("history-recording-off")).toBeTruthy();
  });

  it("lists the visits, each leading to its day", async () => {
    historyOf();
    await show(<HistoryScreen />);
    await fireEvent.press(screen.getByTestId("history-tab-visits"));
    await fireEvent.press(await screen.findByTestId("visit-open-day-v1"));
    expect(router.push).toHaveBeenCalledWith("/history/2026-11-22");
  });
});

describe("a day", () => {
  it("tells the day in order on the account's clock: the visits, the move between them, who was met", async () => {
    historyOf();
    await show(<DayScreen date="2026-11-22" />);
    expect(await screen.findByTestId("history-day-map")).toBeTruthy();
    expect(screen.getByText("がっこう")).toBeTruthy();
    // 08:00 to 09:30 Tokyo time, then the café.
    expect(screen.getByText(/08:00–09:30/)).toBeTruthy();
    expect(screen.getByText(/10:00–11:00/)).toBeTruthy();
    expect(screen.getByText("1人のともだちと会ったよ!")).toBeTruthy();
  });

  it("saves the memo as the account, keeping its photos", async () => {
    const written = historyOf();
    await show(<DayScreen date="2026-11-22" />);
    await fireEvent.changeText(await screen.findByTestId("history-memo"), "あめだった");
    await fireEvent.press(screen.getByTestId("history-memo-save"));
    await waitFor(() => expect(written).toHaveLength(1));
    expect(written[0]?.request).toMatchObject({
      date: "2026-11-22",
      text: "あめだった",
      photoAssetIds: ["p1"],
    });
  });
});

describe("a day, when the account's settings change", () => {
  it("does not promise that today will be kept while the account keeps no history", async () => {
    mockRecordHistory = false;
    historyOf({ emptyDays: true });
    await show(<DayScreen date="2026-11-22" />);
    expect(await screen.findByTestId("history-recording-off")).toBeTruthy();
    expect(screen.queryByText(/ここに残るよ/)).toBeNull();
  });
});

describe("deleting history", () => {
  it("deletes a range from the first day's midnight to the end of the last, on the account's calendar", async () => {
    const written = historyOf();
    await show(<DeleteHistoryScreen />);
    await fireEvent.press(screen.getByTestId("history-delete-from-previous"));
    await fireEvent(screen.getByTestId("history-delete-footprints"), "valueChange", true);
    await fireEvent.press(screen.getByTestId("history-delete-confirm"));
    await waitFor(() => expect(written).toHaveLength(1));
    const request = written[0]?.request as Deletion;
    expect(toDate(request.from)?.toISOString()).toBe("2026-11-20T15:00:00.000Z");
    expect(toDate(request.to)?.toISOString()).toBe("2026-11-22T15:00:00.000Z");
    expect(request.includeFootprints).toBe(true);
    expect(router.back).toHaveBeenCalled();
  });

  it("deletes everything when asked, and forgets every cached answer about history, footprints and places", async () => {
    const written = historyOf();
    await show(<DeleteHistoryScreen />);
    queryClient.setQueryData(["geo", "footprints", "stats", ""], { totalCellCount: 9 });
    queryClient.setQueryData(["geo", "places", "mine"], { places: [] });
    await fireEvent.press(screen.getByTestId("history-delete-scope-all"));
    await fireEvent.press(screen.getByTestId("history-delete-confirm"));
    await waitFor(() => expect(written).toHaveLength(1));
    expect(written[0]?.request).toEqual({ includeFootprints: false });
    await waitFor(() =>
      expect(queryClient.getQueryData(["geo", "footprints", "stats", ""])).toBeUndefined(),
    );
    expect(queryClient.getQueryData(["geo", "places", "mine"])).toBeUndefined();
  });

  it("answered after an account switch, does nothing to the next account's screens", async () => {
    const written = historyOf({ switchOnWrite: true });
    await show(<DeleteHistoryScreen date="2026-11-21" />);
    queryClient.setQueryData(["geo", "places", "mine"], { places: [] });
    await fireEvent.press(screen.getByTestId("history-delete-confirm"));
    await waitFor(() => expect(written).toHaveLength(1));
    expect(router.back).not.toHaveBeenCalled();
    expect(Alert.alert).not.toHaveBeenCalled();
    expect(queryClient.getQueryData(["geo", "places", "mine"])).toEqual({ places: [] });
  });
});
