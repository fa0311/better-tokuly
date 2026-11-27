import { QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { common, create, stamp } from "@tokuly/api";
import type { ReactNode } from "react";
import { Alert } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { currentAccount, isActive } from "#/features/session/account";
import { api } from "#/lib/api";
import { screenFor } from "#/seams/links";
import { testQueryClient } from "#/testing/query";
import { PackScreen } from "../pack-screen";
import { StampPicker } from "../picker";
import { StoreScreen } from "../store-screen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("#/features/session/account", () => ({ currentAccount: jest.fn(), isActive: jest.fn() }));
jest.mock("expo-router", () => ({
  router: { push: jest.fn(), back: jest.fn(), replace: jest.fn() },
  Stack: { Screen: () => null },
}));

const ref = (id: string, name: string) =>
  create(common.StampRefSchema, {
    id,
    packId: "p1",
    name,
    imageUrl: `/media/${id}.webp`,
    format: common.StampFormat.STATIC,
  });
const item = (id: string, name: string) =>
  create(stamp.StampSchema, { stampRef: ref(id, name), usable: true });

const freePack = (installed: boolean) =>
  create(stamp.PackSchema, {
    summary: {
      id: "p1",
      shortId: "neko",
      name: "ねこスタンプ",
      price: 0,
      stampCount: 1,
      status: stamp.PackStatus.PUBLISHED,
      viewer: { installed, usable: true },
    },
    stamps: [item("s1", "にゃー")],
  });

/** The stamp services of one account; `switchOnInstall` lets the person switch before the answer. */
function server({ switchOnInstall = false } = {}) {
  let switched = false;
  let installed = false;
  const calls: string[] = [];
  const client = {
    service: () => ({
      getPack: async () => ({ pack: freePack(installed) }),
      getLibrary: async () =>
        create(stamp.GetLibraryResponseSchema, {
          recents: [item("s1", "にゃー")],
          installed: [freePack(true).summary!],
        }),
      searchPacks: async (request: { query: string }) => {
        calls.push(`search ${request.query}`);
        return { packs: [freePack(false).summary!] };
      },
      listStorePacks: async (request: { tag: string }) => {
        calls.push(`tag ${request.tag}`);
        return { packs: [freePack(false).summary!] };
      },
      installPack: async () => {
        calls.push("install");
        if (switchOnInstall) switched = true;
        else installed = true;
        return { pack: freePack(true).summary };
      },
    }),
  };
  jest.mocked(api).mockReturnValue(client as never);
  jest
    .mocked(currentAccount)
    .mockReturnValue({ sessionId: "s1", userId: "u1", api: client as never });
  jest.mocked(isActive).mockImplementation(() => !switched);
  return calls;
}

async function show(children: ReactNode) {
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={testQueryClient()}>{children}</QueryClientProvider>
    </SafeAreaProvider>,
  );
}

beforeEach(() => jest.spyOn(Alert, "alert").mockImplementation(() => {}));
afterEach(() => jest.restoreAllMocks());

describe("PackScreen", () => {
  it("adds a free pack to the picker in a tap", async () => {
    const calls = server();
    await show(<PackScreen packId="neko" />);
    await fireEvent.press(await screen.findByTestId("pack-install"));
    await waitFor(() => expect(calls).toEqual(["install"]));
    expect(await screen.findByTestId("pack-uninstall")).toBeOnTheScreen();
    expect(Alert.alert).toHaveBeenCalledWith("追加しました！ピッカーから使えるよ");
  });

  it("shows nothing of an answer that came after the person switched accounts", async () => {
    const calls = server({ switchOnInstall: true });
    await show(<PackScreen packId="neko" />);
    await fireEvent.press(await screen.findByTestId("pack-install"));
    await waitFor(() => expect(calls).toEqual(["install"]));
    expect(screen.getByTestId("pack-install")).toBeOnTheScreen();
    expect(Alert.alert).not.toHaveBeenCalled();
  });
});

describe("StoreScreen", () => {
  it("opens on the search or the tag a link carries", async () => {
    const calls = server();
    await show(<StoreScreen query="neko" />);
    expect(await screen.findByDisplayValue("neko")).toBeOnTheScreen();
    await waitFor(() => expect(calls).toContain("search neko"));
    await show(<StoreScreen tag="cat" />);
    await waitFor(() => expect(calls).toContain("tag cat"));
  });
});

describe("StampPicker", () => {
  it("hands over the stamp tapped and closes", async () => {
    server();
    const onPick = jest.fn();
    const onClose = jest.fn();
    await show(<StampPicker visible onPick={onPick} onClose={onClose} />);
    await fireEvent.press(await screen.findByTestId("stamp-s1"));
    expect(onClose).toHaveBeenCalled();
    expect(onPick).toHaveBeenCalledWith(expect.objectContaining({ id: "s1", name: "にゃー" }));
  });

  it("picks once per opening, however fast the second tap comes", async () => {
    server();
    const onPick = jest.fn();
    await show(<StampPicker visible onPick={onPick} onClose={() => {}} />);
    const cell = await screen.findByTestId("stamp-s1");
    await fireEvent.press(cell);
    await fireEvent.press(cell);
    expect(onPick).toHaveBeenCalledTimes(1);
  });
});

describe("stamp links", () => {
  const web = (path: string) => `/web?path=${encodeURIComponent(path)}`;

  it("open the store, a pack and the library in the app", () => {
    expect(screenFor("/stamps")).toBe("/stamps");
    expect(screenFor("/stamps/browse?q=neko")).toBe("/stamps?q=neko");
    expect(screenFor("/stamps/library")).toBe("/stamps/library");
    expect(screenFor("/stamps/Ab12Cd34Ef0")).toBe("/stamps/Ab12Cd34Ef0");
  });

  it("leave the studio and creators' pages on the web", () => {
    expect(screenFor("/stamps/studio")).toBe(web("/stamps/studio"));
    expect(screenFor("/stamps/studio/p1")).toBe(web("/stamps/studio/p1"));
    expect(screenFor("/stamps/creator/u1")).toBe(web("/stamps/creator/u1"));
  });
});
