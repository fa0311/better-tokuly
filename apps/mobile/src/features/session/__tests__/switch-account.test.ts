import { fromBinary, toBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { create, identity } from "@tokuly/api";
import { api, resetApi } from "#/lib/api";
import { onBeforeLeave, switchAccount, useSessionStore } from "../session";
import { activeTokenStore, parkedTokens } from "../vault";

/** The server, at the edge: answers SwitchAccount as the account the refresh token belongs to. */
const calls: string[] = [];
let switchAnswer: identity.AuthResponse;
async function mockServer(input: string, init: { body: Uint8Array }): Promise<Response> {
  const method = new URL(input).pathname.split("/").pop() ?? "";
  calls.push(method);
  if (method !== "SwitchAccount") throw new Error(`unexpected ${method}`);
  const request = fromBinary(identity.SwitchAccountRequestSchema, init.body);
  calls.push(`token:${request.refreshToken}`);
  return new Response(toBinary(identity.AuthResponseSchema, switchAnswer), {
    status: 200,
    headers: { "content-type": "application/proto" },
  });
}
jest.mock("expo/fetch", () => ({
  fetch: (input: string, init: { body: Uint8Array }) => mockServer(input, init),
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
    const off = onBeforeLeave(() => void left.push(`left while ${calls.length} calls`));

    await expect(switchAccount("s-alt")).resolves.toBe("switched");
    off();

    // Sharing and the like stop while the old session still works: before the switch is sent.
    expect(left).toEqual(["left while 0 calls"]);
    expect(calls).toEqual(["SwitchAccount", "token:rt-alt"]);
    expect(await activeTokenStore.get()).toBe("rt-alt-rotated");
    expect(await parkedTokens()).toEqual({ "s-main": "rt-main" });
    const { state, generation } = useSessionStore.getState();
    expect(state.status === "signedIn" && state.me.user?.id).toBe("alt");
    expect(generation).toBe(1);
  });

  it("changes nothing until the second factor is given", async () => {
    switchAnswer = create(identity.AuthResponseSchema, { mfaRequired: true });

    await expect(switchAccount("s-alt")).resolves.toBe("needsCode");

    expect(await activeTokenStore.get()).toBe("rt-main");
    expect(await parkedTokens()).toEqual({ "s-alt": "rt-alt" });
    expect(useSessionStore.getState().generation).toBe(0);
  });
});
