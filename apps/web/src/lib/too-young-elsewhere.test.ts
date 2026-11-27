import { toBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { create, createApi, identity } from "@tokuly/api";
import { describe, expect, it } from "vitest";

/**
 * An account that answered under the minimum age on another device: this
 * one still holds the `Me` from before, and every call is refused with
 * ACCOUNT_TOO_YOUNG until the next refresh brings the one that says so.
 */
function fakeServer() {
  let tooYoung = false;
  let refreshes = 0;
  const fetch: typeof globalThis.fetch = async (input) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    if (new URL(url).pathname.endsWith(".AuthService/RefreshToken")) {
      refreshes += 1;
      const body = create(identity.AuthResponseSchema, {
        accessToken: `token-${refreshes}`,
        accessTokenExpiresAt: timestampFromMs(Date.now() + 15 * 60_000),
        sessionId: "s1",
        me: { user: { id: "u1", handle: "kid" }, birthdate: "2015-01-01", tooYoung },
      });
      return new Response(toBinary(identity.AuthResponseSchema, body), {
        status: 200,
        headers: { "content-type": "application/proto" },
      });
    }
    return new Response(JSON.stringify({ code: "permission_denied", message: "too young" }), {
      status: 403,
      headers: { "content-type": "application/json", "x-tokuly-error": "ACCOUNT_TOO_YOUNG" },
    });
  };
  return {
    fetch,
    answerUnderTheMinimumAge: () => {
      tooYoung = true;
    },
    refreshes: () => refreshes,
  };
}

const someone = { by: { case: "handle" as const, value: "someone" } };

describe("an account that became too young on another device", () => {
  it("learns so from the first refused call, and asks once", async () => {
    const server = fakeServer();
    const api = createApi({ baseUrl: "http://api.test", client: "web", fetch: server.fetch });
    await api.session.restore();
    server.answerUnderTheMinimumAge();

    const profiles = api.service(identity.ProfileService);
    await Promise.all([1, 2, 3].map(() => expect(profiles.getProfile(someone)).rejects.toThrow()));
    await expect
      .poll(() => api.session.state.status === "signedIn" && api.session.state.me.tooYoung)
      .toBe(true);
    expect(server.refreshes()).toBe(2);

    // Held as too young now: further refusals ask nothing more.
    await expect(profiles.getProfile(someone)).rejects.toThrow();
    expect(server.refreshes()).toBe(2);
  });

  it("asks at most once a minute while the server says otherwise", async () => {
    const server = fakeServer();
    const api = createApi({ baseUrl: "http://api.test", client: "web", fetch: server.fetch });
    await api.session.restore();

    const profiles = api.service(identity.ProfileService);
    for (const _ of [1, 2, 3]) await expect(profiles.getProfile(someone)).rejects.toThrow();
    await expect.poll(() => server.refreshes()).toBe(2);
    expect(api.session.state.status === "signedIn" && api.session.state.me.tooYoung).toBe(false);
  });
});
