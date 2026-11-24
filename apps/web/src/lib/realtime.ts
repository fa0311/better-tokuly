import type { QueryClient, QueryKey } from "@tanstack/react-query";
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

/** After a reconnect, lists are read again at a random moment within this long. */
const REREAD_SPREAD_MS = 2_000;

/**
 * Re-reads the lists under `root` that are on screen (queries with an
 * observer), once after every reconnect Welcome (not the first one), at a
 * random moment within two seconds so that a restarted gateway's audience
 * does not read again all at once. Pushes about them that came while the
 * connection was down are gone. `covered` leaves out queries a topic
 * subscription already reads again itself (`SubscribeOptions.onSubscribed`).
 * Returns a function that stops it.
 */
export function rereadListsAfterReconnect(
  queryClient: QueryClient,
  root: QueryKey,
  covered: (queryKey: QueryKey) => boolean = () => false,
): () => void {
  const client = realtime();
  let connectedBefore = client.connectionStatus === "connected";
  let timer: ReturnType<typeof setTimeout> | undefined;
  const off = client.onFrame((frame) => {
    if (frame.body.case !== "welcome") return;
    const again = connectedBefore;
    connectedBefore = true;
    if (!again) return;
    clearTimeout(timer);
    timer = setTimeout(() => {
      void queryClient.invalidateQueries({
        queryKey: root,
        refetchType: "active",
        predicate: (query) => query.getObserversCount() > 0 && !covered(query.queryKey),
      });
    }, Math.random() * REREAD_SPREAD_MS);
  });
  return () => {
    off();
    clearTimeout(timer);
  };
}
