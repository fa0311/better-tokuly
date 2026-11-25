import { fromBinary, toBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { act, renderHook, waitFor } from "@testing-library/react-native";
import { create, identity, notify } from "@tokuly/api";
import { signOut, switchAccount, useSessionStore } from "#/features/session/session";
import { api, resetApi } from "#/lib/api";
import { pushChoiceKey, usePush, usePushRegistration } from "../push";

/**
 * The server at the edge: every call is written down with the account it
 * was made as (its bearer token), so the tests can say whose session a
 * registration went to.
 */
const calls: string[] = [];
let unregisterFails = false;
let switchAnswer: identity.AuthResponse;

const ok = (body: Uint8Array = new Uint8Array()) =>
  new Response(body as BodyInit, { status: 200, headers: { "content-type": "application/proto" } });

async function mockServer(input: string, init: { body: Uint8Array; headers?: HeadersInit }) {
  const method = new URL(input).pathname.split("/").pop() ?? "";
  const bearer = new Headers(init.headers).get("authorization")?.replace("Bearer access-", "");
  switch (method) {
    case "GetPushConfig":
      return ok(
        toBinary(
          notify.GetPushConfigResponseSchema,
          create(notify.GetPushConfigResponseSchema, {
            channels: [notify.PushChannelKind.APNS, notify.PushChannelKind.FCM],
          }),
        ),
      );
    case "RegisterDevice": {
      const request = fromBinary(notify.RegisterDeviceRequestSchema, init.body);
      calls.push(`register as ${bearer}: ${request.token.case}`);
      return ok();
    }
    case "UnregisterDevice":
      calls.push(`unregister as ${bearer}`);
      if (unregisterFails)
        return new Response(JSON.stringify({ code: "unavailable", message: "down" }), {
          status: 503,
          headers: { "content-type": "application/json" },
        });
      return ok();
    case "SwitchAccount":
      calls.push(`switch as ${bearer}`);
      return ok(toBinary(identity.AuthResponseSchema, switchAnswer));
    default:
      calls.push(`${method} as ${bearer}`);
      return ok();
  }
}
jest.mock("expo/fetch", () => ({
  fetch: (input: string, init: { body: Uint8Array; headers?: HeadersInit }) =>
    mockServer(input, init),
}));

/** The device: notifications allowed, one APNs token, a tray that can be cleared. */
const device: string[] = [];
jest.mock("expo-notifications", () => ({
  IosAuthorizationStatus: { PROVISIONAL: 3 },
  AndroidImportance: { DEFAULT: 3, HIGH: 4 },
  AndroidNotificationVisibility: { PRIVATE: 2 },
  getPermissionsAsync: jest.fn(async () => ({ granted: true, canAskAgain: true })),
  requestPermissionsAsync: jest.fn(async () => ({ granted: true, canAskAgain: true })),
  getDevicePushTokenAsync: jest.fn(async () => ({ type: "ios", data: "a1b2c3" })),
  unregisterForNotificationsAsync: jest.fn(async () => void device.push("token dropped")),
  dismissAllNotificationsAsync: jest.fn(async () => void device.push("tray cleared")),
  setBadgeCountAsync: jest.fn(async () => true),
  addPushTokenListener: jest.fn(() => ({ remove: () => {} })),
}));

const signedInAs = (user: string, sessionId: string) =>
  create(identity.AuthResponseSchema, {
    accessToken: `access-${user}`,
    refreshToken: `rt-${user}`,
    accessTokenExpiresAt: timestampFromMs(Date.now() + 15 * 60_000),
    sessionId,
    me: { user: { id: user, handle: user } },
  });

const { writeSecure, writeJson } = jest.requireActual("#/platform/secure-storage");

/** `main` is signed in and said yes to push on this device; `alt` is parked and never decided. */
async function signedInMain() {
  resetApi();
  await api().session.adopt(signedInAs("main", "s-main"));
  useSessionStore.setState({ state: api().session.state, generation: 0 });
  await writeJson("tokuly.session.parked", { "s-alt": "rt-alt" });
  await writeSecure(pushChoiceKey("main"), "on");
  await writeSecure(pushChoiceKey("alt"), null);
  return renderHook(() => usePushRegistration());
}

beforeEach(() => {
  calls.length = 0;
  device.length = 0;
  unregisterFails = false;
  switchAnswer = signedInAs("alt", "s-alt");
});

describe("push registration", () => {
  it("registers the device for the account that said yes on it", async () => {
    const { unmount } = await signedInMain();
    await waitFor(() => expect(calls).toEqual(["register as main: apns"]));
    unmount();
  });

  it("unregisters the account before another one takes over, and not for the next one unasked", async () => {
    const hook = await signedInMain();
    await waitFor(() => expect(calls).toContain("register as main: apns"));

    await act(async () => {
      await expect(switchAccount("s-alt")).resolves.toBe("switched");
    });
    // The leaving account's registration went while its session still worked.
    expect(calls).toEqual(["register as main: apns", "switch as main", "unregister as main"]);
    expect(device).toEqual(["tray cleared"]);

    // The next account starts undecided: nothing is registered for it.
    hook.rerender({});
    await waitFor(() => expect(usePush.getState()).toMatchObject({ userId: "alt", loaded: true }));
    expect(usePush.getState().choice).toBeNull();
    expect(calls.filter((call) => call.startsWith("register"))).toEqual(["register as main: apns"]);
    hook.unmount();
  });

  it("registers the next account when it said yes here before", async () => {
    const hook = await signedInMain();
    await waitFor(() => expect(calls).toContain("register as main: apns"));
    await writeSecure(pushChoiceKey("alt"), "on");

    await act(() => switchAccount("s-alt"));
    hook.rerender({});
    await waitFor(() => expect(calls).toContain("register as alt: apns"));
    hook.unmount();
  });

  it("unregisters before signing out", async () => {
    const hook = await signedInMain();
    await waitFor(() => expect(calls).toContain("register as main: apns"));

    await act(() => signOut());
    expect(calls.indexOf("unregister as main")).toBeGreaterThan(-1);
    expect(calls.indexOf("unregister as main")).toBeLessThan(calls.indexOf("SignOut as main"));
    hook.unmount();
  });

  it("throws the device token away when the server cannot be told", async () => {
    const hook = await signedInMain();
    await waitFor(() => expect(calls).toContain("register as main: apns"));
    unregisterFails = true;

    await act(() => signOut());
    expect(calls).toContain("unregister as main");
    expect(device).toEqual(["token dropped", "tray cleared"]);
    hook.unmount();
  });
});
