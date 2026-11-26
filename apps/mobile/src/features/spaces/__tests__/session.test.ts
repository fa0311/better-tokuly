import { QueryClient } from "@tanstack/react-query";
import { waitFor } from "@testing-library/react-native";
import { create, spaces } from "@tokuly/api";
import { useSessionStore } from "#/features/session/store";
import { api } from "#/lib/api";
import { enterSpace, leaveVenue, toggleMicrophone, useAudioSession } from "../session";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("../microphone", () => ({ microphoneAllowed: async () => true }));
jest.mock("livekit-client", () => ({
  RoomEvent: new Proxy({}, { get: (_target, name) => String(name) }),
  DisconnectReason: { DUPLICATE_IDENTITY: 2, PARTICIPANT_REMOVED: 4, ROOM_DELETED: 5 },
}));

/** A LiveKit room of the device: what it was told, and its events to fire by hand. */
interface MockRoom {
  handlers: Map<string, (...args: unknown[]) => void>;
  localParticipant: {
    identity: string;
    permissions: { canPublish: boolean };
    isMicrophoneEnabled: boolean;
    setMicrophoneEnabled: jest.Mock;
  };
}
const mockRooms: MockRoom[] = [];
/** Whether the next room the server lets in may send the microphone. */
let mockCanPublish = true;
jest.mock("../livekit", () => ({
  openRoom: async () => {
    const room: MockRoom & Record<string, unknown> = {
      handlers: new Map(),
      on(event: string, handler: (...args: unknown[]) => void) {
        room.handlers.set(event, handler);
      },
      connect: async () => {},
      disconnect: async () => {},
      remoteParticipants: new Map(),
      localParticipant: {
        identity: "me",
        permissions: { canPublish: mockCanPublish },
        isMicrophoneEnabled: false,
        setMicrophoneEnabled: jest.fn(async (on: boolean) => {
          room.localParticipant.isMicrophoneEnabled = on;
        }),
      },
    };
    mockRooms.push(room);
    return room;
  },
  closeRoom: async () => {},
}));

const ROOM = create(spaces.SpaceSchema, {
  id: "s1",
  title: "テスト前のぐち大会",
  state: spaces.SpaceState.LIVE,
  viewer: { inside: true, role: spaces.SpaceRole.SPEAKER },
});

let role = spaces.SpaceRole.SPEAKER;
const service = {
  joinSpace: async () => ({
    space: { ...ROOM, viewer: { inside: true, role } },
    me: { user: { id: "me" }, role },
    credentials: { url: "ws://livekit", token: "token", canPublish: mockCanPublish },
  }),
  getSpace: async () => ({ space: { ...ROOM, viewer: { inside: true, role } } }),
  leaveSpace: async () => {},
};

describe("the audio session", () => {
  beforeEach(() => {
    jest.mocked(api).mockReturnValue({ service: () => service } as never);
    useSessionStore.setState({
      state: { status: "signedIn", sessionId: "session-me", me: { user: { id: "me" } } },
    } as never);
  });

  it("does not open the microphone again on a connection made after being sent off stage", async () => {
    const queryClient = new QueryClient();
    expect(await enterSpace(queryClient, ROOM)).toBe(true);
    await toggleMicrophone(queryClient);
    expect(useAudioSession.getState().active?.micOn).toBe(true);

    // The connection drops; while it is down, a moderator sends the person back to the audience.
    role = spaces.SpaceRole.LISTENER;
    mockCanPublish = false;
    // LiveKit gave up on a network that went away (no reason of the server's).
    mockRooms[0]?.handlers.get("Disconnected")?.(1);
    await waitFor(() => expect(useAudioSession.getState().active?.phase).toBe("connected"));

    const again = mockRooms[1];
    expect(again).toBeDefined();
    expect(again?.localParticipant.setMicrophoneEnabled).not.toHaveBeenCalledWith(true);
    expect(useAudioSession.getState().active).toMatchObject({ micOn: false, phase: "connected" });
    await leaveVenue(queryClient);
  });
});
