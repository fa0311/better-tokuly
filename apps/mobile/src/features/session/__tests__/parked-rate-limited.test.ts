import { fromBinary, toBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { create, identity } from "@tokuly/api";
import { api, resetApi } from "#/lib/api";
import { asParked } from "../parked";
import { switchAccount } from "../session";
import { useSessionStore } from "../store";

jest.mock("#/generated/seams/lifecycle", () => ({ __esModule: true, default: [] }));

const answer = (user: string, refreshToken: string) =>
  create(identity.AuthResponseSchema, {
    accessToken: `access-${user}`,
    refreshToken,
    accessTokenExpiresAt: timestampFromMs(Date.now() + 15 * 60_000),
    sessionId: `s-${user}`,
    me: { user: { id: user, handle: user } },
  });

/** The server, at the edge: refreshes are over the limit for the next minute, a switch goes through. */
const refreshes: string[] = [];
async function mockServer(input: string, init: { body: Uint8Array }): Promise<Response> {
  const method = new URL(input).pathname.split("/").pop() ?? "";
  if (method === "RefreshToken") {
    refreshes.push(fromBinary(identity.RefreshTokenRequestSchema, init.body).refreshToken);
    return new Response(JSON.stringify({ code: "resource_exhausted", message: "busy" }), {
      status: 429,
      headers: {
        "content-type": "application/json",
        "x-tokuly-error": "RATE_LIMITED",
        "retry-after": "60",
      },
    });
  }
  const body =
    method === "SwitchAccount"
      ? toBinary(identity.AuthResponseSchema, answer("alt", "rt-alt-1"))
      : new Uint8Array();
  return new Response(body as BodyInit, {
    status: 200,
    headers: { "content-type": "application/proto" },
  });
}
jest.mock("expo/fetch", () => ({
  fetch: (input: string, init: { body: Uint8Array }) => mockServer(input, init),
}));

beforeEach(async () => {
  resetApi();
  await api().session.adopt(answer("main", "rt-main"));
  useSessionStore.setState({ state: api().session.state, generation: 0 });
  const { writeJson } = jest.requireActual("#/platform/secure-storage");
  await writeJson("tokuly.session.parked", { "s-x": "rt-x", "s-alt": "rt-alt" });
});

describe("work as a parked session while the API is rate-limited", () => {
  it("is put off without holding up a switch behind it", async () => {
    const owed = asParked("s-x", async () => "done");
    const switching = switchAccount("s-alt");

    // Not a minute's wait in the queue of the device's tokens: the switch goes through now.
    await expect(owed).rejects.toThrow("could not be resumed");
    await expect(switching).resolves.toBe("switched");
    expect(refreshes).toEqual(["rt-x"]);
    const { readJson } = jest.requireActual("#/platform/secure-storage");
    expect((await readJson("tokuly.session.parked"))["s-x"]).toBe("rt-x");
  });
});
