import { QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { create, identity } from "@tokuly/api";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { currentAccount } from "#/features/session/account";
import { useSessionStore } from "#/features/session/store";
import { api, authClient, profileClient } from "#/lib/api";
import { testQueryClient } from "#/testing/query";
import { ProfileEditScreen } from "../profile-edit-screen";

jest.mock("#/lib/api", () => ({
  api: jest.fn(),
  apiWith: jest.fn(),
  authClient: jest.fn(),
  profileClient: jest.fn(),
}));
jest.mock("#/features/session/account", () => ({
  currentAccount: jest.fn(),
  isActive: () => true,
  ensureActive: () => {},
  AccountChanged: class AccountChanged extends Error {},
}));
jest.mock("expo-image-picker", () => ({}));
jest.mock("expo-image-manipulator", () => ({}));

const ME = create(identity.MeSchema, {
  user: { id: "u1", handle: "aoi", displayName: "あおい" },
  bio: "",
});

function account() {
  const updates: unknown[] = [];
  const updateMe = jest.fn();
  const service = {
    updateProfile: async (request: Record<string, unknown>) => {
      updates.push(request);
      return { me: { ...ME, user: { ...ME.user, ...request } } };
    },
  };
  return { object: { service: () => service, session: { updateMe } }, updates, updateMe };
}

async function open() {
  const main = account();
  jest.mocked(api).mockReturnValue(main.object as never);
  jest
    .mocked(currentAccount)
    .mockReturnValue({ sessionId: "s1", userId: "u1", api: main.object as never });
  jest.mocked(profileClient).mockReturnValue({
    getProfile: async () => ({ profile: { schoolName: "とくり高校" } }),
  } as never);
  jest.mocked(authClient).mockReturnValue({
    checkHandle: async ({ handle }: { handle: string }) =>
      handle === "taken" ? { available: false, reason: "TAKEN" } : { available: true },
  } as never);
  useSessionStore.setState({ state: { status: "signedIn", sessionId: "s1", me: ME } as never });
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={testQueryClient()}>
        <ProfileEditScreen />
      </QueryClientProvider>
    </SafeAreaProvider>,
  );
  await screen.findByDisplayValue("とくり高校");
  return main;
}

describe("profile settings", () => {
  beforeEach(() => jest.useFakeTimers());
  afterEach(() => jest.useRealTimers());

  it("save only what changed, as the account that changed it", async () => {
    const main = await open();
    await fireEvent.changeText(screen.getByTestId("profile-display-name"), "  あおい🌸 ");
    await fireEvent.press(screen.getByTestId("profile-save"));
    await waitFor(() => expect(main.updates).toEqual([{ displayName: "あおい🌸" }]));
    expect(main.updateMe).toHaveBeenCalled();
  });

  it("say a handle is taken, and keep it from being saved", async () => {
    await open();
    await fireEvent.changeText(screen.getByTestId("profile-handle"), "Taken!");
    await act(() => jest.advanceTimersByTimeAsync(400));
    expect(await screen.findByText("このハンドルはもう使われています")).toBeOnTheScreen();
    expect(screen.getByTestId("profile-save")).toBeDisabled();
  });
});
