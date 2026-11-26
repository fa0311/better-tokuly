import { fromBinary, toBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { create, identity } from "@tokuly/api";
import { api, resetApi } from "#/lib/api";
import type { Account } from "../account";
import { switchAccount } from "../session";
import { useSessionStore } from "../store";
import { activeTokenStore, parkedTokens } from "../vault";

/** The server, at the edge: answers SwitchAccount as the account the refresh token belongs to. */
const calls: string[] = [];
let switchAnswer: identity.AuthResponse;
/** The server refuses the switch (a wrong code), as Connect errors look on the wire. */
let switchFails = false;
async function mockServer(input: string, init: { body: Uint8Array }): Promise<Response> {
  const method = new URL(input).pathname.split("/").pop() ?? "";
  calls.push(method);
  if (method !== "SwitchAccount") throw new Error(`unexpected ${method}`);
  const request = fromBinary(identity.SwitchAccountRequestSchema, init.body);
  calls.push(`token:${request.refreshToken}`);
  if (switchFails)
    return new Response(JSON.stringify({ code: "unauthenticated", message: "wrong code" }), {
      status: 401,
      headers: { "content-type": "application/json", "x-tokuly-error": "INVALID_TOTP" },
    });
  return new Response(toBinary(identity.AuthResponseSchema, switchAnswer), {
    status: 200,
    headers: { "content-type": "application/proto" },
  });
}
jest.mock("expo/fetch", () => ({
  fetch: (input: string, init: { body: Uint8Array }) => mockServer(input, init),
}));

/** What a feature does before its account leaves (`lifecycle.ts`), as the registry lists it. */
const mockBeforeLeave = jest.fn<Promise<void> | void, [Account]>();
jest.mock("#/generated/seams/lifecycle", () => ({
  __esModule: true,
  default: [{ beforeLeave: (account: Account) => mockBeforeLeave(account) }],
}));

const signedInAs = (user: string, sessionId: string, refreshToken: string) =>
  create(identity.AuthResponseSchema, {
    accessToken: `access-${user}`,
    refreshToken,
    accessTokenExpiresAt: timestampFromMs(Date.now() + 15 * 60_000),
    sessionId,
    me: { user: { id: user, handle: user } },
  });

beforeEach(async () => {
  calls.length = 0;
  switchFails = false;
  mockBeforeLeave.mockReset();
  // `main` is signed in; `alt` is parked on this device.
  resetApi();
  await api().session.adopt(signedInAs("main", "s-main", "rt-main"));
  useSessionStore.setState({ state: api().session.state, generation: 0 });
  const { writeJson } = jest.requireActual("#/platform/secure-storage");
  await writeJson("tokuly.session.parked", { "s-alt": "rt-alt" });
});

describe("switchAccount", () => {
  it("parks the account in use and starts over as the parked one", async () => {
    switchAnswer = signedInAs("alt", "s-alt", "rt-alt-rotated");
    const left: string[] = [];
    const main = api();
    mockBeforeLeave.mockImplementation((account) => {
      const itsOwn = account.api === main && api() === main;
      left.push(`${account.userId} (${account.sessionId}) left while ${calls.length} calls`);
      left.push(itsOwn ? "its session still the one in use" : "another session in use");
    });

    await expect(switchAccount("s-alt")).resolves.toBe("switched");

    // The account in use leaves once the switch went through, before the new one starts.
    expect(left).toEqual(["main (s-main) left while 2 calls", "its session still the one in use"]);
    expect(calls).toEqual(["SwitchAccount", "token:rt-alt"]);
    expect(await activeTokenStore.get()).toBe("rt-alt-rotated");
    expect(await parkedTokens()).toEqual({ "s-main": "rt-main" });
    const { state, generation } = useSessionStore.getState();
    expect(state.status === "signedIn" && state.me.user?.id).toBe("alt");
    expect(generation).toBe(1);
  });

  it("keeps the rotated token before the account in use leaves", async () => {
    switchAnswer = signedInAs("alt", "s-alt", "rt-alt-rotated");
    // What a restart would find if the app died while a leave hook was still running.
    const stored: Record<string, string>[] = [];
    mockBeforeLeave.mockImplementation(async () => void stored.push(await parkedTokens()));

    await expect(switchAccount("s-alt")).resolves.toBe("switched");

    expect(stored).toEqual([{ "s-alt": "rt-alt-rotated" }]);
  });

  it("changes nothing until the second factor is given", async () => {
    switchAnswer = create(identity.AuthResponseSchema, { mfaRequired: true });
    await expect(switchAccount("s-alt")).resolves.toBe("needsCode");

    expect(mockBeforeLeave).not.toHaveBeenCalled();
    expect(await activeTokenStore.get()).toBe("rt-main");
    expect(await parkedTokens()).toEqual({ "s-alt": "rt-alt" });
    expect(useSessionStore.getState().generation).toBe(0);
  });

  it("keeps the account in use as it was when the switch fails", async () => {
    switchFails = true;
    await expect(switchAccount("s-alt", "000000")).rejects.toThrow();

    expect(mockBeforeLeave).not.toHaveBeenCalled();
    expect(await activeTokenStore.get()).toBe("rt-main");
    expect(await parkedTokens()).toEqual({ "s-alt": "rt-alt" });
    expect(useSessionStore.getState().generation).toBe(0);
  });
});
