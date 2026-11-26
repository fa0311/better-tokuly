import { renderHook } from "@testing-library/react-native";
import { create, identity } from "@tokuly/api";
import { useSessionStore } from "#/features/session/store";
import { startBackgroundUpdates, stopBackgroundUpdates } from "#/platform/location";
import { writeSecure } from "#/platform/secure-storage";
import { choiceKey, useOwnLocationSharing, useSharing } from "../sharing";

// The device, at the edge: everything allowed, "always" included.
jest.mock("#/platform/location", () => ({
  readLocationAccess: jest.fn(async () => ({
    servicesEnabled: true,
    foreground: "granted",
    background: true,
    approximate: false,
  })),
  startBackgroundUpdates: jest.fn(async () => undefined),
  stopBackgroundUpdates: jest.fn(async () => undefined),
  watchForeground: jest.fn(async () => () => undefined),
  currentPosition: jest.fn(async () => null),
  readBattery: jest.fn(async () => ({})),
}));

const signedInAs = (user: string) =>
  useSessionStore.setState({
    state: {
      status: "signedIn",
      sessionId: `s-${user}`,
      me: create(identity.MeSchema, { user: { id: user, handle: user } }),
    },
  });

const settle = () => new Promise((resolve) => setTimeout(resolve, 20));

beforeEach(() => {
  jest.mocked(startBackgroundUpdates).mockClear();
  jest.mocked(stopBackgroundUpdates).mockClear();
});

describe("the background task", () => {
  it("starts for an account that shares, without stopping first", async () => {
    await writeSecure(choiceKey("alice"), "on");
    signedInAs("alice");
    useSharing.setState({ userId: null, loaded: false, choice: null, access: null });
    await renderHook(() => useOwnLocationSharing());
    await settle();
    expect(startBackgroundUpdates).toHaveBeenCalledTimes(1);
    expect(stopBackgroundUpdates).not.toHaveBeenCalled();
  });

  it("does not start for the next account on the previous one's yes", async () => {
    // Right after a switch: the store still holds alice's choice; bob never said yes.
    useSharing.setState({
      userId: "alice",
      loaded: true,
      choice: "on",
      leaving: false,
      access: {
        servicesEnabled: true,
        foreground: "granted",
        background: true,
        approximate: false,
      },
    });
    signedInAs("bob");
    await renderHook(() => useOwnLocationSharing());
    await settle();
    expect(startBackgroundUpdates).not.toHaveBeenCalled();
    expect(useSharing.getState()).toMatchObject({ userId: "bob", loaded: true, choice: null });
  });
});
