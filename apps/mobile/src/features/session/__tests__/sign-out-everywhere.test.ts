import { fromBinary, toBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { create, identity } from "@tokuly/api";
import { api, resetApi } from "#/lib/api";
import { settleDeparted, signOut, useSessionStore } from "../session";

/** The server, at the edge: what it was asked, as whom. */
const calls: string[] = [];
let offline = false;
const ended = new Set<string>();

const answer = (user: string, refreshToken: string) =>
  create(identity.AuthResponseSchema, {
    accessToken: `access-${user}`,
    refreshToken,
    accessTokenExpiresAt: timestampFromMs(Date.now() + 15 * 60_000),
    sessionId: `s-${user}`,
    me: { user: { id: user, handle: user } },
  });

const reply = (status: number, body: Uint8Array | string, type: string) =>
  new Response(body as BodyInit, { status, headers: { "content-type": type } });

async function mockServer(input: string, init: { body: Uint8Array; headers?: HeadersInit }) {
  if (offline) throw new TypeError("Network request failed");
  const method = new URL(input).pathname.split("/").pop() ?? "";
  const bearer = new Headers(init.headers).get("authorization")?.replace("Bearer access-", "");
  switch (method) {
    case "RefreshToken": {
      const { refreshToken } = fromBinary(identity.RefreshTokenRequestSchema, init.body);
      const user = refreshToken.replace(/^rt-/, "").replace(/-\d+$/, "");
      if (ended.has(user))
        return reply(
          401,
          JSON.stringify({ code: "unauthenticated", message: "ended" }),
          "application/json",
        );
      return reply(
        200,
        toBinary(identity.AuthResponseSchema, answer(user, `${refreshToken}-1`)),
        "application/proto",
      );
    }
    case "SignOut": {
      const { allSessions } = fromBinary(identity.SignOutRequestSchema, init.body);
      calls.push(`SignOut as ${bearer}${allSessions ? " everywhere" : ""}`);
      if (bearer) ended.add(bearer);
      return reply(200, new Uint8Array(), "application/proto");
    }
    default:
      calls.push(`${method} as ${bearer}`);
      return reply(200, new Uint8Array(), "application/proto");
  }
}
jest.mock("expo/fetch", () => ({
  fetch: (input: string, init: { body: Uint8Array; headers?: HeadersInit }) =>
    mockServer(input, init),
}));

beforeEach(async () => {
  calls.length = 0;
  ended.clear();
  offline = false;
  resetApi();
  await api().session.adopt(answer("main", "rt-main"));
  useSessionStore.setState({ state: api().session.state, generation: 0 });
});

describe("signing out of every session", () => {
  it("offline, is kept as everywhere, said so, and done once the phone is back online", async () => {
    offline = true;
    await expect(signOut(true)).resolves.toEqual({ everywherePending: true });
    expect(useSessionStore.getState().state.status).toBe("signedOut");

    // Next launches: still offline, then online.
    await settleDeparted();
    expect(calls).toEqual([]);
    offline = false;
    await settleDeparted();
    expect(calls).toEqual(["UnregisterDevice as main", "SignOut as main everywhere"]);
    calls.length = 0;
    await settleDeparted();
    expect(calls).toEqual([]);
  });

  it("online, is done at once and nothing is left pending", async () => {
    await expect(signOut(true)).resolves.toEqual({ everywherePending: false });
    expect(calls).toEqual(["SignOut as main everywhere"]);
  });
});
