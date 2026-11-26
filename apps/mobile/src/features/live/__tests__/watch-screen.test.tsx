import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen } from "@testing-library/react-native";
import { create, live } from "@tokuly/api";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { api } from "#/lib/api";
import { WatchLiveScreen } from "../watch-screen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("expo-router", () => ({ router: { push: jest.fn(), replace: jest.fn() } }));
jest.mock("#/platform/device", () => ({ deviceId: async () => "install-1" }));
// The player is the device's.
jest.mock("expo-video", () => {
  const { View } = require("react-native");
  return {
    VideoView: () => require("react").createElement(View, { testID: "video-view" }),
    useVideoPlayer: () => ({
      addListener: () => ({ remove: () => {} }),
      replaceAsync: async () => {},
      play: jest.fn(),
      pause: jest.fn(),
      currentOffsetFromLive: null,
    }),
  };
});
// The realtime connection: the test pushes the frames.
const mockFrames: ((frame: unknown) => void)[] = [];
const mockTopics: string[] = [];
jest.mock("#/lib/realtime", () => ({
  realtime: () => ({
    onFrame: (listener: (frame: unknown) => void) => {
      mockFrames.push(listener);
      return () => {};
    },
    subscribe: (topic: string) => {
      mockTopics.push(topic);
      return { release: () => {} };
    },
    start: () => {},
  }),
}));

const STREAM = create(live.StreamSchema, {
  summary: {
    id: "s1",
    shortId: "abc",
    title: "まったり雑談",
    status: live.StreamStatus.LIVE,
    viewerCount: 3n,
    channel: { id: "c1", name: "ミナ" },
  },
  playback: { hlsUrl: "https://media.example/live/abc/index.m3u8" },
  chat: { enabled: true },
  viewer: { canChat: true },
  realtimeTopic: "live:s1",
  reportIntervalSeconds: 15,
});

const line = (id: string, text: string, userId: string) =>
  create(live.ChatMessageSchema, {
    id,
    streamId: "s1",
    kind: live.ChatMessageKind.TEXT,
    text,
    offsetMs: BigInt(id.length),
    author: { user: { id: userId, displayName: userId, handle: userId } },
  });

/** The API of one account: the stream, its chat, the people it blocked, and what it sent. */
function account(name: string) {
  const sent: { name: string; text: string }[] = [];
  const object = {
    service: () => ({
      getStream: async () => ({ stream: STREAM }),
      listChatMessages: async () => ({ messages: [line("m1", "こんばんは", "yui")] }),
      listBlockedUsers: async () => ({ users: [{ id: "troll" }] }),
      reportWatching: async () => ({
        viewerCount: 4n,
        status: live.StreamStatus.LIVE,
        reportIntervalSeconds: 15,
      }),
      getEngagement: async () => ({}),
      getGiftTiers: async () => ({ tiers: [] }),
      sendChatMessage: async (request: { text: string }) => {
        sent.push({ name, text: request.text });
        return { message: line(`own-${sent.length}`, request.text, name) };
      },
    }),
  };
  return { object, sent };
}

const push = (event: live.LiveEvent["event"]) =>
  act(async () => {
    for (const listener of mockFrames)
      listener({ topic: "live:s1", body: { case: "live", value: { event } } });
  });

describe("WatchLiveScreen", () => {
  it("plays the stream and keeps its chat live, without the lines of people the viewer blocked", async () => {
    const a = account("a");
    jest.mocked(api).mockReturnValue(a.object as never);
    await render(
      <SafeAreaProvider
        initialMetrics={{
          frame: { x: 0, y: 0, width: 320, height: 640 },
          insets: { top: 0, left: 0, right: 0, bottom: 0 },
        }}
      >
        <QueryClientProvider client={new QueryClient()}>
          <WatchLiveScreen streamId="abc" />
        </QueryClientProvider>
      </SafeAreaProvider>,
    );
    expect(await screen.findByTestId("live-player")).toBeOnTheScreen();
    expect(await screen.findByText("こんばんは")).toBeOnTheScreen();
    expect(mockTopics).toContain("live:s1");
    // The viewer count follows the heartbeat.
    expect(await screen.findByText("4人が視聴中")).toBeOnTheScreen();

    await push({ case: "chatMessage", value: line("m22", "わこつ！", "rin") });
    await push({ case: "chatMessage", value: line("m333", "あらし", "troll") });
    expect(await screen.findByText("わこつ！")).toBeOnTheScreen();
    expect(screen.queryByText("あらし")).not.toBeOnTheScreen();

    // A line goes out as the account that pressed send, and shows at once.
    await fireEvent.changeText(screen.getByTestId("live-chat-input"), "はじめまして");
    await fireEvent.press(screen.getByTestId("live-chat-send"));
    expect(a.sent).toEqual([{ name: "a", text: "はじめまして" }]);
    expect(await screen.findByText("はじめまして")).toBeOnTheScreen();
  });
});
