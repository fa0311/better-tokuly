import { fromBinary, toBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { type Api, create, identity } from "@tokuly/api";
import { api, resetApi } from "#/lib/api";
import { LEAVE_TIMEOUT_MS } from "#/seams/lifecycle";
import type { Account } from "../account";
import { enterAccount, settleDeparted, signOut } from "../session";
import { useSessionStore } from "../store";
import { departedTokens, parkedTokens } from "../vault";

/** Two features' hooks (`lifecycle.ts`), as the registry lists them. */
const mockBeforeLeave = jest.fn<Promise<void> | void, [Account]>();
const mockBeforeEnd = jest.fn<Promise<unknown>, [Api]>();
const mockOtherBeforeLeave = jest.fn<Promise<void> | void, [Account]>();
jest.mock("#/generated/seams/lifecycle", () => ({
  __esModule: true,
  default: [
    {
      beforeLeave: (account: Account) => mockBeforeLeave(account),
      beforeEnd: (session: Api) => mockBeforeEnd(session),
    },
    { beforeLeave: (account: Account) => mockOtherBeforeLeave(account) },
  ],
}));

/** The server, at the edge: what it was asked, as whom. */
const calls: string[] = [];
const ended = new Set<string>();

const answer = (user: string, refreshToken: string) =>
  create(identity.AuthResponseSchema, {
    accessToken: `access-${user}`,
    refreshToken,
    accessTokenExpiresAt: timestampFromMs(Date.now() + 15 * 60_000),
    sessionId: `s-${user}`,
    me: { user: { id: user, handle: user } },
  });

async function mockServer(input: string, init: { body: Uint8Array; headers?: HeadersInit }) {
  const method = new URL(input).pathname.split("/").pop() ?? "";
  const bearer = new Headers(init.headers).get("authorization")?.replace("Bearer access-", "");
  calls.push(`${method} as ${bearer}`);
  let body = new Uint8Array();
  if (method === "SignOut" && bearer) ended.add(bearer);
  if (method === "RefreshToken") {
    const { refreshToken } = fromBinary(identity.RefreshTokenRequestSchema, init.body);
    const user = refreshToken.replace(/^rt-/, "").replace(/-\d+$/, "");
    if (ended.has(user))
      return new Response(JSON.stringify({ code: "unauthenticated", message: "ended" }), {
        status: 401,
        headers: { "content-type": "application/json" },
      });
    body = toBinary(identity.AuthResponseSchema, answer(user, `${refreshToken}-1`));
  }
  return new Response(body as BodyInit, {
    status: 200,
    headers: { "content-type": "application/proto" },
  });
}
jest.mock("expo/fetch", () => ({
  fetch: (input: string, init: { body: Uint8Array; headers?: HeadersInit }) =>
    mockServer(input, init),
}));

beforeEach(async () => {
  calls.length = 0;
  ended.clear();
  mockBeforeLeave.mockReset();
  mockOtherBeforeLeave.mockReset();
  mockBeforeEnd.mockReset().mockResolvedValue(undefined);
  const { writeJson } = jest.requireActual("#/platform/secure-storage");
  await writeJson("tokuly.session.parked", {});
  await writeJson("tokuly.session.departed", {});
  resetApi();
  await api().session.adopt(answer("main", "rt-main"));
  useSessionStore.setState({ state: api().session.state, generation: 0 });
});

describe("a feature's beforeLeave", () => {
  it("runs on a sign-out, as the account that leaves, before the server ends its session", async () => {
    const main = api();
    mockBeforeLeave.mockImplementation((account) => {
      expect(account).toEqual({ sessionId: "s-main", userId: "main", api: main });
      expect(api()).toBe(main);
      calls.push("the feature's goodbye");
    });

    await signOut();

    // The last call is the look at what left the device: the server refuses the ended session.
    expect(calls).toEqual([
      "the feature's goodbye",
      "SignOut as main",
      "RefreshToken as undefined",
    ]);
    expect(mockOtherBeforeLeave).toHaveBeenCalledTimes(1);
    expect(useSessionStore.getState().state.status).toBe("signedOut");
  });

  it("runs when another account is added beside the one in use, before that one is parked", async () => {
    const parkedThen: unknown[] = [];
    mockBeforeLeave.mockImplementation(async () => void parkedThen.push(await parkedTokens()));

    await enterAccount(answer("alt", "rt-alt"));

    expect(parkedThen).toEqual([{}]);
    expect(await parkedTokens()).toEqual({ "s-main": "rt-main" });
    const { state } = useSessionStore.getState();
    expect(state.status === "signedIn" && state.me.user?.id).toBe("alt");
  });

  it("does not run when nobody is signed in", async () => {
    await signOut();
    mockBeforeLeave.mockClear();

    await enterAccount(answer("alt", "rt-alt"));

    expect(mockBeforeLeave).not.toHaveBeenCalled();
  });

  it("is left behind when it fails or hangs: the account still leaves", async () => {
    jest.useFakeTimers();
    try {
      mockBeforeLeave.mockImplementation(() => new Promise(() => {}));
      mockOtherBeforeLeave.mockImplementation(() => {
        throw new Error("the feature broke");
      });

      const leaving = signOut();
      await jest.advanceTimersByTimeAsync(LEAVE_TIMEOUT_MS - 1);
      expect(useSessionStore.getState().state.status).toBe("signedIn");
      await jest.advanceTimersByTimeAsync(1);
      await leaving;

      expect(useSessionStore.getState().state.status).toBe("signedOut");
    } finally {
      jest.useRealTimers();
    }
  });
});

describe("a feature's beforeEnd", () => {
  beforeEach(async () => {
    // `main` signed out while the server could not be told: its session waits aside.
    const { writeJson } = jest.requireActual("#/platform/secure-storage");
    await writeJson("tokuly.session.departed", {
      "s-main": { token: "rt-main", allSessions: false },
    });
  });

  it("runs as the session that left, before the server ends it", async () => {
    mockBeforeEnd.mockImplementation(async (session) => {
      expect(session).not.toBe(api());
      expect(session.session.state.status).toBe("signedIn");
      calls.push("the feature takes its things back");
    });

    await settleDeparted();

    expect(calls).toEqual([
      "RefreshToken as undefined",
      "the feature takes its things back",
      "SignOut as main",
    ]);
    expect(await departedTokens()).toEqual({});
  });

  it("keeps the session aside when it fails, for the next start", async () => {
    mockBeforeEnd.mockRejectedValueOnce(new Error("offline again"));

    await settleDeparted();

    expect(calls).toEqual(["RefreshToken as undefined"]);
    expect(Object.keys(await departedTokens())).toEqual(["s-main"]);

    await settleDeparted();
    expect(calls.at(-1)).toBe("SignOut as main");
    expect(await departedTokens()).toEqual({});
  });
});
