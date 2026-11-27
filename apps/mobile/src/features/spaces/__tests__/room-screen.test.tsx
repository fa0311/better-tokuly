import { QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen } from "@testing-library/react-native";
import { create, spaces } from "@tokuly/api";
import { applySpacesEvent, onStage } from "@tokuly/core/spaces";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { currentAccount } from "#/features/session/account";
import { useSessionStore } from "#/features/session/store";
import { api } from "#/lib/api";
import { testQueryClient } from "#/testing/query";
import lifecycle from "../lifecycle";
import { SpaceScreen } from "../room-screen";
import { useAudioSession } from "../session";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("expo-router", () => ({
  router: { push: jest.fn(), back: jest.fn() },
  Stack: { Screen: () => null },
}));
jest.mock("expo-crypto", () => ({ randomUUID: () => "nonce-1" }));
jest.mock("#/lib/realtime", () => ({
  realtime: () => ({
    onFrame: () => () => {},
    subscribe: () => ({ release: () => {} }),
    start: () => {},
  }),
}));
// LiveKit is the device's: a room that connects at once and holds no one else.
const mockRooms: { connect: jest.Mock; disconnect: jest.Mock }[] = [];
jest.mock("livekit-client", () => ({
  RoomEvent: new Proxy({}, { get: (_target, name) => String(name) }),
  DisconnectReason: {},
}));
jest.mock("../livekit", () => ({
  openRoom: async () => {
    const room = {
      on: () => {},
      connect: jest.fn(async () => {}),
      disconnect: jest.fn(async () => {}),
      localParticipant: {
        identity: "me",
        permissions: { canPublish: false },
        isMicrophoneEnabled: false,
        isSpeaking: false,
      },
      remoteParticipants: new Map(),
    };
    mockRooms.push(room);
    return room;
  },
  closeRoom: async (room: { disconnect(): Promise<void> }) => room.disconnect(),
}));

const user = (id: string) => ({ id, handle: id, displayName: id.toUpperCase() });

const ROOM = create(spaces.SpaceSchema, {
  id: "s1",
  title: "テスト前のぐち大会",
  state: spaces.SpaceState.LIVE,
  audience: spaces.SpaceAudience.FRIENDS,
  host: user("host"),
  listenerCount: 4,
  stage: [{ user: user("host"), role: spaces.SpaceRole.HOST }],
  viewer: { inside: false, role: spaces.SpaceRole.LISTENER },
});

/** The API of one account: the room, and every call it was asked to make. */
function account(role = spaces.SpaceRole.LISTENER) {
  const calls: string[] = [];
  const service = {
    getSpacesConfig: async () => ({ config: { audioEnabled: true, hostGraceSeconds: 180 } }),
    getSpace: async () => ({ space: ROOM }),
    listParticipants: async () => ({ participants: [], hiddenCount: 0 }),
    joinSpace: async () => {
      calls.push("joinSpace");
      return {
        space: { ...ROOM, viewer: { inside: true, role } },
        me: { user: user("me"), role },
        credentials: { url: "ws://livekit", token: "token", canPublish: onStage(role) },
      };
    },
    leaveSpace: async () => void calls.push("leaveSpace"),
    setHandRaised: async (request: { raised: boolean }) =>
      void calls.push(`setHandRaised:${request.raised}`),
  };
  return { object: { service: () => service }, calls };
}

function signIn(who: ReturnType<typeof account>, userId: string) {
  jest.mocked(api).mockReturnValue(who.object as never);
  useSessionStore.setState({
    state: { status: "signedIn", sessionId: `session-${userId}`, me: { user: user(userId) } },
  } as never);
}

async function open() {
  const queryClient = testQueryClient();
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 360, height: 720 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={queryClient}>
        <SpaceScreen spaceId="s1" />
      </QueryClientProvider>
    </SafeAreaProvider>,
  );
  return queryClient;
}

describe("SpaceScreen", () => {
  beforeEach(() => {
    mockRooms.length = 0;
    useAudioSession.setState({ active: null, exit: null });
  });

  it("shows the stage outside, enters with the account's API and raises a hand inside", async () => {
    const me = account();
    signIn(me, "me");
    await open();
    expect(await screen.findAllByTestId("stage-person")).toHaveLength(1);
    expect(screen.getByText("4人が中にいるよ。入るとだれが聴いてるか見えるよ")).toBeTruthy();

    await fireEvent.press(screen.getByTestId("space-join"));
    await screen.findByTestId("space-leave");
    expect(me.calls).toEqual(["joinSpace"]);
    expect(mockRooms[0]?.connect).toHaveBeenCalledWith("ws://livekit", "token");

    await fireEvent.press(screen.getByTestId("space-hand"));
    expect(me.calls).toContain("setHandRaised:true");
  });

  it("leaves the room as the account that entered when it leaves the device", async () => {
    const first = account();
    signIn(first, "me");
    await open();
    await fireEvent.press(await screen.findByTestId("space-join"));
    await screen.findByTestId("space-leave");
    const leaving = currentAccount();
    if (!leaving) throw new Error("signed out");

    // A switch: the next account's API is the active one by the time the hook runs.
    const next = account();
    signIn(next, "other");
    await act(async () => {
      await lifecycle.beforeLeave?.(leaving);
    });
    expect(first.calls).toContain("leaveSpace");
    expect(next.calls).not.toContain("leaveSpace");
    expect(mockRooms[0]?.disconnect).toHaveBeenCalled();
    expect(useAudioSession.getState().active).toBeNull();
  });

  it("follows the server when the host hands the room over", async () => {
    const me = account(spaces.SpaceRole.HOST);
    signIn(me, "me");
    const queryClient = await open();
    await fireEvent.press(await screen.findByTestId("space-join"));
    await fireEvent.press(await screen.findByTestId("space-menu"));
    expect(screen.getByText("ルームを終了する")).toBeTruthy();

    // The host named someone else: the realtime event says this person is a co-host now.
    await act(async () => {
      applySpacesEvent(
        queryClient,
        {
          case: "participantUpdated",
          value: create(spaces.SpaceParticipantUpdatedSchema, {
            spaceId: "s1",
            participant: { user: user("me"), role: spaces.SpaceRole.COHOST },
          }),
        },
        "me",
      );
    });
    expect(await screen.findByText("ステージをおりる")).toBeTruthy();
    expect(screen.queryByText("ルームを終了する")).toBeNull();
  });
});
