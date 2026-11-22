import { common, realtime as rt } from "@tokuly/api";
import { RealtimeClient } from "@tokuly/realtime";
import { api } from "#/lib/api";

const DEVICE_ID_KEY = "tokuly.deviceId";

/** A stable random ID for this browser profile (de-duplicates sessions and connections). */
export function deviceId(): string {
  let id = window.localStorage.getItem(DEVICE_ID_KEY);
  if (!id) {
    id = crypto.randomUUID();
    window.localStorage.setItem(DEVICE_ID_KEY, id);
  }
  return id;
}

let client: RealtimeClient | null = null;

/** The app's single realtime connection (created on first use, browser only). */
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
      platform: common.Platform.WEB,
      appVersion: "web",
      deviceId: deviceId(),
    });
    document.addEventListener("visibilitychange", () => {
      client?.setForeground(document.visibilityState === "visible");
    });
    // Development aid: inspect the connection from the browser console.
    if (import.meta.env.DEV)
      (window as unknown as { __tokulyRealtime?: RealtimeClient }).__tokulyRealtime = client;
  }
  return client;
}
