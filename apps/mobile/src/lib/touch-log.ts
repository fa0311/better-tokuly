import type { GestureResponderEvent, ViewProps } from "react-native";

/**
 * Every tap the app takes, logged in the builds the end-to-end flows run
 * (`EXPO_PUBLIC_TOUCH_LOG=1`, inlined at build time; CI's Maestro build sets
 * it and keeps logcat). When a flow's tap seems lost, the log says how far it
 * got: `touch` (it reached the JS thread, `late` ms after the finger),
 * `tab` (the tab bar took it), `shown` (the tab is on screen).
 */
const ENABLED = process.env.EXPO_PUBLIC_TOUCH_LOG === "1";

function log(line: string): void {
  console.log(`[touch-log] ${line}`);
}

function logTouch(event: GestureResponderEvent): boolean {
  const { pageX, pageY, timestamp } = event.nativeEvent;
  const late = Math.round(performance.now() - timestamp);
  log(`touch ${Math.round(pageX)},${Math.round(pageY)} late ${late} ms`);
  // Only watches: the touch goes on to whatever it is on.
  return false;
}

/** For the root view: each touch that starts, before anything takes it. */
export const touchLogProps: ViewProps = ENABLED
  ? { onStartShouldSetResponderCapture: logTouch }
  : {};

/** For the tab bar's `screenListeners`: the tab pressed, and the tab shown. */
export function tabLogListeners({
  route,
}: {
  route: { name: string };
}): Record<string, () => void> {
  if (!ENABLED) return {};
  return {
    tabPress: () => log(`tab ${route.name}`),
    focus: () => log(`shown ${route.name}`),
  };
}
