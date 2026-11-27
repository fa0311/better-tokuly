import { QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { create, shorts } from "@tokuly/api";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { api } from "#/lib/api";
import { testQueryClient } from "#/testing/query";
import { ShortsFeed } from "../feed";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("expo-router", () => ({ router: { push: jest.fn() }, useFocusEffect: () => {} }));
jest.mock("expo-video", () => {
  const { View } = require("react-native");
  return {
    VideoView: () => require("react").createElement(View),
    useVideoPlayer: () => ({
      addListener: () => ({ remove: () => {} }),
      play: jest.fn(),
      pause: jest.fn(),
      currentTime: 0,
    }),
  };
});

const short = create(shorts.ShortSchema, {
  id: "u1",
  shortId: "abc",
  caption: "maestro",
  channel: { name: "ch" },
});
const getForYouFeed = jest
  .fn()
  .mockResolvedValueOnce({ items: [{ short }], page: { nextCursor: "e30", hasMore: true } })
  .mockResolvedValue({
    items: [{ short, repeat: true }],
    page: { nextCursor: "e30", hasMore: true },
  });

describe("ShortsFeed", () => {
  it("reads the next page while the first short is on screen, so there is one to swipe to", async () => {
    jest.mocked(api).mockReturnValue({
      service: () => ({
        getForYouFeed,
        getEngagement: async () => ({}),
        reportPlayback: async () => ({}),
      }),
    } as never);
    await render(
      <SafeAreaProvider
        initialMetrics={{
          frame: { x: 0, y: 0, width: 320, height: 640 },
          insets: { top: 0, left: 0, right: 0, bottom: 0 },
        }}
      >
        <QueryClientProvider client={testQueryClient()}>
          <ShortsFeed context={{ kind: "forYou" }} />
        </QueryClientProvider>
      </SafeAreaProvider>,
    );
    const feed = await screen.findByTestId("shorts-feed");
    await fireEvent(feed, "layout", { nativeEvent: { layout: { width: 320, height: 640 } } });
    await waitFor(() => expect(getForYouFeed).toHaveBeenCalledTimes(2));
    expect(await screen.findByTestId("short-slide-1")).toBeOnTheScreen();
    expect(screen.getByTestId("short-repeat")).toBeOnTheScreen();
  });
});
