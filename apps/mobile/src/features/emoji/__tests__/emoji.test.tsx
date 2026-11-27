import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { create, emoji, type MessageInitShape } from "@tokuly/api";
import { EMPTY_CATALOG } from "@tokuly/core/emoji";
import type { ReactNode } from "react";
import { Alert, Text } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { currentAccount, isActive } from "#/features/session/account";
import { api } from "#/lib/api";
import { screenFor } from "#/seams/links";
import { syncCatalog, useCatalogStore } from "../catalog";
import { DetailScreen } from "../detail-screen";
import { useEmojiText } from "../emoji-text";
import { EmojiPicker } from "../picker";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("#/features/session/account", () => ({ currentAccount: jest.fn(), isActive: jest.fn() }));
jest.mock("expo-router", () => ({
  router: { push: jest.fn(), back: jest.fn(), replace: jest.fn() },
  Stack: { Screen: () => null },
}));

const custom = (
  name: string,
  fields: Omit<MessageInitShape<typeof emoji.CustomEmojiSchema>, "$typeName"> = {},
) =>
  create(emoji.CustomEmojiSchema, {
    id: `id-${name}`,
    name,
    image: { url: `/media/${name}.webp` },
    ...fields,
  });

/** The emoji service of one account; `switchOnSave` lets the person switch before the deck is saved. */
function server({ switchOnSave = false } = {}) {
  let switched = false;
  const calls: string[] = [];
  const service = {
    getCatalog: async () => {
      calls.push("catalog");
      return {
        version: 2n,
        emojis: [
          custom("neko", {
            aliases: ["ねこ"],
            category: "どうぶつ",
            image: {
              url: "/media/neko.webp",
              variants: [
                { size: 32, url: "/media/neko-32.webp" },
                { size: 128, url: "/media/neko-128.webp" },
              ],
            },
          }),
          custom("inu"),
          custom("ecchi", { sensitive: true }),
        ],
        categories: [{ name: "どうぶつ", emojiCount: 1 }],
      };
    },
    getCatalogChanges: async ({ sinceVersion }: { sinceVersion: bigint }) => {
      calls.push(`changes:${sinceVersion}`);
      return {
        version: 3n,
        full: false,
        upserted: [custom("tori")],
        removedIds: ["id-inu"],
        categories: [],
      };
    },
    getDeck: async () => ({ items: [], isDefault: true }),
    listRecentEmoji: async () => ({ items: [] }),
    getEmoji: async () => ({
      emoji: { emoji: custom("neko", { aliases: ["ねこ"] }), license: "CC0" },
    }),
    setDeck: async ({ emojis }: { emojis: string[] }) => {
      calls.push(`deck:${emojis.join(",")}`);
      if (switchOnSave) switched = true;
      return { items: emojis.map((item) => ({ emoji: item })) };
    },
  };
  const client = { service: () => service };
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
      <QueryClientProvider client={new QueryClient()}>{children}</QueryClientProvider>
    </SafeAreaProvider>,
  );
}

beforeEach(() => {
  useCatalogStore.setState({ catalog: EMPTY_CATALOG });
  jest.spyOn(Alert, "alert").mockImplementation(() => {});
});
afterEach(() => jest.restoreAllMocks());

describe("the catalogue copy", () => {
  it("is read whole once, then only its changes", async () => {
    const calls = server();
    await syncCatalog();
    expect(useCatalogStore.getState().catalog.emojis.map((entry) => entry.name)).toEqual([
      "ecchi",
      "inu",
      "neko",
    ]);
    await syncCatalog(3n);
    expect(calls).toEqual(["catalog", "changes:2"]);
    expect(useCatalogStore.getState().catalog.emojis.map((entry) => entry.name)).toEqual([
      "ecchi",
      "neko",
      "tori",
    ]);
    // An announcement of a version the copy has asks nothing.
    await syncCatalog(3n);
    expect(calls).toHaveLength(2);
  });
});

function Line({ text, size }: { text: string; size: number }) {
  const withEmoji = useEmojiText();
  return <Text testID="line">{withEmoji(text, size)}</Text>;
}

describe("text with custom emoji", () => {
  it("keeps a sensitive emoji as its code: the phone has no way to blur it inline", async () => {
    server();
    await syncCatalog();
    await show(<Line text="これ :ecchi:" size={20} />);
    expect(screen.queryByLabelText(":ecchi:")).toBeNull();
    expect(screen.getByTestId("line")).toHaveTextContent("これ :ecchi:");
  });
});

describe("EmojiPicker", () => {
  it("finds an emoji by its Japanese alias and hands over its code", async () => {
    server();
    await syncCatalog();
    const onPick = jest.fn();
    await show(<EmojiPicker visible onPick={onPick} onClose={() => {}} />);
    expect(await screen.findByTestId("emoji-inu")).toBeOnTheScreen();
    await fireEvent.changeText(screen.getByTestId("emoji-search"), "ネコ");
    await waitFor(() => expect(screen.queryByTestId("emoji-inu")).not.toBeOnTheScreen());
    await fireEvent.press(screen.getByTestId("emoji-neko"));
    expect(onPick).toHaveBeenCalledWith(":neko:");
  });
});

describe("DetailScreen", () => {
  it("pins the emoji to the deck", async () => {
    const calls = server();
    await show(<DetailScreen name="neko" />);
    await fireEvent.press(await screen.findByTestId("emoji-pin"));
    await waitFor(() => expect(calls).toContain("deck::neko:"));
    expect(await screen.findByTestId("emoji-unpin")).toBeOnTheScreen();
    expect(Alert.alert).toHaveBeenCalledWith("デッキにピンどめしたよ！");
  });

  it("shows nothing of a save answered after the person switched accounts", async () => {
    const calls = server({ switchOnSave: true });
    await show(<DetailScreen name="neko" />);
    await fireEvent.press(await screen.findByTestId("emoji-pin"));
    await waitFor(() => expect(calls).toContain("deck::neko:"));
    expect(screen.getByTestId("emoji-pin")).toBeOnTheScreen();
    expect(Alert.alert).not.toHaveBeenCalled();
  });
});

describe("emoji links", () => {
  const web = (path: string) => `/web?path=${encodeURIComponent(path)}`;

  it("open the catalogue, the deck and an emoji in the app; submissions on the web", () => {
    expect(screenFor("/emoji")).toBe("/emoji");
    expect(screenFor("/emoji/my-deck")).toBe("/emoji");
    expect(screenFor("/emoji/neko")).toBe("/emoji/neko");
    expect(screenFor("/emoji/new-emoji")).toBe(web("/emoji/new-emoji"));
    expect(screenFor("/emoji/submissions/e1")).toBe(web("/emoji/submissions/e1"));
  });
});
