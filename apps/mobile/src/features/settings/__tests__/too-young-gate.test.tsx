import type { MessageInitShape } from "@bufbuild/protobuf";
import { timestampFromDate } from "@bufbuild/protobuf/wkt";
import { QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { common, create, identity } from "@tokuly/api";
import { Text } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { useSessionStore } from "#/features/session/store";
import { api } from "#/lib/api";
import { testQueryClient } from "#/testing/query";
import { BirthdateGate } from "../birthdate-gate";
import { TooYoungGate } from "../too-young-gate";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));

function signIn(me: MessageInitShape<typeof identity.MeSchema>) {
  const account = create(identity.MeSchema, me);
  account.user ??= create(common.UserSummarySchema, { id: "u1" });
  useSessionStore.setState({ state: { status: "signedIn", me: account } as never });
}

/** The API of an account without a birthdate; what it was asked. */
function account() {
  const calls: string[] = [];
  const updateMe = jest.fn((me: identity.Me) => signIn(me));
  const refresh = jest.fn(async () => {
    calls.push("refresh");
    return true;
  });
  const service = {
    setBirthdate: async ({ birthdate }: { birthdate: string }) => {
      calls.push(`birthdate ${birthdate}`);
      return { me: create(identity.MeSchema, { user: { id: "u1" }, birthdate, minor: true, tooYoung: true }) };
    },
    requestAccountDeletion: async ({ currentPassword }: { currentPassword: string }) => {
      calls.push(`delete ${currentPassword}`);
      return { deletionScheduledAt: timestampFromDate(new Date(Date.now() + 30 * 24 * 3600 * 1000)) };
    },
  };
  jest.mocked(api).mockReturnValue({ service: () => service, session: { updateMe, refresh } } as never);
  return { calls };
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
        <TooYoungGate>
          <BirthdateGate>
            <Text>アプリ</Text>
          </BirthdateGate>
        </TooYoungGate>
      </QueryClientProvider>
    </SafeAreaProvider>,
  );
}

describe("the too-young gate", () => {
  it("lets everyone else in", async () => {
    signIn({ birthdate: "2010-01-01", minor: true });
    await open();
    expect(screen.getByText("アプリ")).toBeTruthy();
    expect(screen.queryByTestId("too-young-screen")).toBeNull();
  });

  it("shows an answer under 13 why, with a fresh token, and nothing of the app", async () => {
    const { calls } = account();
    signIn({ birthdate: "" });
    await open();
    await fireEvent.changeText(screen.getByTestId("birthdate-input"), "2016-05-05");
    await fireEvent.press(screen.getByTestId("birthdate-submit"));
    await waitFor(() => expect(screen.getByTestId("too-young-screen")).toBeTruthy());
    expect(screen.getByText("ごめんね、Tokulyは13歳からなんだ")).toBeTruthy();
    expect(screen.queryByText("アプリ")).toBeNull();
    await waitFor(() => expect(calls).toEqual(["birthdate 2016-05-05", "refresh"]));
  });

  it("deletes the account with the password", async () => {
    const { calls } = account();
    signIn({ birthdate: "2016-05-05", tooYoung: true, hasPassword: true });
    await open();
    await fireEvent.press(screen.getByTestId("too-young-delete"));
    await fireEvent.changeText(screen.getByTestId("too-young-password"), "secret pass");
    await fireEvent.press(screen.getByTestId("too-young-confirm"));
    await waitFor(() => expect(screen.getByTestId("too-young-scheduled")).toBeTruthy());
    expect(calls).toEqual(["delete secret pass"]);
  });
});
