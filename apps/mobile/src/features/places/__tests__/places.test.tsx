import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { common, create, geo } from "@tokuly/api";
import { router } from "expo-router";
import type { ReactNode } from "react";
import { Alert, type AlertButton } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { currentAccount, isActive } from "#/features/session/account";
import { api } from "#/lib/api";
import { CheckInScreen } from "../check-in-screen";
import { PlaceScreen } from "../place-screen";
import { PlacesScreen } from "../places-screen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("#/features/session/account", () => ({ currentAccount: jest.fn(), isActive: jest.fn() }));
jest.mock("#/features/session/store", () => ({
  useMe: () => ({ timeZone: "Asia/Tokyo", user: { id: "me" } }),
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
  return { Map: Stub, Camera: () => null, Marker: Stub };
});

const cafe = create(geo.PlaceSchema, {
  id: "p1",
  ownerId: "someone",
  name: "みんなのカフェ",
  emoji: "☕",
  label: geo.PlaceLabel.CUSTOM,
  center: { lat: 35.66, lng: 139.7 },
  visibility: common.Visibility.PUBLIC,
  address: "渋谷区",
});
const home = create(geo.PlaceSchema, {
  id: "p2",
  ownerId: "me",
  name: "おうち",
  label: geo.PlaceLabel.HOME,
  center: { lat: 35.7, lng: 139.6 },
  visibility: common.Visibility.PRIVATE,
});
const suggestion = create(geo.PlaceSuggestionSchema, {
  center: { lat: 35.68, lng: 139.75 },
  suggestedName: "部室",
  suggestedLabel: geo.PlaceLabel.CUSTOM,
  visitCount: 4,
  totalSeconds: 7200n,
});

/**
 * One account's place services. `switchOnWrite` answers every write only
 * after the person switched to another account. What was written is listed.
 */
function placesOf({ switchOnWrite = false } = {}) {
  let switched = false;
  const written: { name: string; request: unknown }[] = [];
  const write = <T,>(name: string, request: unknown, answer: T) => {
    written.push({ name, request });
    if (switchOnWrite) switched = true;
    return Promise.resolve(answer);
  };
  const client = {
    service: () => ({
      listMyPlaces: async () => ({ places: [home] }),
      listPlaceSuggestions: async () => ({ suggestions: [suggestion] }),
      listSavedPlaces: async () => ({ places: [], page: { nextCursor: "" } }),
      getPlace: async ({ placeId }: { placeId: string }) =>
        create(geo.GetPlaceResponseSchema, {
          place: placeId === "p2" ? home : cafe,
          friendsVisited: [{ id: "u2", handle: "mio", displayName: "みお" }],
          totalVisitCount: 3,
        }),
      listVisits: async () => ({ visits: [], page: { nextCursor: "" } }),
      searchPlaces: (request: unknown) => {
        written.push({ name: "search", request });
        return Promise.resolve({ results: [{ place: cafe, name: cafe.name, friendsVisited: [] }] });
      },
      createPlace: (request: unknown) => write("create", request, { place: { name: "部室" } }),
      savePlace: (request: unknown) => write("save", request, {}),
      unsavePlace: (request: unknown) => write("unsave", request, {}),
      deletePlace: (request: unknown) => write("delete", request, {}),
      checkIn: (request: unknown) => write("checkIn", request, { visit: {} }),
    }),
  };
  jest.mocked(api).mockReturnValue(client as never);
  jest
    .mocked(currentAccount)
    .mockReturnValue({ sessionId: "s1", userId: "me", api: client as never });
  jest.mocked(isActive).mockImplementation(() => !switched);
  return written;
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

beforeEach(() => jest.spyOn(Alert, "alert").mockImplementation(() => {}));
afterEach(() => {
  jest.clearAllMocks();
  jest.restoreAllMocks();
});

describe("the places screen", () => {
  it("shows the own places and the slots still free", async () => {
    placesOf();
    await show(<PlacesScreen />);
    expect(await screen.findByText("おうち")).toBeTruthy();
    expect(screen.getByTestId("places-add-school")).toBeTruthy();
    expect(screen.queryByTestId("places-add-home")).toBeNull();
  });

  it("registers a suggested spot as a private place", async () => {
    const written = placesOf();
    await show(<PlacesScreen />);
    await fireEvent.press(await screen.findByTestId("places-suggestion-accept"));
    await waitFor(() => expect(written).toHaveLength(1));
    expect(written[0]?.request).toMatchObject({
      name: "部室",
      visibility: common.Visibility.PRIVATE,
      center: { lat: 35.68, lng: 139.75 },
    });
  });

  it("puts a suggestion off without telling the server", async () => {
    const written = placesOf();
    await show(<PlacesScreen />);
    await fireEvent.press(await screen.findByTestId("places-suggestion-dismiss"));
    await waitFor(() => expect(screen.queryByTestId("places-suggestion")).toBeNull());
    expect(written).toEqual([]);
  });

  it("searches by name without the phone's position", async () => {
    const written = placesOf();
    await show(<PlacesScreen />);
    await fireEvent.press(screen.getByTestId("places-tab-search"));
    await fireEvent.changeText(screen.getByTestId("places-search"), "カフェ");
    expect(await screen.findByText("みんなのカフェ")).toBeTruthy();
    expect(written).toEqual([{ name: "search", request: { query: "カフェ", limit: 20 } }]);
  });
});

describe("a place", () => {
  it("is saved by someone it does not belong to", async () => {
    const written = placesOf();
    await show(<PlaceScreen placeId="p1" />);
    await fireEvent.press(await screen.findByTestId("place-save"));
    await waitFor(() => expect(written.map((entry) => entry.name)).toEqual(["save"]));
    expect(await screen.findByText("お気に入り済み")).toBeTruthy();
    expect(screen.queryByTestId("place-delete")).toBeNull();
  });

  it("of one's own is deleted after asking", async () => {
    const written = placesOf();
    await show(<PlaceScreen placeId="p2" />);
    await fireEvent.press(await screen.findByTestId("place-delete"));
    const buttons = jest.mocked(Alert.alert).mock.calls[0]?.[2] as AlertButton[];
    buttons.find((button) => button.style === "destructive")?.onPress?.();
    await waitFor(() => expect(written.map((entry) => entry.name)).toEqual(["delete"]));
    expect(router.back).toHaveBeenCalled();
    expect(screen.queryByTestId("place-save")).toBeNull();
  });

  it("is checked in at with a few words", async () => {
    const written = placesOf();
    await show(<CheckInScreen placeId="p1" />);
    await fireEvent.changeText(await screen.findByTestId("check-in-message"), "まったり");
    await fireEvent.press(screen.getByTestId("check-in-submit"));
    await waitFor(() => expect(written).toHaveLength(1));
    expect(written[0]?.request).toEqual({
      where: { case: "placeId", value: "p1" },
      message: "まったり",
    });
    expect(router.back).toHaveBeenCalled();
  });
});

describe("a place write answered after an account switch", () => {
  it("does not close the check-in over the next account's screens", async () => {
    const written = placesOf({ switchOnWrite: true });
    await show(<CheckInScreen placeId="p1" />);
    await fireEvent.press(await screen.findByTestId("check-in-submit"));
    await waitFor(() => expect(written).toHaveLength(1));
    expect(router.back).not.toHaveBeenCalled();
    expect(Alert.alert).not.toHaveBeenCalled();
  });

  it("does not mark the place saved for the next account", async () => {
    const written = placesOf({ switchOnWrite: true });
    await show(<PlaceScreen placeId="p1" />);
    await fireEvent.press(await screen.findByTestId("place-save"));
    await waitFor(() => expect(written).toHaveLength(1));
    expect(screen.queryByText("お気に入り済み")).toBeNull();
  });
});
