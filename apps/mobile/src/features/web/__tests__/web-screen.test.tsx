import { fromBinary, toBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { act, render, screen } from "@testing-library/react-native";
import { create, identity } from "@tokuly/api";
import { Linking } from "react-native";
import { useSessionStore } from "#/features/session/session";
import { api, resetApi } from "#/lib/api";
import { SERVER_URL } from "#/lib/config";
import { WebScreen } from "../web-screen";

/** What the web view was given, render by render (the native view is the edge). */
const mockWebViews: Record<string, unknown>[] = [];
const webViews = mockWebViews;
jest.mock("react-native-webview", () => {
  const { View } = jest.requireActual("react-native");
  return {
    WebView: (props: Record<string, unknown>) => {
      mockWebViews.push(props);
      return <View testID="web-view" />;
    },
  };
});
jest.mock("expo-router", () => ({ useNavigation: () => ({ setOptions: jest.fn() }) }));

/** The server, at the edge: each CreateWebHandoff waits until the test answers it. */
interface Asked {
  path: string;
  verifierHash: string;
  bearer: string;
  credentials: string | undefined;
  answer(code: string): void;
}
const asked: Asked[] = [];
function mockServer(
  input: string,
  init: { body: Uint8Array; headers: Record<string, string>; credentials?: string },
): Promise<Response> {
  const method = new URL(input).pathname.split("/").pop();
  if (method !== "CreateWebHandoff") throw new Error(`unexpected ${method}`);
  const request = fromBinary(identity.CreateWebHandoffRequestSchema, init.body);
  const headers = new Headers(init.headers);
  return new Promise<Response>((resolve) => {
    asked.push({
      path: request.path,
      verifierHash: request.verifierHash,
      bearer: headers.get("authorization") ?? "",
      credentials: init.credentials,
      answer: (code) =>
        resolve(
          new Response(
            toBinary(
              identity.CreateWebHandoffResponseSchema,
              create(identity.CreateWebHandoffResponseSchema, {
                url: `/api/auth/handoff?code=${code}`,
              }),
            ),
            { status: 200, headers: { "content-type": "application/proto" } },
          ),
        ),
    });
  });
}
// The device's randomness and hashing, at the edge (Node's own).
jest.mock("expo-crypto", () => {
  const crypto = jest.requireActual("node:crypto");
  return {
    CryptoDigestAlgorithm: { SHA256: "SHA-256" },
    getRandomBytes: (n: number) => new Uint8Array(crypto.randomBytes(n)),
    digestStringAsync: async (_: string, text: string) =>
      crypto.createHash("sha256").update(text).digest("hex"),
  };
});
jest.mock("expo/fetch", () => ({
  fetch: (input: string, init: never) => mockServer(input, init),
}));

const signedInAs = (user: string, sessionId: string) =>
  create(identity.AuthResponseSchema, {
    accessToken: `access-${user}`,
    refreshToken: `rt-${user}`,
    accessTokenExpiresAt: timestampFromMs(Date.now() + 15 * 60_000),
    sessionId,
    me: { user: { id: user, handle: user } },
  });

/** What a sign-in, a switch or a sign-out does to the app: a new API as whoever is active. */
async function becomeActive(user: string, sessionId: string) {
  resetApi();
  await api().session.adopt(signedInAs(user, sessionId));
  useSessionStore.setState((s) => ({ state: api().session.state, generation: s.generation + 1 }));
}

const loadedUrls = () => webViews.map((props) => (props.source as { uri: string }).uri);

beforeEach(async () => {
  jest.clearAllMocks();
  webViews.length = 0;
  asked.length = 0;
  await becomeActive("alice", "s-alice");
});

describe("WebScreen", () => {
  it("loads the hand-off for the account in use, never its token, in a web view of its own", async () => {
    await render(<WebScreen path="/notes" />);
    expect(screen.getByTestId("web-loading")).toBeOnTheScreen();
    expect(asked).toHaveLength(1);
    // Asked as the app, with the app's token; the app's requests carry no cookies.
    expect(asked[0]).toMatchObject({ path: "/notes", bearer: "Bearer access-alice" });
    expect(asked[0]?.credentials).toBe("omit");

    await act(async () => asked[0]?.answer("code-alice"));
    expect(screen.getByTestId("web-view")).toBeOnTheScreen();
    const props = webViews.at(-1) ?? {};
    const source = props.source as { uri: string; headers: Record<string, string> };
    expect(source.uri).toBe(`${SERVER_URL}/api/auth/handoff?code=code-alice`);
    // The code is bound to a verifier only this load carries, in a header, never in the URL.
    const verifier = source.headers["x-tokuly-handoff-verifier"] ?? "";
    expect(verifier).toMatch(/^[0-9a-f]{64}$/);
    expect(source.uri).not.toContain(verifier);
    const { createHash } = jest.requireActual("node:crypto");
    expect(asked[0]?.verifierHash).toBe(createHash("sha256").update(verifier).digest("hex"));
    expect(Object.keys(source.headers)).toEqual(["x-tokuly-handoff-verifier"]);
    // No token, no scripts: nothing of the app's session reaches the page.
    expect(props.injectedJavaScript).toBeUndefined();
    expect(props.injectedJavaScriptBeforeContentLoaded).toBeUndefined();
    // Cookies: none to start with and none kept, nothing shared with the system or other sites.
    expect(props).toMatchObject({
      incognito: true,
      sharedCookiesEnabled: false,
      thirdPartyCookiesEnabled: false,
    });
  });

  it("never loads a code asked for as one account after a switch to another", async () => {
    const { rerender } = await render(<WebScreen path="/settings" />);
    expect(asked[0]?.bearer).toBe("Bearer access-alice");

    // Alice's code is on its way when the app switches to Bob.
    await act(() => becomeActive("bob", "s-bob"));
    await act(async () => asked[0]?.answer("code-alice"));
    expect(screen.queryByTestId("web-view")).toBeNull();
    expect(loadedUrls()).toEqual([]);

    // Bob's own web screen asks as Bob and loads only Bob's code.
    await rerender(<WebScreen key="bob" path="/settings" />);
    expect(asked[1]?.bearer).toBe("Bearer access-bob");
    await act(async () => asked[1]?.answer("code-bob"));
    expect(loadedUrls().every((url) => url.endsWith("code=code-bob"))).toBe(true);
    expect(loadedUrls().length).toBeGreaterThan(0);
  });

  it("closes the web view as soon as another account is the active one", async () => {
    await render(<WebScreen path="/" />);
    await act(async () => asked[0]?.answer("code-alice"));
    expect(screen.getByTestId("web-view")).toBeOnTheScreen();
    const shown = webViews.length;
    await act(() => becomeActive("bob", "s-bob"));
    expect(screen.queryByTestId("web-view")).toBeNull();
    expect(webViews).toHaveLength(shown);
  });

  it("opens links to anywhere else in the system browser", async () => {
    const openURL = jest.spyOn(Linking, "openURL").mockResolvedValue(undefined);
    await render(<WebScreen path="/" />);
    await act(async () => asked[0]?.answer("code-alice"));
    const decide = webViews.at(-1)?.onShouldStartLoadWithRequest as (r: { url: string }) => boolean;
    expect(decide({ url: `${SERVER_URL}/notes/abc` })).toBe(true);
    expect(openURL).not.toHaveBeenCalled();
    expect(decide({ url: "https://example.com/page" })).toBe(false);
    expect(decide({ url: "mailto:hi@example.com" })).toBe(false);
    expect(openURL).toHaveBeenCalledWith("https://example.com/page");
    expect(openURL).toHaveBeenCalledWith("mailto:hi@example.com");
  });

  it("leaves embeds of other sites in the page (iOS asks for iframes too)", async () => {
    const openURL = jest.spyOn(Linking, "openURL").mockResolvedValue(undefined);
    await render(<WebScreen path="/notes/abc" />);
    await act(async () => asked[0]?.answer("code-alice"));
    const decide = webViews.at(-1)?.onShouldStartLoadWithRequest as (r: {
      url: string;
      isTopFrame?: boolean;
    }) => boolean;
    expect(decide({ url: "https://www.youtube.com/embed/abc", isTopFrame: false })).toBe(true);
    expect(decide({ url: "https://open.spotify.com/embed/track/1", isTopFrame: false })).toBe(true);
    expect(openURL).not.toHaveBeenCalled();
    // The page itself going elsewhere still leaves the app.
    expect(decide({ url: "https://www.youtube.com/watch?v=abc", isTopFrame: true })).toBe(false);
    expect(openURL).toHaveBeenCalledWith("https://www.youtube.com/watch?v=abc");
  });
});
