import { QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { create, identity } from "@tokuly/api";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { currentAccount, isActive } from "#/features/session/account";
import { api } from "#/lib/api";
import { testQueryClient } from "#/testing/query";
import { PrivacyScreen } from "../privacy-screen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("#/features/session/account", () => ({
  currentAccount: jest.fn(),
  isActive: jest.fn(() => true),
}));
jest.mock("expo-router", () => ({ router: { push: jest.fn() } }));

/** One account's API: the settings it holds, and every save it was sent. */
function account() {
  let privacy = create(identity.PrivacySettingsSchema, {
    shareBattery: true,
    friendRequestsFrom: identity.Audience.EVERYONE,
    locationHistoryDays: 30,
  });
  const saves: identity.PrivacySettings[] = [];
  const service = {
    getSettings: async () => ({ settings: create(identity.SettingsSchema, { privacy }) }),
    updateSettings: async (request: { privacy: identity.PrivacySettings }) => {
      saves.push(request.privacy);
      privacy = request.privacy;
      return { settings: create(identity.SettingsSchema, { privacy }) };
    },
  };
  return { object: { service: () => service }, saves };
}

async function open() {
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={testQueryClient()}>
        <PrivacyScreen />
      </QueryClientProvider>
    </SafeAreaProvider>,
  );
  await screen.findByTestId("privacy-screen");
}

describe("privacy settings", () => {
  it("save the whole group the moment a control changes, one change after another", async () => {
    const main = account();
    jest.mocked(api).mockReturnValue(main.object as never);
    jest
      .mocked(currentAccount)
      .mockReturnValue({ sessionId: "s1", userId: "u1", api: main.object as never });
    await open();

    await fireEvent(screen.getByTestId("privacy-shareBattery"), "valueChange", false);
    await fireEvent.press(screen.getByTestId("privacy-friendRequestsFrom"));
    await fireEvent.press(screen.getByTestId("privacy-friendRequestsFrom-NOBODY"));

    await waitFor(() => expect(main.saves).toHaveLength(2));
    expect(main.saves[0]).toMatchObject({ shareBattery: false, locationHistoryDays: 30 });
    // The second save builds on the first: nothing it changed is undone.
    expect(main.saves[1]).toMatchObject({
      shareBattery: false,
      friendRequestsFrom: identity.Audience.NOBODY,
    });
    expect(screen.getByTestId("privacy-friendRequestsFrom")).toHaveProp(
      "accessibilityHint",
      "だれにも",
    );
  });

  it("send a change as the account that made it, even once another is active", async () => {
    const main = account();
    const other = account();
    jest.mocked(api).mockReturnValue(main.object as never);
    jest
      .mocked(currentAccount)
      .mockReturnValue({ sessionId: "s1", userId: "u1", api: main.object as never });
    await open();

    await fireEvent(screen.getByTestId("privacy-shareBattery"), "valueChange", false);
    jest.mocked(api).mockReturnValue(other.object as never);
    jest.mocked(isActive).mockReturnValue(false);

    await waitFor(() => expect(main.saves).toHaveLength(1));
    expect(other.saves).toHaveLength(0);
  });
});
