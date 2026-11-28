import { QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen } from "@testing-library/react-native";
import { create, emoji } from "@tokuly/api";
import { EMPTY_CATALOG } from "@tokuly/core/emoji";
import * as ImagePicker from "expo-image-picker";
import { Alert } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { syncCatalog, useCatalogStore } from "#/features/emoji/catalog";
import { api } from "#/lib/api";
import { testQueryClient } from "#/testing/query";
import { Composer } from "../composer";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("#/lib/realtime", () => ({ realtime: () => ({ sendTyping: jest.fn() }) }));
jest.mock("../outbox", () => ({ send: jest.fn() }));
jest.mock("../data", () => ({
  editMessage: jest.fn(),
  useCapabilities: () => ({ maxTextLength: 12, maxAttachments: 4 }),
}));
jest.mock("expo-router", () => ({ router: { push: jest.fn() } }));
jest.mock("expo-image-picker", () => ({
  UIImagePickerPreferredAssetRepresentationMode: { Compatible: "compatible" },
  requestCameraPermissionsAsync: jest.fn(async () => ({ granted: true })),
  requestMediaLibraryPermissionsAsync: jest.fn(async () => ({ granted: true })),
  launchCameraAsync: jest.fn(),
  launchImageLibraryAsync: jest.fn(),
}));

async function openComposer() {
  const service = {
    getCatalog: async () => ({
      version: 1n,
      emojis: [
        create(emoji.CustomEmojiSchema, { id: "e1", name: "neko", image: { url: "/n.webp" } }),
      ],
      categories: [],
    }),
    getDeck: async () => ({ items: [], isDefault: true }),
    listRecentEmoji: async () => ({ items: [] }),
  };
  jest.mocked(api).mockReturnValue({ service: () => service } as never);
  useCatalogStore.setState({ catalog: EMPTY_CATALOG });
  await syncCatalog();
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={testQueryClient()}>
        <Composer
          conversationId="c1"
          myId="u1"
          replyTo={null}
          editing={null}
          onDone={() => {}}
          textOnly={false}
        />
      </QueryClientProvider>
    </SafeAreaProvider>,
  );
}

describe("Composer", () => {
  it("writes a custom emoji in as its code", async () => {
    await openComposer();
    await fireEvent.changeText(screen.getByTestId("composer-input"), "やあ");
    await fireEvent(screen.getByTestId("composer-input"), "selectionChange", {
      nativeEvent: { selection: { start: 2, end: 2 } },
    });
    await fireEvent.press(screen.getByTestId("composer-emoji"));
    await fireEvent.press(await screen.findByTestId("emoji-neko"));
    expect(screen.getByTestId("composer-input").props.value).toBe("やあ :neko: ");
  });

  it("adds no emoji past the longest message the conversation takes", async () => {
    await openComposer();
    await fireEvent.changeText(screen.getByTestId("composer-input"), "1234567");
    await fireEvent(screen.getByTestId("composer-input"), "selectionChange", {
      nativeEvent: { selection: { start: 7, end: 7 } },
    });
    await fireEvent.press(screen.getByTestId("composer-emoji"));
    await fireEvent.press(await screen.findByTestId("emoji-neko"));
    expect(screen.getByTestId("composer-input").props.value).toBe("1234567");
  });

  // An emulator has no camera; an iCloud photo may not download: the tap says so instead of nothing.
  it.each([
    ["camera", ImagePicker.launchCameraAsync],
    ["library", ImagePicker.launchImageLibraryAsync],
  ] as const)("says the %s could not open the photo", async (source, launch) => {
    jest.mocked(launch).mockRejectedValueOnce(new Error("no camera available"));
    const alert = jest.spyOn(Alert, "alert").mockImplementation(() => {});
    await openComposer();
    await fireEvent.press(screen.getByTestId("composer-photo"));
    await fireEvent.press(screen.getByTestId(`sheet-${source}`));
    expect(alert).toHaveBeenCalledWith("ファイルを読み込めなかったよ。もう一度えらんでね");
  });
});
