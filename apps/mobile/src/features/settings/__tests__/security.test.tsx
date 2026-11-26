import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { Code, ConnectError, create, identity } from "@tokuly/api";
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

const refused = (reason: string) =>
  new ConnectError(reason, Code.FailedPrecondition, { "x-tokuly-error": reason });

type Service = Record<string, (request: never) => Promise<unknown>>;

/** One account's AccountService: its sessions, and what it was asked. `answers` replace single methods. */
function account(answers: Service = {}) {
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
    ...answers,
  };
  const updateMe = jest.fn();
  return { object: { service: () => service, session: { updateMe } }, calls, updateMe };
}

async function open(element: ReactElement, { me = ME, answers = {} as Service } = {}) {
  const main = account(answers);
  jest.mocked(api).mockReturnValue(main.object as never);
  jest
    .mocked(currentAccount)
    .mockReturnValue({ sessionId: "here", userId: "u1", api: main.object as never });
  useSessionStore.setState({
    state: { status: "signedIn", sessionId: "here", me } as never,
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

  it("sign every other device out: one that already left is no failure, this one is never asked for", async () => {
    const alert = jest.spyOn(Alert, "alert").mockClear();
    const revoked: string[] = [];
    await open(<SecurityScreen />, {
      answers: {
        listSessions: async () => ({
          sessions: [
            { id: "here", current: true, client: { deviceName: "Pixel" } },
            { id: "laptop", current: false, client: { deviceName: "MacBook" } },
            { id: "tablet", current: false, client: { deviceName: "iPad" } },
          ],
        }),
        revokeSession: async ({ sessionId }: { sessionId: string }) => {
          if (sessionId === "laptop") throw refused("SESSION_NOT_FOUND");
          revoked.push(sessionId);
          return {};
        },
      },
    });
    await fireEvent.press(await screen.findByTestId("session-revoke-others"));
    const buttons = alert.mock.calls[0]?.[2] ?? [];
    buttons.find((button) => button.style === "destructive")?.onPress?.();

    await waitFor(() => expect(alert).toHaveBeenLastCalledWith("ほかのデバイスをすべてログアウトさせました"));
    expect(revoked).toEqual(["tablet"]);
  });

  it("two-step verification that was turned off is off, even when the account cannot be read again", async () => {
    const alert = jest.spyOn(Alert, "alert").mockClear();
    const main = await open(<SecurityScreen />, {
      me: { ...ME, totpEnabled: true },
      answers: {
        disableTotp: async () => ({}),
        getMe: async () => {
          throw new Error("offline");
        },
      },
    });
    await fireEvent.press(screen.getByTestId("totp-disable"));
    await fireEvent.changeText(screen.getByTestId("totp-disable-code"), "123456");
    await fireEvent.press(screen.getByTestId("totp-disable-confirm"));

    await waitFor(() => expect(alert).toHaveBeenCalledWith("2段階認証をオフにしました"));
    expect(main.updateMe).toHaveBeenCalledWith(expect.objectContaining({ totpEnabled: false }));
  });

  it("a setup that ran out starts over with a new key", async () => {
    const secrets = ["JBSWY3DP", "NEWKEY22"];
    await open(<TotpSetupScreen />, {
      answers: {
        beginTotpSetup: async () => ({ secret: secrets.shift(), otpauthUrl: "otpauth://totp/Tokuly" }),
        enableTotp: async () => {
          throw refused("TOTP_SETUP_EXPIRED");
        },
      },
    });
    expect(await screen.findByTestId("totp-secret")).toHaveTextContent("JBSWY3DP");
    await fireEvent.changeText(screen.getByTestId("totp-code"), "123456");
    await fireEvent.changeText(screen.getByTestId("totp-password"), "secret-pass");
    await fireEvent.press(screen.getByTestId("totp-confirm"));

    await waitFor(() => expect(screen.getByTestId("totp-secret")).toHaveTextContent("NEWKEY22"));
    expect(screen.getByTestId("totp-code")).toHaveProp("value", "");
  });
});
