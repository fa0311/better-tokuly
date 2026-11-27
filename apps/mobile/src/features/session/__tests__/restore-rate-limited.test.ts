import { fromBinary, toBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { create, identity } from "@tokuly/api";
import { readSecure, writeSecure } from "#/platform/secure-storage";
import { startSession } from "../session";
import { useSessionStore } from "../store";

jest.mock("#/generated/seams/lifecycle", () => ({ __esModule: true, default: [] }));

/** The app's own storage, at the edge: this install has run before. */
jest.mock("expo-file-system", () => ({
  Paths: { document: "documents" },
  File: class {
    get exists() {
      return true;
    }
    create() {}
  },
}));

/**
 * The server, at the edge: a school opening the app in the minute after the
 * bell. The first refresh is over the address's limit and told when to try again.
 */
const refreshes: { token: string; at: number }[] = [];
async function mockServer(input: string, init: { body: Uint8Array }) {
  const method = new URL(input).pathname.split("/").pop() ?? "";
  if (method !== "RefreshToken") return new Response(null, { status: 404 });
  const { refreshToken } = fromBinary(identity.RefreshTokenRequestSchema, init.body);
  refreshes.push({ token: refreshToken, at: Date.now() });
  if (refreshes.length === 1)
    return new Response(JSON.stringify({ code: "resource_exhausted", message: "busy" }), {
      status: 429,
      headers: {
        "content-type": "application/json",
        "x-tokuly-error": "RATE_LIMITED",
        "retry-after": "1",
      },
    });
  const answer = create(identity.AuthResponseSchema, {
    accessToken: "access-me",
    refreshToken: `${refreshToken}-2`,
    accessTokenExpiresAt: timestampFromMs(Date.now() + 15 * 60_000),
    sessionId: "s-me",
    me: { user: { id: "me", handle: "me" } },
  });
  return new Response(toBinary(identity.AuthResponseSchema, answer) as BodyInit, {
    status: 200,
    headers: { "content-type": "application/proto" },
  });
}
jest.mock("expo/fetch", () => ({
  fetch: (input: string, init: { body: Uint8Array }) => mockServer(input, init),
}));

describe("starting the app while the API is rate-limited", () => {
  it("keeps the session and tries again after Retry-After", async () => {
    await writeSecure("tokuly.session.active", "rt-me");
    const states: string[] = [];
    useSessionStore.subscribe(({ state }) => states.push(state.status));

    await startSession();

    // Never the sign-in screen: the splash stays until the refresh went through.
    expect(states).not.toContain("signedOut");
    expect(useSessionStore.getState().state.status).toBe("signedIn");
    expect(refreshes.map((r) => r.token)).toEqual(["rt-me", "rt-me"]);
    const [first, second] = refreshes;
    expect((second?.at ?? 0) - (first?.at ?? 0)).toBeGreaterThanOrEqual(950);
    expect(await readSecure("tokuly.session.active")).toBe("rt-me-2");
  });
});
