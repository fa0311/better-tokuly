import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { create, notes } from "@tokuly/api";
import { storyKeys } from "@tokuly/core/stories";
import { router } from "expo-router";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { useSessionStore } from "#/features/session/session";
import { api } from "#/lib/api";
import { StoryViewerScreen } from "../viewer-screen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("expo-router", () => ({
  router: { back: jest.fn(), canGoBack: () => true, replace: jest.fn(), push: jest.fn() },
}));
// The video player is the edge: an empty one, as on Android, reports that it ended.
jest.mock("expo-video", () => {
  const { View } = require("react-native");
  return {
    VideoView: () => require("react").createElement(View),
    useVideoPlayer: () => ({
      status: "idle",
      addListener: () => ({ remove: () => {} }),
      replaceAsync: jest.fn(),
      play: jest.fn(),
      pause: jest.fn(),
      duration: 0,
      currentTime: 0,
    }),
  };
});

// Animations and gestures run natively; here they are plain views.
jest.mock("react-native-reanimated", () => {
  const { View } = require("react-native");
  return {
    __esModule: true,
    default: { View },
    useSharedValue: (value: number) => ({ value }),
    useAnimatedStyle: (style: () => object) => style(),
  };
});
jest.mock("react-native-gesture-handler", () => {
  const chain: object = new Proxy({}, { get: () => () => chain });
  return {
    Gesture: new Proxy({}, { get: () => () => chain }),
    GestureDetector: ({ children }: { children: unknown }) => children,
  };
});

const markStoryViewed = jest.fn(async () => ({}));

function setUp() {
  jest.mocked(api).mockReturnValue({ service: () => ({ markStoryViewed }) } as never);
  useSessionStore.setState({
    state: { status: "signedIn", sessionId: "s", me: { user: { id: "me" } } as never },
  });
  const queryClient = new QueryClient();
  queryClient.setQueryData(storyKeys.tray, [
    create(notes.StoryReelSchema, {
      user: { id: "friend", displayName: "Aoi" },
      hasUnseen: true,
      stories: [
        {
          id: "s1",
          title: "maestro day",
          owner: { id: "friend" },
          items: [{ media: { image: { url: "https://media/s1.webp" } } }],
        },
      ],
    }),
  ]);
  return queryClient;
}

describe("StoryViewerScreen", () => {
  it("shows a friend's photo story and marks it watched once the picture is up", async () => {
    const queryClient = setUp();
    await render(
      <SafeAreaProvider
        initialMetrics={{
          frame: { x: 0, y: 0, width: 320, height: 640 },
          insets: { top: 0, left: 0, right: 0, bottom: 0 },
        }}
      >
        <QueryClientProvider client={queryClient}>
          <StoryViewerScreen userId="friend" />
        </QueryClientProvider>
      </SafeAreaProvider>,
    );
    expect(screen.getByTestId("story-viewer")).toBeOnTheScreen();
    expect(screen.getByText("maestro day")).toBeOnTheScreen();
    expect(markStoryViewed).not.toHaveBeenCalled();
    await fireEvent(screen.getByTestId("story-picture"), "load", { nativeEvent: { source: {} } });
    await waitFor(() => expect(markStoryViewed).toHaveBeenCalledWith({ storyId: "s1" }));
    expect(router.back).not.toHaveBeenCalled();
    expect(screen.getByTestId("story-viewer")).toBeOnTheScreen();
  });
});
