import { realtime as rt } from "@tokuly/api";
import { RealtimeClient } from "@tokuly/realtime";
import { AppState } from "react-native";
import { api } from "#/lib/api";
import { appVersion, deviceId, platform } from "#/platform/device";

let client: RealtimeClient | null = null;
let device = "";

/** The app's single realtime connection (WebSocket: React Native has no WebTransport). */
export function realtime(): RealtimeClient {
  if (!client) {
    client = new RealtimeClient({
      async getEndpoints() {
        const endpoints = await api().service(rt.RealtimeService).getEndpoints({});
        return {
          webtransportUrl: endpoints.webtransportUrl,
          webtransportCertHashes: endpoints.webtransportCertHashes,
          websocketUrl: endpoints.websocketUrl,
        };
      },
      getAccessToken: () => api().session.accessToken(),
      refreshAccessToken: () => api().session.refresh(),
      platform,
      appVersion,
      deviceId: device,
      webTransport: false,
    });
    client.setForeground(AppState.currentState === "active");
  }
  return client;
}

/** Learns the install's ID before the first connection (it is read from secure storage). */
export async function prepareRealtime(): Promise<void> {
  device = await deviceId();
}

/** Closes the connection of the account that was active; the next `realtime()` is a new one. */
export function resetRealtime(): void {
  client?.stop();
  client = null;
}

AppState.addEventListener("change", (state) => client?.setForeground(state === "active"));
