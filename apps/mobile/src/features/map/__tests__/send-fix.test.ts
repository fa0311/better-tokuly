import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { create, identity } from "@tokuly/api";
import { api, resetApi } from "#/lib/api";
import { leaveSharing, sendFix, useSharing } from "../sharing";

/** The server, at the edge: records each call and who made it. */
const calls: string[] = [];
async function mockServer(input: string, init: { headers: Headers }): Promise<Response> {
  const method = new URL(input).pathname.split("/").pop() ?? "";
  calls.push(`${method} as ${new Headers(init.headers).get("authorization")}`);
  return new Response(new Uint8Array(), {
    status: 200,
    headers: { "content-type": "application/proto" },
  });
}
jest.mock("expo/fetch", () => ({
  fetch: (input: string, init: { headers: Headers }) => mockServer(input, init),
}));

/** The battery is read before each fix is sent: the moment a switch or a stop can slip in. */
let releaseBattery: () => void = () => undefined;
jest.mock("expo-battery", () => ({
  BatteryState: { CHARGING: 2, FULL: 3 },
  getBatteryLevelAsync: () =>
    new Promise<number>((resolve) => {
      releaseBattery = () => resolve(0.5);
    }),
  getBatteryStateAsync: async () => 1,
}));

const signedInAs = (user: string) =>
  create(identity.AuthResponseSchema, {
    accessToken: `access-${user}`,
    refreshToken: `rt-${user}`,
    accessTokenExpiresAt: timestampFromMs(Date.now() + 15 * 60_000),
    sessionId: `s-${user}`,
    me: { user: { id: user, handle: user } },
  });

const FIX = {
  lat: 35.66,
  lng: 139.7,
  accuracyMeters: 10,
  speedMps: 0,
  headingDegrees: null,
  recordedAt: Date.now(),
};

const sharingAs = (userId: string) =>
  useSharing.setState({ userId, loaded: true, choice: "on", leaving: false, stopOwed: false });

/** Lets the pending awaits run. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(async () => {
  calls.length = 0;
  resetApi();
  await api().session.adopt(signedInAs("alice"));
  sharingAs("alice");
});

describe("sendFix", () => {
  it("sends a fix as the account that shares", async () => {
    const sending = sendFix(FIX, "alice");
    releaseBattery();
    await sending;
    expect(calls).toEqual(["ReportLocations as Bearer access-alice"]);
  });

  it("never sends a fix taken before a switch as the next account", async () => {
    const sending = sendFix(FIX, "alice");
    // The switch: a new API for bob, who shares too.
    resetApi();
    await api().session.adopt(signedInAs("bob"));
    sharingAs("bob");
    releaseBattery();
    await sending;
    expect(calls).toEqual([]);
  });

  it("sends nothing for an account after it stopped on leaving", async () => {
    const sending = sendFix(FIX, "alice");
    const leaving = leaveSharing("alice");
    releaseBattery();
    await Promise.all([sending, leaving]);
    await settle();
    expect(calls).toEqual(["StopSharing as Bearer access-alice"]);
    // A fix the watcher hands over afterwards goes nowhere either.
    await sendFix({ ...FIX, recordedAt: FIX.recordedAt + 1000 }, "alice");
    expect(calls).toEqual(["StopSharing as Bearer access-alice"]);
  });
});
