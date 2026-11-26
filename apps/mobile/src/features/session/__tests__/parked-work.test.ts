import { fromBinary, toBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { create, identity } from "@tokuly/api";
import { api, resetApi } from "#/lib/api";
import { asParked } from "../parked";
import { removeParkedAccount, switchAccount } from "../session";
import { useSessionStore } from "../store";

/** Every refresh token the server was sent, by whom: none may be sent twice (that revokes the session). */
const sent: string[] = [];

const answer = (user: string, refreshToken: string) =>
  create(identity.AuthResponseSchema, {
    accessToken: `access-${user}`,
    refreshToken,
    accessTokenExpiresAt: timestampFromMs(Date.now() + 15 * 60_000),
    sessionId: `s-${user}`,
    me: { user: { id: user, handle: user } },
  });

const ok = (body: Uint8Array = new Uint8Array()) =>
  new Response(body as BodyInit, { status: 200, headers: { "content-type": "application/proto" } });

async function mockServer(input: string, init: { body: Uint8Array }): Promise<Response> {
  const method = new URL(input).pathname.split("/").pop() ?? "";
  if (method === "RefreshToken" || method === "SwitchAccount" || method === "RemoveDeviceAccount") {
    const schema =
      method === "SwitchAccount"
        ? identity.SwitchAccountRequestSchema
        : method === "RemoveDeviceAccount"
          ? identity.RemoveDeviceAccountRequestSchema
          : identity.RefreshTokenRequestSchema;
    const { refreshToken } = fromBinary(schema, init.body) as { refreshToken: string };
    sent.push(`${method} ${refreshToken}`);
    // Something to do as each session takes a moment: the work of several overlaps.
    await new Promise((resolve) => setTimeout(resolve, 20));
    const user = refreshToken.replace(/^rt-/, "").replace(/-\d+$/, "");
    return method === "RemoveDeviceAccount"
      ? ok()
      : ok(toBinary(identity.AuthResponseSchema, answer(user, `${refreshToken}-1`)));
  }
  return ok();
}
jest.mock("expo/fetch", () => ({
  fetch: (input: string, init: { body: Uint8Array }) => mockServer(input, init),
}));

beforeEach(async () => {
  sent.length = 0;
  resetApi();
  await api().session.adopt(answer("main", "rt-main"));
  useSessionStore.setState({ state: api().session.state, generation: 0 });
  const { writeJson } = jest.requireActual("#/platform/secure-storage");
  await writeJson("tokuly.session.parked", { "s-x": "rt-x", "s-alt": "rt-alt" });
});

const twice = () =>
  sent.map((line) => line.split(" ")[1]).filter((t, i, all) => all.indexOf(t) !== i);

describe("work as parked sessions", () => {
  it("never sends a parked token twice when a switch comes while other work is under way", async () => {
    const x = asParked("s-x", async () => "x");
    const switching = switchAccount("s-alt");
    const alt = asParked("s-alt", async () => "alt");
    await Promise.all([x, switching, alt]);
    expect(twice()).toEqual([]);
  });

  it("nor when an account is removed meanwhile", async () => {
    const x = asParked("s-x", async () => "x");
    const removing = removeParkedAccount("s-alt");
    const alt = asParked("s-alt", async () => "alt");
    await Promise.all([x, removing, alt]);
    expect(twice()).toEqual([]);
  });
});
