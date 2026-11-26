import { fromBinary, toBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { create, identity } from "@tokuly/api";
import * as SecureStore from "expo-secure-store";
import { api, resetApi } from "#/lib/api";
import { settleDeparted, signOut, startSession } from "../session";
import { useSessionStore } from "../store";
import { activeTokenStore, departedTokens } from "../vault";

jest.mock("#/generated/seams/lifecycle", () => ({ __esModule: true, default: [] }));

/** The app's own storage, at the edge: this install has started before. */
jest.mock("expo-file-system", () => ({
  Paths: { document: "documents" },
  File: class {
    get exists() {
      return true;
    }
  },
}));

const answer = (user: string, refreshToken: string) =>
  create(identity.AuthResponseSchema, {
    accessToken: `access-${user}`,
    refreshToken,
    accessTokenExpiresAt: timestampFromMs(Date.now() + 15 * 60_000),
    sessionId: `s-${user}`,
    me: { user: { id: user, handle: user } },
  });

/** The server, at the edge: what it was asked, once it can be reached. */
const calls: string[] = [];
let online = false;
const proto = (message: Uint8Array) =>
  new Response(message as BodyInit, {
    status: 200,
    headers: { "content-type": "application/proto" },
  });
async function mockServer(input: string, init: { body: Uint8Array }) {
  if (!online) throw new Error("offline");
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

beforeEach(async () => {
  calls.length = 0;
  online = false;
  resetApi();
  await api().session.adopt(answer("main", "rt-main"));
  useSessionStore.setState({ state: api().session.state, generation: 0 });
  const { writeJson } = jest.requireActual("#/platform/secure-storage");
  await writeJson("tokuly.session.parked", {});
  await writeJson("tokuly.session.departed", {});
});

describe("signing out when the active store cannot be emptied", () => {
  it("signs the app out all the same, and the next start resumes nobody", async () => {
    // Offline, and the phone refuses to delete: the token is set aside but still in the active store.
    jest.mocked(SecureStore.deleteItemAsync).mockRejectedValueOnce(new Error("storage is locked"));
    const warn = jest.spyOn(console, "warn").mockImplementation(() => undefined);
    await signOut();
    warn.mockRestore();

    expect(useSessionStore.getState().state.status).toBe("signedOut");
    expect(await departedTokens()).toEqual({ "s-main": { token: "rt-main", allSessions: false } });
    expect(await activeTokenStore.get()).toBe("rt-main");

    // The app is opened again, with a network: whoever holds the phone is not `main`.
    online = true;
    resetApi();
    await startSession();
    await settleDeparted();

    expect(useSessionStore.getState().state.status).toBe("signedOut");
    expect(await activeTokenStore.get()).toBeNull();
    expect(calls).toEqual(["RefreshToken with rt-main", "SignOut"]);
    expect(await departedTokens()).toEqual({});
  });
});
