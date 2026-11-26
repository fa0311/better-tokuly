import { fromBinary, toBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { act, render, screen } from "@testing-library/react-native";
import { create, identity } from "@tokuly/api";
import { Linking } from "react-native";
import { currentAccount } from "#/features/session/account";
import { useSessionStore } from "#/features/session/store";
import { api, resetApi } from "#/lib/api";
import { SERVER_URL } from "#/lib/config";
import { clearWebData } from "#/platform/web-data";
import { runBeforeEnd, runBeforeLeave } from "#/seams/lifecycle";
import lifecycle from "../lifecycle";
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
// The device's web data (cookies, storage, cache of every web view), at the edge.
const mockEvents: string[] = [];
jest.mock("#/platform/web-data", () => ({
  clearWebData: jest.fn(async () => void mockEvents.push("cleared")),
}));
/** The navigator, at the edge: what the screens on the stack asked to hear when they are back on show. */
const mockFocusListeners = new Set<() => void>();
jest.mock("expo-router", () => ({
  useNavigation: () => ({
    setOptions: jest.fn(),
    addListener: (event: string, listener: () => void) => {
      if (event === "focus") mockFocusListeners.add(listener);
      return () => mockFocusListeners.delete(listener);
    },
  }),
}));
/** The screens come back on show, the oldest on the stack first. */
const focus = (index: number) => [...mockFocusListeners][index]?.();

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
  mockEvents.push("asked");
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
  mockEvents.length = 0;
  mockFocusListeners.clear();
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

  it("hands the system only links a person may follow from a page", async () => {
    const openURL = jest.spyOn(Linking, "openURL").mockResolvedValue(undefined);
    await render(<WebScreen path="/notes/abc" />);
    await act(async () => asked[0]?.answer("code-alice"));
    const decide = webViews.at(-1)?.onShouldStartLoadWithRequest as (r: { url: string }) => boolean;
    // A page (or an embed in it) may not start other apps, open files or run script URLs
    // through the app: the web view does not load them and nothing else opens them.
    for (const url of [
      "intent://scan/#Intent;scheme=zxing;package=com.example;end",
      "tokuly://web?path=/wallet",
      "someapp://pay?to=mallory",
      "file:///data/data/app.tokuly/files/secret",
      "content://com.android.contacts/contacts",
      "javascript:alert(1)",
      "data:text/html,<script>alert(1)</script>",
      "not a url",
    ])
      expect(decide({ url })).toBe(false);
    expect(openURL).not.toHaveBeenCalled();
    for (const url of ["https://example.com/", "http://example.com/", "tel:110", "sms:110"])
      expect(decide({ url })).toBe(false);
    expect(openURL).toHaveBeenCalledTimes(4);
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

  it("starts from empty web data, and leaves none behind when it closes", async () => {
    const { unmount } = await render(<WebScreen path="/notes" />);
    // Cleared before the code is even asked for: nothing of an earlier web view (another
    // account's localStorage, a cookie left on disk by a crash) is there when it loads.
    expect(mockEvents).toEqual(["cleared", "asked"]);
    await act(async () => asked[0]?.answer("code-alice"));
    await unmount();
    expect(mockEvents).toEqual(["cleared", "asked", "cleared"]);
  });

  it("opens again when it is back on show after a newer web screen took its session", async () => {
    const first = await render(<WebScreen path="/wallet" />);
    await act(async () => asked[0]?.answer("code-wallet"));
    // Back on show with nothing opened since (a native screen was over it): it stays as it is.
    await act(async () => focus(0));
    expect(asked).toHaveLength(1);

    // A push about a note opens over it: the server keeps one web session per app
    // session, the newest, so the wallet's is over.
    const second = await render(<WebScreen path="/notes" />);
    await act(async () => asked[1]?.answer("code-notes"));
    await second.unmount();
    await act(async () => focus(0));
    expect(asked).toHaveLength(3);
    expect(asked[2]).toMatchObject({ path: "/wallet", bearer: "Bearer access-alice" });
    await act(async () => asked[2]?.answer("code-wallet-again"));
    expect(loadedUrls().at(-1)).toBe(`${SERVER_URL}/api/auth/handoff?code=code-wallet-again`);
    await first.unmount();
  });

  it("leaves a web screen opened after it its data when it closes", async () => {
    // One web screen is replaced by another (a push about a note while the wallet is
    // open, a tile tapped while the last screen still slides away): the first one is
    // gone only after the second has loaded its hand-off. Android's web views share
    // one cookie store, so clearing it now would sign the second one out.
    const first = await render(<WebScreen path="/wallet" />);
    await act(async () => asked[0]?.answer("code-wallet"));
    const second = await render(<WebScreen path="/notes" />);
    await act(async () => asked[1]?.answer("code-notes"));
    mockEvents.length = 0;

    await first.unmount();
    expect(mockEvents).toEqual([]);
    // The last one to close leaves nothing behind.
    await second.unmount();
    expect(mockEvents).toEqual(["cleared"]);
  });
});

describe("the web screens' lifecycle", () => {
  it("clears the web data before the account leaves and before a departed session ends", async () => {
    const account = currentAccount();
    if (!account) throw new Error("signed in");
    await runBeforeLeave(account, [lifecycle]);
    expect(clearWebData).toHaveBeenCalledTimes(1);
    await runBeforeEnd(api(), [lifecycle]);
    expect(clearWebData).toHaveBeenCalledTimes(2);
  });

  it("does not take a web screen's data when a departed session ends in the background", async () => {
    // The departed sessions are settled while the app is already in use: the web screen
    // on show started from empty data after that session left, and its cookie is its own.
    const { unmount } = await render(<WebScreen path="/notes" />);
    await act(async () => asked[0]?.answer("code-alice"));
    mockEvents.length = 0;
    await runBeforeEnd(api(), [lifecycle]);
    expect(mockEvents).toEqual([]);

    // The account in use leaving is another matter: its web screens are about to close.
    const account = currentAccount();
    if (!account) throw new Error("signed in");
    await runBeforeLeave(account, [lifecycle]);
    expect(mockEvents).toEqual(["cleared"]);
    await unmount();
  });
});
