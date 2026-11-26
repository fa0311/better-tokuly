import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { api } from "#/lib/api";
import { CreateShortScreen } from "../create-screen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("expo-router", () => ({ router: { replace: jest.fn() } }));
// The camera and the library are the device's.
jest.mock("react-native-vision-camera", () => ({}));
jest.mock("#/features/media/pick", () => ({ pickFromLibrary: jest.fn(), trimVideo: jest.fn() }));

const PUBLIC_HINT = "おすすめやタグ、検索にも出るよ";
const LOCKED_NOTE = "鍵アカウントなので、全体に公開してもフォロワーとともだちにだけ届くよ";

async function open(locked: boolean) {
  jest.mocked(api).mockReturnValue({
    service: () => ({ getFollowSettings: async () => ({ settings: { locked } }) }),
  } as never);
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={new QueryClient()}>
        <CreateShortScreen />
      </QueryClientProvider>
    </SafeAreaProvider>,
  );
}

describe("CreateShortScreen", () => {
  it("tells a locked account that public reaches only its followers and friends", async () => {
    await open(true);
    expect(await screen.findByTestId("locked-note")).toBeOnTheScreen();
    expect(screen.queryByText(PUBLIC_HINT)).not.toBeOnTheScreen();
    expect(screen.getAllByText(LOCKED_NOTE).length).toBeGreaterThan(0);
  });

  it("says public reaches everyone for an account that is not locked", async () => {
    await open(false);
    expect(await screen.findByText(PUBLIC_HINT)).toBeOnTheScreen();
    expect(screen.queryByTestId("locked-note")).not.toBeOnTheScreen();
  });
});
