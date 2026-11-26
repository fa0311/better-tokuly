import { fromBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { renderHook } from "@testing-library/react-native";
import { create, geo, identity, toMillis } from "@tokuly/api";
import { useSessionStore } from "#/features/session/store";
import { api, resetApi } from "#/lib/api";
import { readSecure, writeSecure } from "#/platform/secure-storage";
import {
  choiceKey,
  confirmStop,
  stopSharing,
  turnOn,
  useOwnLocationSharing,
  useSharing,
} from "../sharing";

/** The server, at the edge: the time of each stop it was told; unreachable while `offline`. */
const stops: number[] = [];
let offline = false;
async function mockServer(input: string, init: { body: Uint8Array }): Promise<Response> {
  const method = new URL(input).pathname.split("/").pop() ?? "";
  if (method !== "StopSharing") throw new Error(`unexpected ${method}`);
  if (offline) throw new TypeError("Network request failed");
  stops.push(toMillis(fromBinary(geo.StopSharingRequestSchema, init.body).stoppedAt));
  return new Response(new Uint8Array(), {
    status: 200,
    headers: { "content-type": "application/proto" },
  });
}
jest.mock("expo/fetch", () => ({
  fetch: (input: string, init: { body: Uint8Array }) => mockServer(input, init),
}));

// The device, at the edge: everything allowed; stopping the background task waits when held.
const mockAccess = {
  servicesEnabled: true,
  foreground: "granted",
  background: true,
  approximate: false,
};
let mockHeldStop: Promise<void> | null = null;
jest.mock("#/platform/location", () => ({
  readLocationAccess: jest.fn(async () => mockAccess),
  askForegroundAccess: jest.fn(async () => mockAccess),
  startBackgroundUpdates: jest.fn(async () => undefined),
  stopBackgroundUpdates: jest.fn(async () => mockHeldStop ?? undefined),
  watchForeground: jest.fn(async () => () => undefined),
  currentPosition: jest.fn(async () => null),
  readBattery: jest.fn(async () => ({})),
}));

const OWED_KEY = "tokuly.map.stopOwed.alice";
const settle = (ms = 20) => new Promise((resolve) => setTimeout(resolve, ms));

/** alice is signed in and shares from here. */
beforeEach(async () => {
  stops.length = 0;
  offline = false;
  mockHeldStop = null;
  resetApi();
  await api().session.adopt(
    create(identity.AuthResponseSchema, {
      accessToken: "access-alice",
      refreshToken: "rt-alice",
      accessTokenExpiresAt: timestampFromMs(Date.now() + 15 * 60_000),
      sessionId: "s-alice",
      me: { user: { id: "alice", handle: "alice" } },
    }),
  );
  useSessionStore.setState({ state: api().session.state });
  await writeSecure(choiceKey("alice"), "on");
  await writeSecure(OWED_KEY, null);
  useSharing.setState({
    userId: "alice",
    loaded: true,
    choice: "on",
    access: mockAccess as never,
    stopOwed: false,
    leaving: false,
  });
});

describe("a stop the server has not heard of", () => {
  it("is told again with the time sharing stopped, not the time of the retry", async () => {
    offline = true;
    const before = Date.now();
    await stopSharing();
    const after = Date.now();
    expect(useSharing.getState().stopOwed).toBe(true);

    // Later (the next start, the retry button): what another device shared meanwhile is newer than the stop.
    await settle(50);
    offline = false;
    await confirmStop();

    expect(stops).toHaveLength(1);
    expect(stops[0]).toBeGreaterThanOrEqual(before);
    expect(stops[0]).toBeLessThanOrEqual(after);
    expect(await readSecure(OWED_KEY)).toBeNull();
  });

  it("is void once the account shares from here again", async () => {
    // alice left while sharing (a switch) and the stop never reached the server; now she is back, sharing.
    await writeSecure(OWED_KEY, String(Date.now() - 60_000));
    useSharing.setState({ userId: null, loaded: false, choice: null, access: null });
    await renderHook(() => useOwnLocationSharing());
    await settle();
    expect(useSharing.getState()).toMatchObject({ choice: "on", stopOwed: false });
    expect(await readSecure(OWED_KEY)).toBeNull();

    // Nothing left to tell again: a stop now would take her live pin away.
    await confirmStop();
    expect(stops).toEqual([]);
  });

  it("is not sent when sharing was turned on again before it went out", async () => {
    let releaseStop = () => {};
    mockHeldStop = new Promise<void>((resolve) => {
      releaseStop = resolve;
    });
    const stopping = stopSharing();
    await settle();
    await turnOn();
    releaseStop();
    await stopping;

    expect(stops).toEqual([]);
    expect(useSharing.getState()).toMatchObject({ choice: "on", stopOwed: false });
    expect(await readSecure(OWED_KEY)).toBeNull();
  });
});
