import { createFileRoute } from "@tanstack/react-router";
import { DeviceScreen } from "#/features/dev/oauth/device-screen";
import { NO_FRAMING } from "#/features/dev/oauth/headers";

/**
 * `/oauth/device`: where the code a device shows is typed (the OAuth device
 * flow). Like the consent page it works for visitors who still have to sign
 * in, and may never be shown inside a frame.
 */
export const Route = createFileRoute("/oauth/device")({
  ssr: false,
  headers: () => NO_FRAMING,
  component: DeviceScreen,
});
