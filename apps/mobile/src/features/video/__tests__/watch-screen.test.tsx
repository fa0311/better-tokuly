import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { create, video } from "@tokuly/api";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { api } from "#/lib/api";
import { WatchScreen } from "../watch-screen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("expo-router", () => ({ router: { push: jest.fn(), replace: jest.fn() } }));
jest.mock("#/features/web/handoff", () => ({
  webScreenHref: (path: string) => `/web?path=${path}`,
}));

// The player is the edge: the test says when it plays, and reads where it was put.
const mockListeners = new Map<string, Set<(payload: unknown) => void>>();
/** Tells every listener of the player's event, as the native player would. */
const emit = (event: string, payload?: unknown) =>
  act(async () => {
    for (const listener of mockListeners.get(event) ?? []) listener(payload);
  });
const mockPlayer = {
  currentTime: 0,
  playing: false,
  timeUpdateEventInterval: 0,
  staysActiveInBackground: false,
  showNowPlayingNotification: false,
  addListener: (event: string, listener: (payload: unknown) => void) => {
    const listeners = mockListeners.get(event) ?? new Set();
    listeners.add(listener);
    mockListeners.set(event, listeners);
    return { remove: () => listeners.delete(listener) };
  },
  play: jest.fn(),
  pause: jest.fn(),
};
jest.mock("expo-video", () => {
  const React = require("react");
  const { View } = require("react-native");
  return {
    VideoView: () => require("react").createElement(View),
    useVideoPlayer: (_source: unknown, setup?: (player: typeof mockPlayer) => void) => {
      const ready = React.useRef(false);
      if (!ready.current) {
        ready.current = true;
        setup?.(mockPlayer);
      }
      return mockPlayer;
    },
  };
});

const item = create(video.VideoSchema, {
  summary: {
    id: "v1",
    shortId: "abc123",
    title: "夏祭りに行ってきた！",
    durationSeconds: 600,
    resumePositionSeconds: 120,
    likeCount: 4n,
    status: video.VideoStatus.PUBLISHED,
    visibility: 1,
    channel: { id: "c1", handle: "natsu", name: "なつチャンネル" },
  },
  playback: { hlsUrl: "/media/v1/master.m3u8" },
  chapters: [
    { startSeconds: 0, title: "はじまり" },
    { startSeconds: 300, title: "花火" },
  ],
  commentsEnabled: true,
  viewer: { rating: video.Rating.NONE },
});

/** The API object of one account, with what it was asked. */
function account() {
  const calls = { start: [] as unknown[], reports: [] as unknown[], rate: [] as unknown[] };
  const client = {
    getVideo: async () => ({ video: item }),
    listRelatedVideos: async () => ({ videos: [] }),
    listPlaylistItems: async () => ({ items: [] }),
    getPlaylist: async () => ({}),
    startPlayback: async (request: unknown) => {
      calls.start.push(request);
      return { sessionId: "s1", reportIntervalSeconds: 10, viewCount: 1n };
    },
    reportPlayback: async (request: unknown) => {
      calls.reports.push(request);
      return { viewCount: 1n };
    },
    rateVideo: async (request: { rating: video.Rating }) => {
      calls.rate.push(request);
      return { rating: request.rating, likeCount: 5n };
    },
  };
  return { object: { service: () => client }, calls };
}

async function open(params = {}) {
  const view = await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 360, height: 720 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider
        client={
          new QueryClient({ defaultOptions: { queries: { gcTime: Number.POSITIVE_INFINITY } } })
        }
      >
        <WatchScreen videoId="abc123" params={params} />
      </QueryClientProvider>
    </SafeAreaProvider>,
  );
  await screen.findByTestId("video-title");
  return view;
}

beforeEach(() => {
  mockListeners.clear();
  mockPlayer.currentTime = 0;
  mockPlayer.playing = false;
});

describe("WatchScreen", () => {
  it("starts where the viewer left off, or at the link's time", async () => {
    jest.mocked(api).mockReturnValue(account().object as never);
    const first = await open();
    expect(mockPlayer.currentTime).toBe(120);
    expect(mockPlayer.play).toHaveBeenCalled();
    await first.unmount();

    await open({ t: "5m" });
    expect(mockPlayer.currentTime).toBe(300);
  });

  it("marks the chapter playing and jumps to one on a tap", async () => {
    jest.mocked(api).mockReturnValue(account().object as never);
    await open();
    await emit("timeUpdate", { currentTime: 310 });
    expect(screen.getByTestId("chapter-300").props.accessibilityState).toEqual(
      expect.objectContaining({ selected: true }),
    );
    await act(async () => fireEvent.press(screen.getByTestId("chapter-0")));
    expect(mockPlayer.currentTime).toBe(0);
  });

  it("likes at once and takes the server's count", async () => {
    const a = account();
    jest.mocked(api).mockReturnValue(a.object as never);
    await open();
    await act(async () => fireEvent.press(screen.getByTestId("video-like")));
    expect(a.calls.rate).toEqual([
      expect.objectContaining({ videoId: "v1", rating: video.Rating.LIKE }),
    ]);
    await waitFor(() => expect(screen.getByTestId("video-like-count").props.children).toBe("5"));
  });

  it("reports the viewing as the account that watched, even when the screen closes after a switch", async () => {
    const a = account();
    const b = account();
    jest.mocked(api).mockReturnValue(a.object as never);
    const view = await open();
    mockPlayer.playing = true;
    mockPlayer.currentTime = 150;
    await emit("playingChange", { isPlaying: true });
    expect(a.calls.start).toEqual([
      expect.objectContaining({ videoId: "v1", source: video.TrafficSource.DIRECT }),
    ]);
    // Another account takes over; then the screen closes.
    jest.mocked(api).mockReturnValue(b.object as never);
    await view.unmount();
    expect(b.calls.reports).toEqual([]);
    expect(a.calls.reports).toEqual([
      expect.objectContaining({ sessionId: "s1", positionSeconds: 150 }),
    ]);
  });
});
