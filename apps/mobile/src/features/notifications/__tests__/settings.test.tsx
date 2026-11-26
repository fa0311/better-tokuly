import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { create, notify } from "@tokuly/api";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { currentAccount } from "#/features/session/account";
import { api } from "#/lib/api";
import { NotificationSettingsScreen } from "../settings-screen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("#/features/session/account", () => ({
  currentAccount: jest.fn(),
  isActive: jest.fn(() => true),
}));
jest.mock("expo-router", () => ({ router: { push: jest.fn() } }));
// Push on this device has its own tests (push.test.tsx).
jest.mock("../push-card", () => ({ PushCard: () => null }));

/** One account's preferences service: what it holds, and every update it was sent. */
function account() {
  let preferences = create(notify.PreferencesSchema, {
    pushPreview: notify.PushPreview.FULL,
    timeZone: "Asia/Tokyo",
    quietHours: { enabled: false, startMinute: 22 * 60, endMinute: 7 * 60 },
    categories: [{ category: "chat", label: "チャット", inbox: true, push: true }],
  });
  const updates: Record<string, unknown>[] = [];
  const service = {
    getPreferences: async () => ({ preferences }),
    updatePreferences: async (request: Record<string, unknown>) => {
      updates.push(request);
      preferences = create(notify.PreferencesSchema, { ...preferences, ...request });
      return { preferences };
    },
  };
  return { object: { service: () => service }, updates };
}

async function open() {
  const main = account();
  jest.mocked(api).mockReturnValue(main.object as never);
  jest
    .mocked(currentAccount)
    .mockReturnValue({ sessionId: "s1", userId: "u1", api: main.object as never });
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={new QueryClient()}>
        <NotificationSettingsScreen />
      </QueryClientProvider>
    </SafeAreaProvider>,
  );
  await screen.findByTestId("push-preview-hidden");
  return main;
}

describe("notification settings", () => {
  it("choose what a push shows on the lock screen", async () => {
    const main = await open();
    await fireEvent.press(screen.getByTestId("push-preview-senderOnly"));
    await waitFor(() =>
      expect(main.updates).toEqual([{ pushPreview: notify.PushPreview.SENDER_ONLY }]),
    );
    await waitFor(() =>
      expect(screen.getByTestId("push-preview-senderOnly")).toHaveProp("accessibilityState", {
        selected: true,
      }),
    );
  });

  it("build each quiet-hours change on the one before, even before it was answered", async () => {
    const main = await open();
    await fireEvent(screen.getByTestId("quiet-hours"), "valueChange", true);
    await fireEvent.press(screen.getByTestId("quiet-end"));
    await fireEvent.press(screen.getByTestId("quiet-end-480"));
    await waitFor(() => expect(main.updates).toHaveLength(2));
    expect(main.updates[1]).toEqual({
      quietHours: { enabled: true, startMinute: 22 * 60, endMinute: 8 * 60 },
    });
    // The start is never offered as the end: the window needs two different times.
    await fireEvent.press(screen.getByTestId("quiet-end"));
    expect(screen.queryByTestId(`quiet-end-${22 * 60}`)).toBeNull();
  });

  it("switch a category's push off and keep it in the inbox", async () => {
    const main = await open();
    await fireEvent(screen.getByTestId("category-chat-push"), "valueChange", false);
    await waitFor(() =>
      expect(main.updates).toEqual([{ categories: [{ category: "chat", push: false }] }]),
    );
  });
});
