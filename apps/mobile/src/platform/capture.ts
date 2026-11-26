import {
  disableAppSwitcherProtectionAsync,
  enableAppSwitcherProtectionAsync,
  usePreventScreenCapture,
} from "expo-screen-capture";
import { useEffect } from "react";
import { Platform } from "react-native";

/**
 * Keeps the screen that calls it out of screenshots, screen recordings and
 * the app switcher's preview for as long as it is up: for what is shown once
 * and opens the account (a two-step key, recovery codes). Android's
 * FLAG_SECURE covers all three; iOS blurs the app while it is not in front.
 */
export function useHiddenFromCapture(): void {
  usePreventScreenCapture("hidden-from-capture");
  useEffect(() => {
    if (Platform.OS !== "ios") return;
    void enableAppSwitcherProtectionAsync(1).catch(() => {});
    return () => void disableAppSwitcherProtectionAsync().catch(() => {});
  }, []);
}
