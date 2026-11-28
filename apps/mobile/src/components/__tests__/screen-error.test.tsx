import { render } from "@testing-library/react-native";
import { router, Stack, Tabs } from "expo-router";
import { act, fireEvent, renderRouter, screen } from "expo-router/testing-library";
import { Text } from "react-native";
import { FailsAlone } from "../fails-alone";
import { RootError, ScreenError } from "../screen-error";

const FAILED = "うまくいきませんでした";

/** Whether `Broken` throws now: a bug that a retry may get past. */
let broken = true;

function Broken({ name }: { name: string }) {
  if (broken) throw new Error(`${name} cannot render`);
  return <Text>{name}</Text>;
}

function Fine({ name }: { name: string }) {
  return <Text>{name}</Text>;
}

/** The app's shape: a stack whose screens inherit `ScreenError` (as `app/_layout.tsx`), tabs inside it. */
const APP = {
  _layout: () => <Stack unstable_screenErrorBoundary={ScreenError} />,
  "(tabs)/_layout": () => (
    <Tabs>
      <Tabs.Screen name="map" />
      <Tabs.Screen name="chats" options={{ tabBarButtonTestID: "tab-chats" }} />
    </Tabs>
  ),
  "(tabs)/chats": () => <Fine name="トーク一覧" />,
};

// React and React Native report every error a boundary caught; that is expected here.
beforeEach(() => {
  broken = true;
  jest.spyOn(console, "error").mockImplementation(() => {});
});
afterEach(() => jest.restoreAllMocks());

describe("ScreenError", () => {
  it("fails one tab in its place, and the other tabs still open", async () => {
    await renderRouter(
      { ...APP, "(tabs)/map": () => <Broken name="マップ" /> },
      { initialUrl: "/map" },
    );
    expect(screen.getByText(FAILED)).toBeTruthy();
    expect(screen.getByText("もう一度")).toBeTruthy();

    await fireEvent.press(screen.getByTestId("tab-chats"));
    expect(screen.getByText("トーク一覧")).toBeTruthy();
  });

  it("renders the screen again on もう一度", async () => {
    await renderRouter(
      { ...APP, "(tabs)/map": () => <Broken name="マップ" /> },
      { initialUrl: "/map" },
    );
    broken = false;
    await fireEvent.press(screen.getByTestId("screen-error-retry"));
    expect(screen.getByText("マップ")).toBeTruthy();
    expect(screen.queryByText(FAILED)).toBeNull();
  });

  it("leaves a pushed screen that failed on もどる", async () => {
    const app = await renderRouter(
      {
        ...APP,
        "(tabs)/map": () => <Fine name="マップ" />,
        "chat/[id]": () => <Broken name="トーク" />,
      },
      { initialUrl: "/map" },
    );
    expect(screen.queryByTestId("screen-error-back")).toBeNull();
    await act(() => router.push("/chat/1"));
    expect(screen.getByText(FAILED)).toBeTruthy();

    await fireEvent.press(screen.getByTestId("screen-error-back"));
    expect(app.getPathname()).toBe("/map");
    expect(screen.getByText("マップ")).toBeTruthy();
  });
});

describe("RootError", () => {
  it("shows the failure, not a blank app, when the root layout throws; もう一度 renders it again", async () => {
    await renderRouter(
      {
        _layout: { default: () => <Broken name="ルート" />, ErrorBoundary: RootError },
        index: () => <Fine name="ホーム" />,
      },
      { initialUrl: "/" },
    );
    expect(screen.getByText(FAILED)).toBeTruthy();
    expect(screen.queryByTestId("screen-error-back")).toBeNull();

    broken = false;
    await fireEvent.press(screen.getByTestId("screen-error-retry"));
    expect(screen.getByText("ルート")).toBeTruthy();
  });
});

describe("FailsAlone", () => {
  it("stops only what threw", async () => {
    const view = await render(
      <>
        <FailsAlone>
          <Broken name="同期" />
        </FailsAlone>
        <FailsAlone>
          <Fine name="位置の共有" />
        </FailsAlone>
      </>,
    );
    expect(view.getByText("位置の共有")).toBeTruthy();
    expect(view.queryByText("同期")).toBeNull();
  });
});
