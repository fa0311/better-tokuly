import { fromBinary, toBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { create, identity } from "@tokuly/api";
import { api, resetApi, settleApi } from "#/lib/api";
import { signOut, switchAccount } from "../session";
import { useSessionStore } from "../store";
import { activeTokenStore, parkedTokens } from "../vault";

jest.mock("#/generated/seams/lifecycle", () => ({ __esModule: true, default: [] }));

const answer = (user: string, refreshToken: string) =>
  create(identity.AuthResponseSchema, {
    accessToken: `access-${user}`,
    refreshToken,
    accessTokenExpiresAt: timestampFromMs(Date.now() + 15 * 60_000),
    sessionId: `s-${user}`,
    me: { user: { id: user, handle: user } },
  });

/**
 * The server, at the edge. It rotates a refresh token the moment it is
 * asked (`rt-main` is spent from then on); its first answer waits for `release`.
 */
const calls: string[] = [];
let held = false;
let release = () => {};
const proto = (message: Uint8Array) =>
  new Response(message as BodyInit, {
    status: 200,
    headers: { "content-type": "application/proto" },
  });
async function mockServer(input: string, init: { body: Uint8Array; headers?: HeadersInit }) {
  const method = new URL(input).pathname.split("/").pop() ?? "";
  const bearer = new Headers(init.headers).get("authorization")?.replace("Bearer access-", "");
  switch (method) {
    case "SwitchAccount":
      calls.push("SwitchAccount");
      return proto(toBinary(identity.AuthResponseSchema, answer("alt", "rt-alt-2")));
    case "RefreshToken": {
      const { refreshToken } = fromBinary(identity.RefreshTokenRequestSchema, init.body);
      calls.push(`RefreshToken with ${refreshToken}`);
      if (refreshToken === "rt-main" && !held) {
        held = true;
        await new Promise<void>((resolve) => {
          release = resolve;
        });
      }
      const user = refreshToken.replace(/^rt-/, "").replace(/-\w+$/, "");
      return proto(toBinary(identity.AuthResponseSchema, answer(user, `${refreshToken}-r`)));
    }
    default:
      calls.push(`${method} as ${bearer}`);
      return proto(new Uint8Array());
  }
}
jest.mock("expo/fetch", () => ({
  fetch: (input: string, init: { body: Uint8Array; headers?: HeadersInit }) =>
    mockServer(input, init),
}));

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(async () => {
  calls.length = 0;
  held = false;
  resetApi();
  await api().session.adopt(answer("main", "rt-main"));
  useSessionStore.setState({ state: api().session.state, generation: 0 });
  const { writeJson } = jest.requireActual("#/platform/secure-storage");
  await writeJson("tokuly.session.parked", { "s-alt": "rt-alt" });
  await writeJson("tokuly.session.departed", {});
});

describe("a refresh on its way while the account leaves", () => {
  it("lands before the account is parked: the token kept is the one its session ended up with", async () => {
    // `main`'s access token is about to run out: a call refreshed it in passing, and the answer is late.
    const refreshing = api().session.refresh();
    await settle();
    const switching = switchAccount("s-alt");
    await settle();
    release();
    await expect(switching).resolves.toBe("switched");
    await refreshing;

    // Parked with the spent `rt-main`, the next switch to `main` would get its session revoked.
    expect(await parkedTokens()).toEqual({ "s-main": "rt-main-r" });
    expect(await activeTokenStore.get()).toBe("rt-alt-2");
  });

  it("starts no refresh once its token is about to be moved", async () => {
    const main = api();
    await settleApi();
    // Something of `main` needs a fresh access token while its refresh token goes to the parked ones.
    await expect(main.session.refresh()).resolves.toBe(false);

    expect(calls).toEqual([]);
    expect(await activeTokenStore.get()).toBe("rt-main");
    expect(main.session.state.status).toBe("signedIn");
  });

  it("lands before a sign-out sets the token aside: the session is ended with a token that still works", async () => {
    const refreshing = api().session.refresh();
    await settle();
    const signingOut = signOut();
    await settle();
    release();
    await signingOut;
    await refreshing;

    expect(calls).toEqual([
      "RefreshToken with rt-main",
      "SignOut as main",
      "RefreshToken with rt-main-r",
      "SignOut as main",
    ]);
  });
});
