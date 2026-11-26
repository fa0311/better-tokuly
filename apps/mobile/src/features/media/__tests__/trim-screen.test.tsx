import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { trimFile } from "#/platform/trim";
import { clampStart, TrimScreen } from "../trim-screen";

// The device's cutting and the video player are the edges.
jest.mock("#/platform/trim", () => ({ trimFile: jest.fn(async () => "file:///cache/trim-1.mp4") }));
jest.mock("expo-video", () => {
  const { View } = require("react-native");
  return {
    VideoView: () => require("react").createElement(View),
    useVideoPlayer: () => ({ play: jest.fn(), pause: jest.fn(), currentTime: 0 }),
  };
});

const video = {
  uri: "file:///picked/long.mov",
  kind: "video" as const,
  name: "long.mov",
  contentType: "video/quicktime",
  duration: 240,
};

async function open(onDone = jest.fn()) {
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <TrimScreen video={video} maxSeconds={180} onDone={onDone} onCancel={jest.fn()} />
    </SafeAreaProvider>,
  );
  await fireEvent(screen.getByTestId("trim-track"), "layout", {
    nativeEvent: { layout: { width: 240, height: 56 } },
  });
  return onDone;
}

describe("TrimScreen", () => {
  it("keeps the window inside the video", () => {
    expect(clampStart(-5, 180, 240)).toBe(0);
    expect(clampStart(30, 180, 240)).toBe(30);
    expect(clampStart(100, 180, 240)).toBe(60);
    expect(clampStart(10, 60, 40)).toBe(0);
  });

  it("starts with the first three minutes and cuts the part the window was dragged to", async () => {
    const onDone = await open();
    expect(screen.getByTestId("trim-range")).toHaveTextContent("0:00 〜 3:00 の 3:00 を使うよ");
    const handle = screen.getByTestId("trim-window");
    // 240 px for 240 s: 30 px to the right is 30 s later.
    await fireEvent(handle, "responderGrant", { touchHistory: { touchBank: [] } });
    await fireEvent(handle, "responderMove", {
      touchHistory: {
        numberActiveTouches: 1,
        indexOfSingleActiveTouch: 0,
        mostRecentTimeStamp: 1,
        touchBank: [
          {
            touchActive: true,
            startPageX: 0,
            startPageY: 0,
            startTimeStamp: 0,
            currentPageX: 30,
            currentPageY: 0,
            currentTimeStamp: 1,
            previousPageX: 0,
            previousPageY: 0,
            previousTimeStamp: 0,
          },
        ],
      },
    });
    expect(screen.getByTestId("trim-range")).toHaveTextContent("0:30 〜 3:30 の 3:00 を使うよ");
    await fireEvent.press(screen.getByTestId("trim-done"));
    await waitFor(() =>
      expect(onDone).toHaveBeenCalledWith(
        expect.objectContaining({
          uri: "file:///cache/trim-1.mp4",
          name: "long.mp4",
          contentType: "video/mp4",
          duration: 180,
        }),
      ),
    );
    expect(trimFile).toHaveBeenCalledWith("file:///picked/long.mov", 30_000, 210_000);
  });
});
