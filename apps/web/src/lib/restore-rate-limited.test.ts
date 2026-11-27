import { toBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { create, createApi, identity, type SessionState } from "@tokuly/api";
import { afterEach, describe, expect, it, vi } from "vitest";

/**
 * A school opening the app in the minute after the bell: the first refresh
 * is over the address's limit and told when to try again.
 */
function busyServer(refusals: number, retryAfter = "1") {
  const refreshes: number[] = [];
  const busy = () =>
    new Response(JSON.stringify({ code: "resource_exhausted", message: "busy" }), {
      status: 429,
      headers: {
        "content-type": "application/json",
        "x-tokuly-error": "RATE_LIMITED",
        "retry-after": retryAfter,
      },
    });
  const fetch: typeof globalThis.fetch = async (input) => {
    // Signing out is over the limit as well: the server still holds the session.
    if (!String(input).endsWith("/RefreshToken")) return busy();
    refreshes.push(Date.now());
    if (refreshes.length <= refusals) return busy();
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

function restoring(server: ReturnType<typeof busyServer>) {
  const api = createApi({ baseUrl: "http://api.test", client: "web", fetch: server.fetch });
  const states: SessionState["status"][] = [];
  api.session.subscribe((state) => states.push(state.status));
  return { session: api.session, states };
}

afterEach(() => {
  vi.useRealTimers();
});

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

  it("waits once, however many ask for the session meanwhile", async () => {
    vi.useFakeTimers();
    const server = busyServer(1, "10");
    const { session } = restoring(server);

    const first = session.restore();
    await vi.advanceTimersByTimeAsync(3_000);
    const second = session.restore();
    await vi.advanceTimersByTimeAsync(3_000);

    // The second asker adds nothing to the count while the wait is on.
    expect(server.refreshes).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(4_000);
    expect(await Promise.all([first, second])).toEqual([true, true]);
    expect(server.refreshes).toHaveLength(2);
  });

  it("does not ask again at once when the server names no wait at all", async () => {
    vi.useFakeTimers();
    const server = busyServer(Number.POSITIVE_INFINITY, "0");
    const { session } = restoring(server);

    void session.restore();
    await vi.advanceTimersByTimeAsync(900);

    expect(server.refreshes).toHaveLength(1);
  });

  it("gives up after two minutes: the sign-in screen instead of the splash for good", async () => {
    vi.useFakeTimers();
    const server = busyServer(Number.POSITIVE_INFINITY, "30");
    const { session, states } = restoring(server);

    const restored = session.restore();
    await vi.advanceTimersByTimeAsync(119_000);
    expect(states).toEqual([]);
    await vi.advanceTimersByTimeAsync(2_000);

    expect(await restored).toBe(false);
    expect(states).toEqual(["signedOut"]);
    await vi.advanceTimersByTimeAsync(120_000);
    expect(server.refreshes).toHaveLength(5);
  });
});
