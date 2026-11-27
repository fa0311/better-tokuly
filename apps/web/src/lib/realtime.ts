import { common, realtime as rt } from "@tokuly/api";
import { RealtimeClient, type ServerFrame, type ServerFrameBody } from "@tokuly/realtime";
import { api } from "#/lib/api";
import { restartWhenSessionChanges } from "#/lib/realtime-identity";

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
    const connection = client;
    restartWhenSessionChanges(api().session, () => connection.restart());
    document.addEventListener("visibilitychange", () => {
      client?.setForeground(document.visibilityState === "visible");
    });
    // Development aid: inspect the connection from the browser console.
    if (import.meta.env.DEV)
      (window as unknown as { __tokulyRealtime?: RealtimeClient }).__tokulyRealtime = client;
  }
  return client;
}

type FrameCase = NonNullable<ServerFrameBody["case"]>;
type FrameValue<Case extends FrameCase> = Extract<ServerFrameBody, { case: Case }>["value"];

/**
 * Hands the frames of one case (`"wallet"`, `"social"`) to what `load` returns, loaded with the
 * first of them: what reacts to a domain's events (its cache code, its API client, its schemas)
 * stays off the first page. Frames that arrive meanwhile wait and follow in the order they came;
 * a load that fails loses them and is tried again with the next frame. Returns the unsubscribe.
 */
export function onFramesLoading<Case extends FrameCase>(
  kind: Case,
  load: () => Promise<(value: FrameValue<Case>, frame: ServerFrame) => void>,
): () => void {
  type Handle = (value: FrameValue<Case>, frame: ServerFrame) => void;
  let handle: Handle | null = null;
  let waiting: ServerFrame[] | null = null;
  let stopped = false;
  const deliver = (to: Handle, frame: ServerFrame) =>
    to(frame.body.value as FrameValue<Case>, frame);
  const off = realtime().onFrame((frame) => {
    if (frame.body.case !== kind) return;
    if (handle) return deliver(handle, frame);
    if (waiting) return void waiting.push(frame);
    const queue = [frame];
    waiting = queue;
    load().then(
      (loaded) => {
        waiting = null;
        if (stopped) return;
        handle = loaded;
        for (const queued of queue) deliver(loaded, queued);
      },
      () => {
        waiting = null;
      },
    );
  });
  return () => {
    stopped = true;
    off();
  };
}
