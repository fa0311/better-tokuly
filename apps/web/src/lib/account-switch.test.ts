import { toBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { create, createApi, identity } from "@tokuly/api";
import { describe, expect, it } from "vitest";

/**
 * A tab whose browser switched accounts in another tab: its next refresh
 * comes back as the other account. Nothing may then be sent, or sent again,
 * with that account's token.
 */
function fakeServer(refreshAs: () => string) {
  const sent: { path: string; authorization: string | null }[] = [];
  let calls = 0;
  const fetch: typeof globalThis.fetch = async (input, init) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const path = new URL(url).pathname;
    const headers = new Headers(init?.headers);
    if (path.endsWith(".AuthService/RefreshToken")) {
      const user = refreshAs();
      const body = create(identity.AuthResponseSchema, {
        accessToken: `token-of-${user}`,
        accessTokenExpiresAt: timestampFromMs(Date.now() + 15 * 60_000),
        sessionId: `session-of-${user}`,
        me: { user: { id: user, handle: user } },
      });
      return new Response(toBinary(identity.AuthResponseSchema, body), {
        status: 200,
        headers: { "content-type": "application/proto" },
      });
    }
    sent.push({ path, authorization: headers.get("authorization") });
    calls += 1;
    // The first call meets an expired token; whatever comes after succeeds.
    if (calls === 1)
      return new Response(JSON.stringify({ code: "unauthenticated", message: "expired" }), {
        status: 401,
        headers: { "content-type": "application/json", "x-tokuly-error": "TOKEN_EXPIRED" },
      });
    return new Response(
      toBinary(identity.GetMeResponseSchema, create(identity.GetMeResponseSchema)),
      {
        status: 200,
        headers: { "content-type": "application/proto" },
      },
    );
  };
  return { fetch, sent };
}

describe("a tab whose browser switched to another account", () => {
  it("does not resend a call with the other account's token after a 401", async () => {
    let user = "alice";
    const server = fakeServer(() => user);
    const api = createApi({ baseUrl: "http://api.test", client: "web", fetch: server.fetch });
    const changes: string[] = [];
    api.session.onAccountChange(({ userId }) => changes.push(userId));
    expect(await api.session.restore()).toBe(true);

    // Another tab switches to bob; this tab's token expires and its call is refused.
    user = "bob";
    const account = api.service(identity.AccountService);
    await expect(account.getMe({})).rejects.toThrow();
    expect(server.sent.map((s) => s.authorization)).toEqual(["Bearer token-of-alice"]);
    expect(changes).toEqual(["bob"]);
    expect(api.session.isStale).toBe(true);
    expect(api.session.peekAccessToken()).toBeNull();

    // Nothing more goes out at all.
    await expect(account.getMe({})).rejects.toThrow();
    expect(server.sent).toHaveLength(1);
    expect(await api.session.refresh()).toBe(false);
  });

  it("does not send the first call after expiry as the other account", async () => {
    let user = "alice";
    const server = fakeServer(() => user);
    const api = createApi({ baseUrl: "http://api.test", client: "web", fetch: server.fetch });
    await api.session.restore();
    // The access token has run out: the next call refreshes first.
    (api.session as unknown as { expiresAtMs: number }).expiresAtMs = 0;
    user = "bob";
    await expect(api.service(identity.AccountService).getMe({})).rejects.toThrow();
    expect(server.sent).toHaveLength(0);
  });

  it("keeps working while the account stays the same", async () => {
    const server = fakeServer(() => "alice");
    const api = createApi({ baseUrl: "http://api.test", client: "web", fetch: server.fetch });
    await api.session.restore();
    await api.service(identity.AccountService).getMe({});
    expect(server.sent.map((s) => s.authorization)).toEqual([
      "Bearer token-of-alice",
      "Bearer token-of-alice",
    ]);
    expect(api.session.isStale).toBe(false);
  });
});
