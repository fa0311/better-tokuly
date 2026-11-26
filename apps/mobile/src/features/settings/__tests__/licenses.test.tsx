import { fireEvent, render, screen } from "@testing-library/react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import notices from "#/generated/licenses.json";
import { LicensesScreen } from "../licenses-screen";

const names = notices.packages.map((notice) => notice.name);

describe("open-source notices", () => {
  it("list what the app bundles, with the native libraries of its own modules", () => {
    expect(names).toEqual(expect.arrayContaining(["react", "react-native", "expo-video"]));
    expect(names).toEqual(
      expect.arrayContaining([
        "androidx.media3:media3-transformer",
        "androidx.media3:media3-common",
      ]),
    );
    const react = notices.packages.find((notice) => notice.name === "react");
    expect(react?.license).toBe("MIT");
    expect(notices.texts[react?.text ?? -1]).toMatch(/Permission is hereby granted/);
  });

  it("leave out the app's own packages, and FFmpeg is not in the app", () => {
    expect(names.some((name) => name.startsWith("@tokuly/"))).toBe(false);
    expect(names.some((name) => /ffmpeg|video-trim/i.test(name))).toBe(false);
  });

  it("show a package's licence text on a tap, and find packages by name", async () => {
    await render(
      <SafeAreaProvider
        initialMetrics={{
          frame: { x: 0, y: 0, width: 320, height: 640 },
          insets: { top: 0, left: 0, right: 0, bottom: 0 },
        }}
      >
        <LicensesScreen />
      </SafeAreaProvider>,
    );
    await fireEvent.changeText(screen.getByTestId("licenses-search"), "media3-transformer");
    await fireEvent.press(screen.getByTestId("license-androidx.media3:media3-transformer"));
    expect(screen.getByText(/Apache License/)).toBeOnTheScreen();
  });
});
