import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { create, identity } from "@tokuly/api";
import { currentAccount } from "#/features/session/account";
import { useSessionStore } from "#/features/session/store";
import { api, resetApi } from "#/lib/api";
import { readSecure } from "#/platform/secure-storage";
import lifecycle from "../lifecycle";
import { useSharing } from "../sharing";

/** The server, at the edge: records each call, who made it and whether the stop was owed by then. */
const calls: string[] = [];
const owedWhenSent: (string | null)[] = [];
async function mockServer(input: string, init: { headers: Headers }): Promise<Response> {
  const method = new URL(input).pathname.split("/").pop() ?? "";
  calls.push(`${method} as ${new Headers(init.headers).get("authorization")}`);
  if (method === "StopSharing") owedWhenSent.push(await readSecure("tokuly.map.stopOwed.alice"));
  return new Response(new Uint8Array(), {
    status: 200,
    headers: { "content-type": "application/proto" },
  });
}
jest.mock("expo/fetch", () => ({
  fetch: (input: string, init: { headers: Headers }) => mockServer(input, init),
}));

/** Stopping the background task is the OS's work: it can take longer than a leave waits. */
let mockReleaseStop: () => void = () => undefined;
jest.mock("#/platform/location", () => ({
  stopBackgroundUpdates: () =>
    new Promise<void>((resolve) => {
      mockReleaseStop = resolve;
    }),
}));

const signedInAs = (user: string) =>
  create(identity.AuthResponseSchema, {
    accessToken: `access-${user}`,
    refreshToken: `rt-${user}`,
    accessTokenExpiresAt: timestampFromMs(Date.now() + 15 * 60_000),
    sessionId: `s-${user}`,
    me: { user: { id: user, handle: user } },
  });

async function enter(user: string): Promise<void> {
  resetApi();
  await api().session.adopt(signedInAs(user));
  useSessionStore.setState({ state: api().session.state });
  useSharing.setState({ userId: user, loaded: true, choice: "on", leaving: false });
}

/** Lets the pending awaits run. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("the map's beforeLeave", () => {
  it("tells the server as the account that leaves, even once the next one is in use", async () => {
    await enter("alice");
    const alice = currentAccount();
    if (!alice) throw new Error("alice is signed in");

    const leaving = lifecycle.beforeLeave(alice);
    await settle();
    // The leave gave up waiting for the OS (3 s) and the switch went on: bob is in use, sharing too.
    await enter("bob");
    mockReleaseStop();
    await leaving;

    expect(calls).toEqual(["StopSharing as Bearer access-alice"]);
  });

  it("writes the stop down as owed before it is sent, and clears it once the server heard it", async () => {
    owedWhenSent.length = 0;
    await enter("alice");
    const alice = currentAccount();
    if (!alice) throw new Error("alice is signed in");

    const leaving = lifecycle.beforeLeave(alice);
    await settle();
    mockReleaseStop();
    await leaving;

    // Killed mid-request, the next start would still find it owed and try again.
    expect(owedWhenSent).toEqual(["1"]);
    expect(await readSecure("tokuly.map.stopOwed.alice")).toBeNull();
  });
});
