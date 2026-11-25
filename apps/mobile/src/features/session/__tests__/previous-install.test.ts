import { fromBinary, toBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { waitFor } from "@testing-library/react-native";
import { create, identity } from "@tokuly/api";
import { readSecure, writeSecure } from "#/platform/secure-storage";
import { startSession, useSessionStore } from "../session";

/** The app's own storage, at the edge: empty, as after a reinstall (the keychain outlived it). */
const files = new Set<string>();
jest.mock("expo-file-system", () => ({
  Paths: { document: "documents" },
  File: class {
    private readonly path: string;
    constructor(directory: string, name: string) {
      this.path = `${directory}/${name}`;
    }
    get exists() {
      return mockFiles().has(this.path);
    }
    create() {
      mockFiles().add(this.path);
    }
  },
}));
const mockFiles = () => files;

/** The server, at the edge: the previous install's sessions are still alive on it. */
const calls: string[] = [];
async function mockServer(input: string, init: { body: Uint8Array; headers?: HeadersInit }) {
  const method = new URL(input).pathname.split("/").pop() ?? "";
  const bearer = new Headers(init.headers).get("authorization")?.replace("Bearer access-", "");
  if (method === "RefreshToken") {
    const { refreshToken } = fromBinary(identity.RefreshTokenRequestSchema, init.body);
    calls.push(`refresh ${refreshToken}`);
    const user = refreshToken.replace(/^rt-/, "");
    const answer = create(identity.AuthResponseSchema, {
      accessToken: `access-${user}`,
      refreshToken: `${refreshToken}-2`,
      accessTokenExpiresAt: timestampFromMs(Date.now() + 15 * 60_000),
      sessionId: `s-${user}`,
      me: { user: { id: user, handle: user } },
    });
    return new Response(toBinary(identity.AuthResponseSchema, answer) as BodyInit, {
      status: 200,
      headers: { "content-type": "application/proto" },
    });
  }
  calls.push(`${method} as ${bearer}`);
  return new Response(new Uint8Array() as BodyInit, {
    status: 200,
    headers: { "content-type": "application/proto" },
  });
}
jest.mock("expo/fetch", () => ({
  fetch: (input: string, init: { body: Uint8Array; headers?: HeadersInit }) =>
    mockServer(input, init),
}));

describe("a reinstall", () => {
  it("signs the previous install's accounts out on the server before forgetting them", async () => {
    await writeSecure("tokuly.session.active", "rt-before");
    await writeSecure("tokuly.session.parked", JSON.stringify({ "s-other": "rt-other" }));

    await startSession();
    expect(useSessionStore.getState().state.status).toBe("signedOut");
    expect(await readSecure("tokuly.session.active")).toBeNull();
    expect(await readSecure("tokuly.session.parked")).toBeNull();

    // Best effort, in the background: each session's push registrations go, then the session.
    await waitFor(() =>
      expect(calls).toEqual([
        "refresh rt-before",
        "UnregisterDevice as before",
        "SignOut as before",
        "refresh rt-other",
        "UnregisterDevice as other",
        "SignOut as other",
      ]),
    );
    await waitFor(async () => expect(await readSecure("tokuly.session.departed")).toBe("{}"));
  });
});
