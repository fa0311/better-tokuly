import { toBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { create, createApi, identity, type SessionState } from "@tokuly/api";
import { describe, expect, it } from "vitest";

/**
 * A school opening the app in the minute after the bell: the first refresh
 * is over the address's limit and told when to try again.
 */
function busyServer(refusals: number) {
  const refreshes: number[] = [];
  const fetch: typeof globalThis.fetch = async () => {
    refreshes.push(Date.now());
    if (refreshes.length <= refusals)
      return new Response(JSON.stringify({ code: "resource_exhausted", message: "busy" }), {
        status: 429,
        headers: {
          "content-type": "application/json",
          "x-tokuly-error": "RATE_LIMITED",
          "retry-after": "1",
        },
      });
    const body = create(identity.AuthResponseSchema, {
      accessToken: "token-of-alice",
      accessTokenExpiresAt: timestampFromMs(Date.now() + 15 * 60_000),
      sessionId: "session-of-alice",
      me: { user: { id: "alice", handle: "alice" } },
    });
    return new Response(toBinary(identity.AuthResponseSchema, body), {
      status: 200,
      headers: { "content-type": "application/proto" },
    });
  };
  return { fetch, refreshes };
}

describe("restoring the session while the API is rate-limited", () => {
  it("keeps the session and tries again after Retry-After", async () => {
    const server = busyServer(1);
    const api = createApi({ baseUrl: "http://api.test", client: "web", fetch: server.fetch });
    const states: SessionState["status"][] = [];
    api.session.subscribe((state) => states.push(state.status));

    expect(await api.session.restore()).toBe(true);

    // Never the sign-in screen: the splash stays until the refresh went through.
    expect(states).toEqual(["signedIn"]);
    expect(server.refreshes).toHaveLength(2);
    const [first = 0, second = 0] = server.refreshes;
    expect(second - first).toBeGreaterThanOrEqual(950);
  });
});
