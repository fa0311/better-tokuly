import { fromBinary, toBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { create, identity } from "@tokuly/api";
import { api, resetApi } from "#/lib/api";
import { enterAccount, signOut } from "../session";
import { useSessionStore } from "../store";

/** A feature that holds something of the account in memory. */
const mockForget = jest.fn();
jest.mock("#/generated/seams/lifecycle", () => ({
  __esModule: true,
  default: [{ forget: () => mockForget() }],
}));

const answer = (user: string, refreshToken: string) =>
  create(identity.AuthResponseSchema, {
    accessToken: `access-${user}`,
    refreshToken,
    accessTokenExpiresAt: timestampFromMs(Date.now() + 15 * 60_000),
    sessionId: `s-${user}`,
    me: { user: { id: user, handle: user } },
  });

/** The server, at the edge: a refresh answers as the token's account, anything else is fine. */
const proto = (message: Uint8Array) =>
  new Response(message as BodyInit, {
    status: 200,
    headers: { "content-type": "application/proto" },
  });
async function mockServer(input: string, init: { body: Uint8Array }) {
  if (new URL(input).pathname.split("/").pop() !== "RefreshToken") return proto(new Uint8Array());
  const { refreshToken } = fromBinary(identity.RefreshTokenRequestSchema, init.body);
  return proto(toBinary(identity.AuthResponseSchema, answer("main", `${refreshToken}-r`)));
}
jest.mock("expo/fetch", () => ({
  fetch: (input: string, init: { body: Uint8Array }) => mockServer(input, init),
}));

/** What the screens are shown: every state the store goes through, with its generation. */
function watchStore(): string[] {
  const seen: string[] = [];
  useSessionStore.subscribe(({ state, generation }) => {
    const who = state.status === "signedIn" ? ` as ${state.me.user?.id}` : "";
    seen.push(`${state.status}${who} (${generation})`);
  });
  return seen;
}

async function signedInAs(user: string): Promise<void> {
  resetApi();
  await api().session.adopt(answer(user, `rt-${user}`));
  useSessionStore.setState({ state: api().session.state, generation: 0 });
}

beforeEach(async () => {
  const { writeJson } = jest.requireActual("#/platform/secure-storage");
  await writeJson("tokuly.session.parked", {});
  await writeJson("tokuly.session.departed", {});
});

describe("starting over as another account", () => {
  it("shows the signed-in screens once, as the new account, after a sign-in", async () => {
    resetApi();
    api().session.assumeSignedOut();
    useSessionStore.setState({ state: api().session.state, generation: 0 });
    const seen = watchStore();

    await enterAccount(answer("main", "rt-main"));

    // The sign-in screen stays until the account is in; no blank screen, no tree of generation 0.
    expect(seen).toEqual(["signedIn as main (1)"]);
  });

  it("takes the account in use off the screen at once, then shows the next one once", async () => {
    await signedInAs("main");
    const seen = watchStore();

    await enterAccount(answer("alt", "rt-alt"));

    // While `alt` is adopted, `api()` is already its API: nothing shows `main` with it.
    expect(seen).toEqual(["unknown (0)", "signedIn as alt (1)"]);
  });

  it("has the features forget the account that left before the next one shows", async () => {
    await signedInAs("main");
    mockForget.mockClear();
    let shownAfterForget = false;
    useSessionStore.subscribe(({ state }) => {
      if (state.status === "signedIn") shownAfterForget = mockForget.mock.calls.length === 1;
    });

    await enterAccount(answer("alt", "rt-alt"));
    expect(shownAfterForget).toBe(true);
    await signOut();
    expect(mockForget).toHaveBeenCalledTimes(2);
  });

  it("signs out in one update", async () => {
    await signedInAs("main");
    const seen = watchStore();

    await signOut();

    expect(seen).toEqual(["signedOut (1)"]);
  });
});
