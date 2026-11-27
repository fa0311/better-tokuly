import { toBinary } from "@bufbuild/protobuf";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react-native";
import { create, identity } from "@tokuly/api";
import { Text } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { useSessionStore } from "#/features/session/store";
import { api, resetApi } from "#/lib/api";
import { testQueryClient } from "#/testing/query";
import { TooYoungGate } from "../too-young-gate";

jest.mock("#/generated/seams/lifecycle", () => ({ __esModule: true, default: [] }));

const answer = (tooYoung: boolean) =>
  create(identity.AuthResponseSchema, {
    accessToken: `access-${tooYoung}`,
    refreshToken: `rt-${tooYoung}`,
    accessTokenExpiresAt: timestampFromMs(Date.now() + 15 * 60_000),
    sessionId: "s-kid",
    me: { user: { id: "kid", handle: "kid" }, birthdate: "2015-01-01", tooYoung },
  });

/** The server after the account answered under the minimum age on another device. */
async function mockServer(input: string): Promise<Response> {
  if (new URL(input).pathname.endsWith("/RefreshToken"))
    return new Response(toBinary(identity.AuthResponseSchema, answer(true)) as BodyInit, {
      status: 200,
      headers: { "content-type": "application/proto" },
    });
  return new Response(JSON.stringify({ code: "permission_denied", message: "too young" }), {
    status: 403,
    headers: { "content-type": "application/json", "x-tokuly-error": "ACCOUNT_TOO_YOUNG" },
  });
}
jest.mock("expo/fetch", () => ({ fetch: (input: string) => mockServer(input) }));

it("a phone shows the too-young screen on the first call the server refuses so", async () => {
  resetApi();
  await api().session.adopt(answer(false));
  useSessionStore.setState({ state: api().session.state });
  api().session.subscribe((state) => useSessionStore.setState({ state }));
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={testQueryClient()}>
        <TooYoungGate>
          <Text>アプリ</Text>
        </TooYoungGate>
      </QueryClientProvider>
    </SafeAreaProvider>,
  );
  expect(screen.getByText("アプリ")).toBeTruthy();

  const profiles = api().service(identity.ProfileService);
  await expect(profiles.getProfile({ by: { case: "handle", value: "someone" } })).rejects.toThrow();
  await waitFor(() => expect(screen.getByTestId("too-young-screen")).toBeTruthy());
  expect(screen.queryByText("アプリ")).toBeNull();
});
