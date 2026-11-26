import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { create, identity } from "@tokuly/api";
import type { ReactElement } from "react";
import { Alert } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { currentAccount } from "#/features/session/account";
import { useSessionStore } from "#/features/session/store";
import { api } from "#/lib/api";
import { SecurityScreen } from "../security-screen";
import { TotpSetupScreen } from "../totp-screen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("#/features/session/account", () => ({
  currentAccount: jest.fn(),
  isActive: () => true,
}));
jest.mock("expo-router", () => ({ router: { push: jest.fn(), back: jest.fn() } }));

const ME = create(identity.MeSchema, { hasPassword: true, totpEnabled: false });

/** One account's AccountService: its sessions, and what it was asked. */
function account() {
  const calls: { method: string; request: unknown }[] = [];
  const record =
    (method: string, answer: () => unknown = () => ({})) =>
    async (request: unknown) => {
      calls.push({ method, request });
      return answer();
    };
  const service = {
    listSessions: async () => ({
      sessions: [
        { id: "here", current: true, client: { deviceName: "Pixel" } },
        { id: "laptop", current: false, client: { deviceName: "MacBook" } },
      ],
    }),
    revokeSession: record("revokeSession"),
    beginTotpSetup: async () => ({ secret: "JBSWY3DP", otpauthUrl: "otpauth://totp/Tokuly" }),
    enableTotp: record("enableTotp", () => ({ recoveryCodes: ["aaaa-bbbb", "cccc-dddd"] })),
    getMe: async () => ({ me: { ...ME, totpEnabled: true } }),
  };
  const updateMe = jest.fn();
  return { object: { service: () => service, session: { updateMe } }, calls, updateMe };
}

async function open(element: ReactElement) {
  const main = account();
  jest.mocked(api).mockReturnValue(main.object as never);
  jest
    .mocked(currentAccount)
    .mockReturnValue({ sessionId: "here", userId: "u1", api: main.object as never });
  useSessionStore.setState({
    state: { status: "signedIn", sessionId: "here", me: ME } as never,
  });
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={new QueryClient()}>{element}</QueryClientProvider>
    </SafeAreaProvider>,
  );
  return main;
}

describe("security settings", () => {
  it("sign another device out after asking, never this one", async () => {
    const alert = jest.spyOn(Alert, "alert");
    const main = await open(<SecurityScreen />);
    await screen.findByText("MacBook");
    expect(screen.queryByTestId("session-revoke-here")).toBeNull();

    await fireEvent.press(screen.getByTestId("session-revoke-laptop"));
    const buttons = alert.mock.calls[0]?.[2] ?? [];
    buttons.find((button) => button.style === "destructive")?.onPress?.();
    await waitFor(() =>
      expect(main.calls).toEqual([{ method: "revokeSession", request: { sessionId: "laptop" } }]),
    );
  });

  it("turn two-step verification on with a code and the password, then show the recovery codes once", async () => {
    const main = await open(<TotpSetupScreen />);
    expect(await screen.findByTestId("totp-secret")).toHaveTextContent("JBSWY3DP");
    await fireEvent.changeText(screen.getByTestId("totp-code"), "12a3456");
    await fireEvent.changeText(screen.getByTestId("totp-password"), "secret-pass");
    await fireEvent.press(screen.getByTestId("totp-confirm"));

    expect(await screen.findByTestId("recovery-codes")).toHaveTextContent(/aaaa-bbbb/);
    expect(main.calls).toEqual([
      { method: "enableTotp", request: { code: "123456", currentPassword: "secret-pass" } },
    ]);
    expect(main.updateMe).toHaveBeenCalledWith(expect.objectContaining({ totpEnabled: true }));
  });
});
