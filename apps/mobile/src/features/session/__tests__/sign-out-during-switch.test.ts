import { fromBinary, toBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { create, identity } from "@tokuly/api";
import * as SecureStore from "expo-secure-store";
import { api, resetApi } from "#/lib/api";
import { signOut, switchAccount } from "../session";
import { useSessionStore } from "../store";
import { activeTokenStore, departedTokens, parkedTokens } from "../vault";

jest.mock("#/generated/seams/lifecycle", () => ({ __esModule: true, default: [] }));

const answer = (user: string, refreshToken: string) =>
  create(identity.AuthResponseSchema, {
    accessToken: `access-${user}`,
    refreshToken,
    accessTokenExpiresAt: timestampFromMs(Date.now() + 15 * 60_000),
    sessionId: `s-${user}`,
    me: { user: { id: user, handle: user } },
  });

/** The server, at the edge: what it was asked, as whom. The switch's answer waits for `release`. */
const calls: string[] = [];
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
    case "SwitchAccount": {
      const { refreshToken } = fromBinary(identity.SwitchAccountRequestSchema, init.body);
      calls.push(`SwitchAccount with ${refreshToken}`);
      await new Promise<void>((resolve) => {
        release = resolve;
      });
      return proto(toBinary(identity.AuthResponseSchema, answer("alt", "rt-alt-2")));
    }
    case "RefreshToken": {
      const { refreshToken } = fromBinary(identity.RefreshTokenRequestSchema, init.body);
      calls.push(`RefreshToken with ${refreshToken}`);
      const user = refreshToken.replace(/^rt-/, "").replace(/-\d+$/, "");
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
  resetApi();
  await api().session.adopt(answer("main", "rt-main"));
  useSessionStore.setState({ state: api().session.state, generation: 0 });
  const { writeJson } = jest.requireActual("#/platform/secure-storage");
  await writeJson("tokuly.session.parked", { "s-alt": "rt-alt" });
  await writeJson("tokuly.session.departed", {});
});

describe("signing out while a switch is on its way", () => {
  it("waits for the switch, then ends the account it was asked for, never the new one", async () => {
    const switching = switchAccount("s-alt");
    await settle();
    // `main` taps 「ログアウト」 while the switch to `alt` is still waiting for the server.
    const signingOut = signOut();
    await settle();
    release();
    await expect(switching).resolves.toBe("switched");
    await signingOut;

    // `alt` is the account in use and stays signed in; `main` left this device and was ended as itself.
    const { state } = useSessionStore.getState();
    expect(state.status === "signedIn" && state.sessionId).toBe("s-alt");
    expect(await activeTokenStore.get()).toBe("rt-alt-2");
    expect(await parkedTokens()).toEqual({});
    expect(await departedTokens()).toEqual({});
    expect(calls).toEqual([
      "SwitchAccount with rt-alt",
      "RefreshToken with rt-main",
      "SignOut as main",
    ]);
  });

  it("ends nobody when it was asked while the new account was being adopted", async () => {
    // Keeping `alt`'s token waits: between the two accounts, nobody is signed in yet.
    const write = jest.mocked(SecureStore.setItemAsync);
    const real = write.getMockImplementation();
    let kept = () => {};
    write.mockImplementation(async (key, value, options) => {
      if (key === "tokuly.session.active" && value === "rt-alt-2")
        await new Promise<void>((resolve) => {
          kept = resolve;
        });
      return real?.(key, value, options);
    });
    const switching = switchAccount("s-alt");
    await settle();
    release();
    while (useSessionStore.getState().state.status !== "unknown") await settle();
    const signingOut = signOut();
    await settle();
    kept();
    await expect(switching).resolves.toBe("switched");
    await signingOut;
    write.mockImplementation(real);

    // `main` was parked by then and `alt` is the account in use: both stay.
    const { state } = useSessionStore.getState();
    expect(state.status === "signedIn" && state.sessionId).toBe("s-alt");
    expect(await activeTokenStore.get()).toBe("rt-alt-2");
    expect(await parkedTokens()).toEqual({ "s-main": "rt-main" });
    expect(await departedTokens()).toEqual({});
    expect(calls).toEqual(["SwitchAccount with rt-alt"]);
  });
});
