import { create, toBinary } from "@bufbuild/protobuf";
import { type common, geo, realtime } from "@tokuly/api";
import { decodeServerFrame, type ServerFrame } from "./events.js";
import { clientIsOutdated } from "./protocol.js";
import {
  connectWebSocket,
  connectWebTransport,
  type Endpoints,
  type FrameTransport,
  webTransportSupported,
} from "./transport.js";

export type ConnectionStatus = "idle" | "connecting" | "connected" | "reconnecting";

export interface RealtimeOptions {
  /** Looks up where to connect (RealtimeService.GetEndpoints). */
  getEndpoints(): Promise<Endpoints>;
  /** A valid access token, or `null` to connect anonymously. */
  getAccessToken(): Promise<string | null>;
  /** Called when the server rejects the token; should refresh the session. */
  refreshAccessToken?(): Promise<boolean>;
  platform: common.Platform;
  appVersion: string;
  deviceId: string;
  /** Set false to force the WebSocket path (e.g. React Native). */
  webTransport?: boolean;
}

type FrameListener = (frame: ServerFrame) => void;
type StatusListener = (status: ConnectionStatus, transport: FrameTransport["kind"] | null) => void;

const CONNECT_TIMEOUT_MS = 5_000;
const MAX_BACKOFF_MS = 30_000;
/** After a WebTransport failure, stay on WebSocket for this long before trying again. */
const WEBTRANSPORT_RETRY_AFTER_MS = 5 * 60_000;

/**
 * One connection to the realtime gateway for the whole app.
 *
 * It prefers WebTransport (QUIC) and falls back to WebSocket, reconnects with
 * backoff, re-authenticates when the token expires and re-subscribes topics
 * after every reconnect. Consumers only ever see `ServerFrame`s, in which a
 * domain's event is already decoded (`frame.body.case === "chat"`).
 */
export class RealtimeClient {
  private transport: FrameTransport | null = null;
  private status: ConnectionStatus = "idle";
  private wanted = false;
  private attempt = 0;
  private generation = 0;
  private nextFrameId = 1n;
  private pingTimer: ReturnType<typeof setInterval> | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private lastPongAt = 0;
  private webTransportBlockedUntil = 0;
  private foreground = true;
  private readonly frameListeners = new Set<FrameListener>();
  private readonly statusListeners = new Set<StatusListener>();
  private readonly outdatedListeners = new Set<() => void>();
  private outdated = false;
  /** topic → number of active subscribers in this app. */
  private readonly topics = new Map<string, number>();
  /** Server-suggested location report cadence, from the last Welcome. */
  cadence = { movingSeconds: 5, stationarySeconds: 60 };

  constructor(private readonly options: RealtimeOptions) {}

  get connectionStatus(): ConnectionStatus {
    return this.status;
  }

  get transportKind(): FrameTransport["kind"] | null {
    return this.transport?.kind ?? null;
  }

  /** Listens to every frame pushed by the server. Returns an unsubscribe function. */
  onFrame(listener: FrameListener): () => void {
    this.frameListeners.add(listener);
    return () => this.frameListeners.delete(listener);
  }

  onStatus(listener: StatusListener): () => void {
    this.statusListeners.add(listener);
    return () => this.statusListeners.delete(listener);
  }

  /**
   * Tells the app that the server speaks a newer protocol than this client
   * was built for: the page has to be loaded again. Called at most once per
   * listener, right away if it is already known.
   */
  onOutdated(listener: () => void): () => void {
    if (this.outdated) listener();
    else this.outdatedListeners.add(listener);
    return () => this.outdatedListeners.delete(listener);
  }

  private setStatus(status: ConnectionStatus): void {
    this.status = status;
    for (const listener of this.statusListeners) listener(status, this.transportKind);
  }

  /** Starts (and keeps) the connection. Safe to call repeatedly. */
  start(): void {
    if (this.wanted) return;
    this.wanted = true;
    void this.connect();
  }

  /** Closes the connection and stops reconnecting. */
  stop(): void {
    this.wanted = false;
    this.generation++;
    this.clearTimers();
    this.transport?.close();
    this.transport = null;
    this.setStatus("idle");
  }

  /** Drops the current connection so the next one authenticates afresh (after sign-in/out). */
  restart(): void {
    if (!this.wanted) return;
    this.generation++;
    this.clearTimers();
    this.transport?.close();
    this.transport = null;
    this.attempt = 0;
    void this.connect();
  }

  private clearTimers(): void {
    if (this.pingTimer) clearInterval(this.pingTimer);
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.pingTimer = null;
    this.reconnectTimer = null;
  }

  private async connect(): Promise<void> {
    const generation = ++this.generation;
    this.setStatus(this.attempt === 0 ? "connecting" : "reconnecting");
    try {
      const endpoints = await this.options.getEndpoints();
      if (generation !== this.generation) return;
      const handlers = {
        onFrame: (bytes: Uint8Array) => {
          if (generation === this.generation) this.handleFrame(bytes);
        },
        onClose: () => {
          if (generation === this.generation) this.handleClose();
        },
      };
      const transport = await this.openTransport(endpoints, handlers);
      if (generation !== this.generation) {
        transport.close();
        return;
      }
      this.transport = transport;
      const token = await this.options.getAccessToken();
      this.sendFrame({
        case: "hello",
        value: create(realtime.HelloSchema, {
          accessToken: token ?? "",
          platform: this.options.platform,
          appVersion: this.options.appVersion,
          deviceId: this.options.deviceId,
          foreground: this.foreground,
          topics: [...this.topics.keys()],
        }),
      });
    } catch {
      if (generation === this.generation) this.scheduleReconnect();
    }
  }

  private async openTransport(
    endpoints: Endpoints,
    handlers: { onFrame(frame: Uint8Array): void; onClose(reason: string): void },
  ): Promise<FrameTransport> {
    const tryWebTransport =
      this.options.webTransport !== false &&
      webTransportSupported() &&
      endpoints.webtransportUrl !== "" &&
      Date.now() >= this.webTransportBlockedUntil;
    if (tryWebTransport) {
      try {
        return await connectWebTransport(endpoints, handlers, CONNECT_TIMEOUT_MS);
      } catch {
        // UDP is blocked on some networks; do not pay the timeout on every reconnect.
        this.webTransportBlockedUntil = Date.now() + WEBTRANSPORT_RETRY_AFTER_MS;
      }
    }
    return connectWebSocket(endpoints, handlers, CONNECT_TIMEOUT_MS);
  }

  private handleClose(): void {
    this.transport = null;
    if (this.pingTimer) clearInterval(this.pingTimer);
    this.pingTimer = null;
    if (this.wanted) this.scheduleReconnect();
  }

  private scheduleReconnect(): void {
    this.setStatus("reconnecting");
    const backoff = Math.min(MAX_BACKOFF_MS, 500 * 2 ** this.attempt);
    const delay = backoff / 2 + Math.random() * (backoff / 2);
    this.attempt++;
    this.reconnectTimer = setTimeout(() => void this.connect(), delay);
  }

  private handleFrame(bytes: Uint8Array): void {
    // Something a newer server sent that this build cannot read is ignored.
    const frame = decodeServerFrame(bytes);
    if (!frame) return;
    switch (frame.body.case) {
      case "welcome":
        this.onWelcome(frame.body.value);
        break;
      case "pong":
        this.lastPongAt = Date.now();
        break;
      case "error":
        void this.onServerError(frame.body.value);
        break;
      default:
        break;
    }
    for (const listener of this.frameListeners) listener(frame);
  }

  private onWelcome(welcome: realtime.Welcome): void {
    this.attempt = 0;
    this.lastPongAt = Date.now();
    this.cadence = {
      movingSeconds: welcome.movingIntervalSeconds || 5,
      stationarySeconds: welcome.stationaryIntervalSeconds || 60,
    };
    if (!this.outdated && clientIsOutdated(welcome.protocolVersion)) {
      this.outdated = true;
      for (const listener of this.outdatedListeners) listener();
      this.outdatedListeners.clear();
    }
    const intervalMs = Math.max(5, welcome.pingIntervalSeconds || 25) * 1000;
    if (this.pingTimer) clearInterval(this.pingTimer);
    this.pingTimer = setInterval(() => {
      // Two missed pongs mean the path is dead even if the socket has not noticed.
      if (Date.now() - this.lastPongAt > intervalMs * 2.5) {
        this.transport?.close();
        return;
      }
      this.sendFrame({ case: "ping", value: create(realtime.PingSchema, { nonce: BigInt(Date.now()) }) }, false);
    }, intervalMs);
    this.setStatus("connected");
  }

  private async onServerError(error: realtime.Error): Promise<void> {
    if (error.code !== "unauthenticated") return;
    // The token expired or was rejected: refresh and say hello again.
    const refreshed = (await this.options.refreshAccessToken?.()) ?? false;
    if (refreshed && this.transport && !error.fatal) {
      const token = await this.options.getAccessToken();
      this.sendFrame({
        case: "hello",
        value: create(realtime.HelloSchema, {
          accessToken: token ?? "",
          platform: this.options.platform,
          appVersion: this.options.appVersion,
          deviceId: this.options.deviceId,
          foreground: this.foreground,
          topics: [...this.topics.keys()],
        }),
      });
    } else if (refreshed) {
      this.restart();
    }
  }

  private sendFrame(body: realtime.ClientFrame["body"], withId = true, unreliable = false): void {
    const transport = this.transport;
    if (!transport) return;
    const frame = create(realtime.ClientFrameSchema, { id: withId ? this.nextFrameId++ : 0n, body });
    const bytes = toBinary(realtime.ClientFrameSchema, frame);
    if (unreliable) transport.sendUnreliable(bytes);
    else transport.send(bytes);
  }

  /**
   * Subscribes to a topic (e.g. `live:<id>`). Subscriptions are counted, so
   * several components may share one topic; the returned function releases
   * this caller's interest.
   */
  subscribe(topic: string): () => void {
    const count = this.topics.get(topic) ?? 0;
    this.topics.set(topic, count + 1);
    if (count === 0) {
      this.sendFrame({ case: "subscribe", value: create(realtime.SubscribeSchema, { topics: [topic] }) });
    }
    let released = false;
    return () => {
      if (released) return;
      released = true;
      const remaining = (this.topics.get(topic) ?? 1) - 1;
      if (remaining > 0) {
        this.topics.set(topic, remaining);
      } else {
        this.topics.delete(topic);
        this.sendFrame({ case: "unsubscribe", value: create(realtime.UnsubscribeSchema, { topics: [topic] }) });
      }
    };
  }

  /** Sends a location sample; dropped silently while disconnected. */
  sendLocation(sample: geo.LocationSample): boolean {
    if (!this.transport || this.status !== "connected") return false;
    // The frame carries the sample encoded: the gateway's protocol does not know the geo messages.
    const encoded = toBinary(geo.LocationSampleSchema, sample);
    this.sendFrame({ case: "location", value: encoded }, false, true);
    return true;
  }

  /** Tells the server whether the app is visible (drives the online halo). */
  setForeground(foreground: boolean): void {
    if (this.foreground === foreground) return;
    this.foreground = foreground;
    this.sendFrame({ case: "appState", value: create(realtime.AppStateSchema, { foreground }) });
  }

  sendTyping(conversationId: string, typing: boolean): void {
    this.sendFrame({ case: "typing", value: create(realtime.TypingSchema, { conversationId, typing }) }, false);
  }

  sendConversationPresence(conversationId: string, present: boolean): void {
    this.sendFrame({
      case: "conversationPresence",
      value: create(realtime.ConversationPresenceSchema, { conversationId, present }),
    });
  }
}
