import { fromBinary, toBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { create, identity } from "@tokuly/api";
import * as SecureStore from "expo-secure-store";
import { api, resetApi } from "#/lib/api";
import { enterAccount, signOut } from "../session";
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

/** The server, at the edge: what it was asked. */
const calls: string[] = [];
const proto = (message: Uint8Array) =>
  new Response(message as BodyInit, {
    status: 200,
    headers: { "content-type": "application/proto" },
  });
async function mockServer(input: string, init: { body: Uint8Array }) {
  const method = new URL(input).pathname.split("/").pop() ?? "";
  if (method !== "RefreshToken") {
    calls.push(method);
    return proto(new Uint8Array());
  }
  const { refreshToken } = fromBinary(identity.RefreshTokenRequestSchema, init.body);
  calls.push(`RefreshToken with ${refreshToken}`);
  return proto(toBinary(identity.AuthResponseSchema, answer("main", `${refreshToken}-r`)));
}
jest.mock("expo/fetch", () => ({
  fetch: (input: string, init: { body: Uint8Array }) => mockServer(input, init),
}));

/** The phone's secure storage refuses the next write of `key` (full, locked, a Keystore error). */
function failNextWrite(key: string): void {
  const write = jest.mocked(SecureStore.setItemAsync);
  const real = write.getMockImplementation();
  write.mockImplementationOnce(async (written, value, options) => {
    if (written === key) throw new Error("storage is full");
    return real?.(written, value, options);
  });
}

beforeEach(async () => {
  calls.length = 0;
  resetApi();
  await api().session.adopt(answer("main", "rt-main"));
  useSessionStore.setState({ state: api().session.state, generation: 0 });
  const { writeJson } = jest.requireActual("#/platform/secure-storage");
  await writeJson("tokuly.session.parked", {});
  await writeJson("tokuly.session.departed", {});
});

describe("the account's token could not be moved", () => {
  it("stays the account in use, and its session refreshes as before", async () => {
    // Adding `alt` beside `main`: parking `main` fails.
    failNextWrite("tokuly.session.parked");
    await expect(enterAccount(answer("alt", "rt-alt"))).rejects.toThrow("storage is full");

    expect(await parkedTokens()).toEqual({});
    expect(await activeTokenStore.get()).toBe("rt-main");
    await expect(api().session.refresh()).resolves.toBe(true);
    expect(calls).toEqual(["RefreshToken with rt-main"]);
    expect(await activeTokenStore.get()).toBe("rt-main-r");
  });

  it("is not left among the parked ones when the active store could not be emptied", async () => {
    // Parking `main` got as far as the copy: the token is in both places when the phone refuses.
    jest.mocked(SecureStore.deleteItemAsync).mockRejectedValueOnce(new Error("storage is locked"));
    await expect(enterAccount(answer("alt", "rt-alt"))).rejects.toThrow("storage is locked");

    // `main` goes on rotating its token: a copy left behind would be a spent one, and a
    // switch to it would get the session revoked.
    expect(await parkedTokens()).toEqual({});
    await expect(api().session.refresh()).resolves.toBe(true);
    expect(await activeTokenStore.get()).toBe("rt-main-r");
  });

  it("stays signed in when its token cannot be set aside for the sign-out", async () => {
    failNextWrite("tokuly.session.departed");
    await expect(signOut()).rejects.toThrow("storage is full");

    const { state } = useSessionStore.getState();
    expect(state.status === "signedIn" && state.sessionId).toBe("s-main");
    await expect(api().session.refresh()).resolves.toBe(true);
    expect(calls).toEqual(["RefreshToken with rt-main"]);
  });
});
