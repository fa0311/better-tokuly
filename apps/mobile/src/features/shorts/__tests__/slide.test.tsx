import { QueryClientProvider } from "@tanstack/react-query";
import { act, render } from "@testing-library/react-native";
import { create, shorts } from "@tokuly/api";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { api } from "#/lib/api";
import { testQueryClient } from "#/testing/query";
import { ShortSlide } from "../slide";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("expo-router", () => ({ router: { push: jest.fn() } }));
// The player is the edge: the test says when it plays.
const mockListeners = new Map<string, (payload: unknown) => void>();
jest.mock("expo-video", () => {
  const { View } = require("react-native");
  return {
    VideoView: () => require("react").createElement(View),
    useVideoPlayer: () => ({
      addListener: (event: string, listener: (payload: unknown) => void) => {
        mockListeners.set(event, listener);
        return { remove: () => {} };
      },
      play: jest.fn(),
      pause: jest.fn(),
      currentTime: 0,
    }),
  };
});

/** The API object of one account, with the playback reports it was given. */
function account() {
  const reports: unknown[] = [];
  const object = {
    service: () => ({
      reportPlayback: async (request: { reports: unknown[] }) => {
        reports.push(...request.reports);
        return {};
      },
      getEngagement: async () => ({}),
    }),
  };
  return { object, reports };
}

const entry = {
  short: create(shorts.ShortSchema, { id: "u1", shortId: "abc", channel: { name: "ch" } }),
};

describe("ShortSlide", () => {
  it("reports a stay as the account that watched it, even when it ends after a switch", async () => {
    let now = 1_000;
    jest.spyOn(Date, "now").mockImplementation(() => now);
    const a = account();
    const b = account();
    jest.mocked(api).mockReturnValue(a.object as never);
    const view = await render(
      <SafeAreaProvider
        initialMetrics={{
          frame: { x: 0, y: 0, width: 320, height: 640 },
          insets: { top: 0, left: 0, right: 0, bottom: 0 },
        }}
      >
        <QueryClientProvider client={testQueryClient()}>
          <ShortSlide
            entry={entry}
            index={0}
            height={640}
            load="playing"
            source={shorts.PlaybackSource.FOR_YOU}
          />
        </QueryClientProvider>
      </SafeAreaProvider>,
    );
    await act(async () => mockListeners.get("playingChange")?.({ isPlaying: true }));
    now = 4_000;
    // Another account takes over; then the slide leaves the screen.
    jest.mocked(api).mockReturnValue(b.object as never);
    await view.unmount();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(b.reports).toEqual([]);
    expect(a.reports).toEqual([
      expect.objectContaining({
        shortId: "u1",
        watchedMs: 3_000n,
        source: shorts.PlaybackSource.FOR_YOU,
      }),
    ]);
  });
});
