import type { MessageInitShape } from "@bufbuild/protobuf";
import { QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { common, create, identity } from "@tokuly/api";
import { Text } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { useSessionStore } from "#/features/session/store";
import { api } from "#/lib/api";
import { testQueryClient } from "#/testing/query";
import { BirthdateGate } from "../birthdate-gate";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));

function signIn(me: MessageInitShape<typeof identity.MeSchema>) {
  const account = create(identity.MeSchema, me);
  account.user ??= create(common.UserSummarySchema, { id: "u1" });
  useSessionStore.setState({ state: { status: "signedIn", me: account } as never });
}

/** The API of an account without a birthdate; every answer it was given. */
function account() {
  const answers: string[] = [];
  const updateMe = jest.fn((me: identity.Me) => signIn(me));
  const service = {
    setBirthdate: async ({ birthdate }: { birthdate: string }) => {
      answers.push(birthdate);
      return { me: create(identity.MeSchema, { user: { id: "u1" }, birthdate, minor: true }) };
    },
  };
  jest.mocked(api).mockReturnValue({ service: () => service, session: { updateMe } } as never);
  return { answers };
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
        <BirthdateGate>
          <Text>アプリ</Text>
        </BirthdateGate>
      </QueryClientProvider>
    </SafeAreaProvider>,
  );
}

describe("the birthdate gate", () => {
  it("lets an account with a birthdate straight in", async () => {
    signIn({ birthdate: "2000-01-01" });
    await open();
    expect(screen.getByText("アプリ")).toBeTruthy();
    expect(screen.queryByTestId("birthdate-screen")).toBeNull();
  });

  it("asks an account without one, and opens the app once it answers", async () => {
    const { answers } = account();
    signIn({ birthdate: "" });
    await open();
    expect(screen.getByTestId("birthdate-screen")).toBeTruthy();
    expect(screen.queryByText("アプリ")).toBeNull();

    await fireEvent.changeText(screen.getByTestId("birthdate-input"), "2011-05-05");
    await fireEvent.press(screen.getByTestId("birthdate-submit"));
    await waitFor(() => expect(screen.getByText("アプリ")).toBeTruthy());
    expect(answers).toEqual(["2011-05-05"]);
  });

  it("lets it answer later", async () => {
    account();
    signIn({ birthdate: "", user: { id: "u2" } });
    await open();
    await fireEvent.press(screen.getByTestId("birthdate-later"));
    expect(screen.getByText("アプリ")).toBeTruthy();
  });
});
