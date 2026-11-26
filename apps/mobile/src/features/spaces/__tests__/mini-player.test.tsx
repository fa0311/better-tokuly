import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen } from "@testing-library/react-native";
import { spaces } from "@tokuly/api";
import { usePathname } from "expo-router";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { useSessionStore } from "#/features/session/store";
import SpacesBackground from "../background";
import { type ActiveVenue, useAudioSession } from "../session";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("expo-router", () => ({
  router: { push: jest.fn(), back: jest.fn() },
  usePathname: jest.fn(),
}));
jest.mock("expo-crypto", () => ({ randomUUID: () => "nonce-1" }));
jest.mock("#/lib/realtime", () => ({
  realtime: () => ({
    onFrame: () => () => {},
    subscribe: () => ({ release: () => {} }),
    start: () => {},
  }),
}));
jest.mock("livekit-client", () => ({
  RoomEvent: new Proxy({}, { get: (_target, name) => String(name) }),
  DisconnectReason: {},
}));
jest.mock("../livekit", () => ({ openRoom: jest.fn(), closeRoom: jest.fn() }));

const HANGOUT: ActiveVenue = {
  kind: "hangout",
  id: "h1",
  title: "放課後",
  emoji: "🛋️",
  phase: "connected",
  role: spaces.SpaceRole.SPEAKER,
  canPublish: true,
  micOn: true,
  revision: 1,
};

async function showAt(path: string) {
  jest.mocked(usePathname).mockReturnValue(path);
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 360, height: 720 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={new QueryClient()}>
        <SpacesBackground />
      </QueryClientProvider>
    </SafeAreaProvider>,
  );
}

describe("the mini player", () => {
  beforeEach(() => {
    useSessionStore.setState({
      state: { status: "signedIn", sessionId: "session-me", me: { user: { id: "me" } } },
    } as never);
  });

  it("shows on every other screen that the microphone is open", async () => {
    useAudioSession.setState({ active: HANGOUT, exit: null });
    await showAt("/map");
    expect(screen.getByTestId("spaces-inside-bar")).toBeTruthy();
    expect(screen.getByText("マイクON")).toBeTruthy();
    expect(screen.getByTestId("spaces-inside-bar-mic")).toBeSelected();
  });

  it("stays out of the way on the venue's own screen and outside", async () => {
    useAudioSession.setState({ active: HANGOUT, exit: null });
    await showAt("/spaces/hangouts/h1");
    expect(screen.queryByTestId("spaces-inside-bar")).toBeNull();

    await act(() => useAudioSession.setState({ active: null, exit: null }));
    await showAt("/map");
    expect(screen.queryByTestId("spaces-inside-bar")).toBeNull();
  });
});
