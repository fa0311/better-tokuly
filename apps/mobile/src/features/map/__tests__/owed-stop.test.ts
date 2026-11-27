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
  sendFix,
  stopSharing,
  turnOn,
  useOwnLocationSharing,
  useSharing,
} from "../sharing";

/**
 * The server, at the edge: the time of each stop it was told, and the
 * running numbers of the stops and fixes; unreachable while `offline`.
 */
const stops: number[] = [];
const numbered: { stop: boolean; sequence: bigint }[] = [];
let offline = false;
async function mockServer(input: string, init: { body: Uint8Array }): Promise<Response> {
  const method = new URL(input).pathname.split("/").pop() ?? "";
  if (offline) throw new TypeError("Network request failed");
  if (method === "StopSharing") {
    const stop = fromBinary(geo.StopSharingRequestSchema, init.body);
    stops.push(toMillis(stop.stoppedAt));
    numbered.push({ stop: true, sequence: stop.sequence });
  } else if (method === "ReportLocations") {
    for (const { sequence } of fromBinary(geo.ReportLocationsRequestSchema, init.body).samples)
      numbered.push({ stop: false, sequence });
  } else {
    throw new Error(`unexpected ${method}`);
  }
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

// No realtime connection: fixes go over the plain RPC.
jest.mock("#/lib/realtime", () => ({ realtime: () => ({ sendLocation: () => false }) }));

const OWED_KEY = "tokuly.map.stopOwed.alice";
const settle = (ms = 20) => new Promise((resolve) => setTimeout(resolve, ms));

/** alice is signed in and shares from here. */
beforeEach(async () => {
  stops.length = 0;
  numbered.length = 0;
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

describe("the device's running count", () => {
  const fixAt = (recordedAt: number) => ({
    lat: 35.66,
    lng: 139.7,
    accuracyMeters: 8,
    speedMps: null,
    headingDegrees: null,
    recordedAt,
  });

  it("numbers a stop above the fixes before it, and the fixes after above the stop, when the clock goes back", async () => {
    const now = Date.now();
    await sendFix(fixAt(now), "alice");
    const clock = jest.spyOn(Date, "now").mockReturnValue(now - 3_600_000);
    try {
      await stopSharing();
      await turnOn();
      await sendFix(fixAt(now - 3_590_000), "alice");
    } finally {
      clock.mockRestore();
    }

    expect(numbered.map(({ stop }) => stop)).toEqual([false, true, false]);
    const [before = 0n, stop = 0n, after = 0n] = numbered.map(({ sequence }) => sequence);
    expect(stop > before && after > stop).toBe(true);
  });
});
