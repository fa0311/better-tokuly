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

/**
 * The reasons that mean access to a topic is over. Anything else the server
 * may say about a topic (`TOPIC_CHECK_FAILED`: its guard could not decide
 * right now) is not a revocation: the topic stays wanted and is asked for
 * again on the next connection.
 */
const REVOCATION_REASONS: ReadonlySet<string> = new Set(["TOPIC_FORBIDDEN", "TOPIC_CLOSED"]);
type RevokedListener = (topic: string, reason: string) => void;

export interface SubscribeOptions {
  /**
   * The server ended this subscription: the viewer lost access (banned,
   * removed, blocked, unfriended, left, the audience was narrowed) or the
   * topic is gone. `reason` is `TOPIC_FORBIDDEN` or `TOPIC_CLOSED`. The
   * topic is not subscribed again by itself; a feature typically reloads
   * what it shows, which then answers as it would for a stranger.
   */
  onRevoked?(reason: string): void;
  /**
   * The subscription is in effect: the server confirmed it, and nothing
   * published on the topic from now on is missed. Called once that is so
   * (right after `subscribe()` when the topic is already in effect), and
   * again whenever it is in effect anew after a reconnect, since what was
   * published while the connection was down is gone; that call comes at a
   * random moment within two seconds, so that a whole gateway's audience
   * does not read again in the same instant. A feature that shows
   * state the topic updates reads that state again here: a read that began
   * before this call may have missed a frame published in between.
   */
  onSubscribed?(): void;
}

/** One caller's hold on a topic (see {@link RealtimeClient.subscribe}). */
export interface Subscription {
  /**
   * Settles with the server's answer: resolves once the subscription is in
   * effect (at once when it already is), rejects with a
   * {@link SubscriptionRefused} when the server refused it. Waits across
   * reconnects while the connection is down. `release()` before the answer
   * drops it: it then never settles.
   */
  readonly ready: Promise<void>;
  /** Ends this caller's interest; the last one out unsubscribes. Safe to call twice. */
  release(): void;
}

/** The server refused a subscription (`Subscription.ready` rejects with it). */
export class SubscriptionRefused extends Error {
  constructor(
    readonly topic: string,
    /** `TOPIC_FORBIDDEN`, `TOPIC_CLOSED`, `TOPIC_CHECK_FAILED`, `BUS_UNAVAILABLE`... */
    readonly reason: string,
    /**
     * Access is over (the topic's `onRevoked` was called and it is no longer
     * followed). Otherwise the topic stays wanted and is asked for again on
     * the next connection.
     */
    readonly revoked: boolean,
  ) {
    super(`realtime: subscribing to ${topic} was refused (${reason})`);
    this.name = "SubscriptionRefused";
  }
}

/** The app's interest in one topic: who holds it, whether it is in effect, who waits for what. */
interface TopicInterest {
  readonly topic: string;
  holders: number;
  /** The server confirmed it on the current connection. */
  active: boolean;
  readonly revokedListeners: Set<(reason: string) => void>;
  readonly subscribedListeners: Set<() => void>;
  /** `ready` promises waiting for the answer. */
  waiters: Waiter[];
  /** It was in effect on an earlier connection (see `REREAD_SPREAD_MS`). */
  confirmedBefore: boolean;
  /** A `Subscribe` the server could not serve right now is sent again then. */
  retry: { timer: ReturnType<typeof setTimeout> | null; attempts: number };
}

interface Waiter {
  resolve(): void;
  reject(error: SubscriptionRefused): void;
}

/** Runs a listener; one that throws is reported and does not stop the others. */
function safely(listener: () => void): void {
  try {
    listener();
  } catch (error) {
    console.error("realtime: a listener failed", error);
  }
}
type StatusListener = (status: ConnectionStatus, transport: FrameTransport["kind"] | null) => void;

const CONNECT_TIMEOUT_MS = 5_000;
/**
 * After a reconnect, the followers of a topic read again within this long,
 * at a random moment: a gateway that restarts drops every connection at
 * once, and their reads should not reach the API in the same second. (The
 * first confirmation is told at once: nothing is synchronized about it.)
 */
const REREAD_SPREAD_MS = 2_000;
/** After `SHUTTING_DOWN`, the next connection waits a random moment up to this long. */
const SHUTDOWN_RECONNECT_SPREAD_MS = 10_000;
/** A `Subscribe` the server could not serve right now: first retry, doubling up to the cap. */
const SUBSCRIBE_RETRY_MIN_MS = 1_000;
const SUBSCRIBE_RETRY_MAX_MS = 30_000;
/** The refusals that may go away by themselves, so a held topic is asked for again. */
const RETRIED_CODES: ReadonlySet<string> = new Set(["unavailable", "resource_exhausted"]);
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
  private readonly revokedListeners = new Set<RevokedListener>();
  private outdated = false;
  /** topic → the components that follow it in this app. */
  private readonly topics = new Map<string, TopicInterest>();
  /** `Subscribe` frames waiting for their answer: frame ID → the interest it asked for. */
  private readonly pendingSubscribes = new Map<bigint, TopicInterest>();
  /** A Hello went out on the current connection: new topics are asked for with `Subscribe`. */
  private helloSent = false;
  /** The subscriptions named in the Hello that awaits its Welcome. */
  private helloInterests: TopicInterest[] | null = null;
  /** The gateway said it is going away: everyone it dropped reconnects, spread out. */
  private gatewayShutDown = false;
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

  /**
   * Hears every subscription the server ended (see
   * {@link SubscribeOptions.onRevoked}). Returns an unsubscribe function.
   */
  onRevoked(listener: RevokedListener): () => void {
    this.revokedListeners.add(listener);
    return () => this.revokedListeners.delete(listener);
  }

  /** Whether the app currently follows `topic` (it was subscribed and not revoked). */
  isSubscribed(topic: string): boolean {
    return this.topics.has(topic);
  }

  private setStatus(status: ConnectionStatus): void {
    this.status = status;
    for (const listener of this.statusListeners) safely(() => listener(status, this.transportKind));
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
    // Answers to frames of an earlier connection will not come, and nothing
    // is in effect on the next one until the server says so.
    this.pendingSubscribes.clear();
    this.helloSent = false;
    this.helloInterests = null;
    for (const interest of this.topics.values()) {
      interest.active = false;
      // The Hello asks for every topic again.
      this.stopRetry(interest);
    }
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
      if (generation !== this.generation) return;
      this.sendHello(token);
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

  /**
   * Says hello with every topic the app follows: its Welcome confirms them
   * all, except those the server refuses with `Unsubscribed` before it.
   */
  private sendHello(token: string | null): void {
    const interests = [...this.topics.values()];
    const topics = interests.map((interest) => interest.topic);
    const id = this.sendFrame({
      case: "hello",
      value: create(realtime.HelloSchema, {
        accessToken: token ?? "",
        platform: this.options.platform,
        appVersion: this.options.appVersion,
        deviceId: this.options.deviceId,
        foreground: this.foreground,
        topics,
      }),
    });
    if (id === 0n) return;
    this.helloSent = true;
    this.helloInterests = interests;
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
    let delay = backoff / 2 + Math.random() * (backoff / 2);
    // Everyone on a gateway that shut down reconnects at once otherwise.
    if (this.gatewayShutDown) delay = Math.max(delay, Math.random() * SHUTDOWN_RECONNECT_SPREAD_MS);
    this.gatewayShutDown = false;
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
        this.onSubscribeRefused(frame.replyTo, frame.body.value);
        void this.onServerError(frame.body.value);
        break;
      case "ack":
        this.onSubscribeAnswered(frame.replyTo);
        break;
      case "unsubscribed":
        this.revoke(frame.body.value.topic, frame.body.value.reason);
        break;
      default:
        break;
    }
    for (const listener of this.frameListeners) safely(() => listener(frame));
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
      for (const listener of this.outdatedListeners) safely(listener);
      this.outdatedListeners.clear();
    }
    // Every subscription of the Hello that the server did not refuse before this Welcome is in
    // effect, unless it was let go meanwhile (a topic followed anew since waits for its own Ack).
    const confirmed = this.helloInterests ?? [];
    this.helloInterests = null;
    for (const interest of confirmed) {
      if (this.topics.get(interest.topic) === interest && !interest.active) this.activate(interest);
    }
    const intervalMs = Math.max(5, welcome.pingIntervalSeconds || 25) * 1000;
    if (this.pingTimer) clearInterval(this.pingTimer);
    this.pingTimer = setInterval(() => {
      // Two missed pongs mean the path is dead even if the socket has not noticed.
      if (Date.now() - this.lastPongAt > intervalMs * 2.5) {
        this.transport?.close();
        return;
      }
      this.sendFrame(
        { case: "ping", value: create(realtime.PingSchema, { nonce: BigInt(Date.now()) }) },
        false,
      );
    }, intervalMs);
    this.setStatus("connected");
  }

  /** The `Ack` of a `Subscribe`: its topic is in effect, unless it was let go meanwhile. */
  private onSubscribeAnswered(replyTo: bigint): void {
    const interest = this.pendingSubscribes.get(replyTo);
    if (!interest) return;
    this.pendingSubscribes.delete(replyTo);
    if (this.topics.get(interest.topic) === interest && !interest.active) this.activate(interest);
  }

  /** A `Subscribe` refused because the guard said no ends the interest the same way. */
  private onSubscribeRefused(replyTo: bigint, error: realtime.Error): void {
    const interest = this.pendingSubscribes.get(replyTo);
    if (!interest) return;
    this.pendingSubscribes.delete(replyTo);
    if (this.topics.get(interest.topic) !== interest) return;
    if (error.code === "permission_denied") this.revoke(interest.topic, error.message);
    if (this.topics.get(interest.topic) !== interest) return;
    // Any other refusal (the guard could not decide, the bus was away) is
    // not the end of access: the topic stays wanted, is asked for again
    // shortly when that may help, and with the next Hello anyway. Whoever
    // waits hears the refusal now.
    this.settle(interest, new SubscriptionRefused(interest.topic, error.message, false));
    if (RETRIED_CODES.has(error.code)) this.retryLater(interest);
  }

  /** Sends the `Subscribe` of a held topic again after a backoff. */
  private retryLater(interest: TopicInterest): void {
    if (interest.retry.timer) return;
    const backoff = Math.min(
      SUBSCRIBE_RETRY_MAX_MS,
      SUBSCRIBE_RETRY_MIN_MS * 2 ** interest.retry.attempts,
    );
    interest.retry.attempts++;
    const generation = this.generation;
    interest.retry.timer = setTimeout(
      () => {
        interest.retry.timer = null;
        if (generation !== this.generation || !this.helloSent) return;
        if (this.topics.get(interest.topic) !== interest || interest.active) return;
        this.sendSubscribe(interest);
      },
      backoff / 2 + Math.random() * (backoff / 2),
    );
  }

  private stopRetry(interest: TopicInterest): void {
    if (interest.retry.timer) clearTimeout(interest.retry.timer);
    interest.retry.timer = null;
  }

  private sendSubscribe(interest: TopicInterest): void {
    const id = this.sendFrame({
      case: "subscribe",
      value: create(realtime.SubscribeSchema, { topics: [interest.topic] }),
    });
    if (id !== 0n) this.pendingSubscribes.set(id, interest);
  }

  /** The server confirmed the topic: waiting callers resolve, followers read again. */
  private activate(interest: TopicInterest): void {
    interest.active = true;
    interest.retry.attempts = 0;
    this.stopRetry(interest);
    this.settle(interest);
    const again = interest.confirmedBefore;
    interest.confirmedBefore = true;
    const tell = () => {
      for (const listener of [...interest.subscribedListeners]) {
        // Let go of, or out of effect again, by the time its turn came.
        if (!interest.active || !interest.subscribedListeners.has(listener)) continue;
        safely(listener);
      }
    };
    if (again) setTimeout(tell, Math.random() * REREAD_SPREAD_MS);
    else tell();
  }

  /** Resolves (or, given a refusal, rejects) every `ready` waiting on the interest. */
  private settle(interest: TopicInterest, refusal?: SubscriptionRefused): void {
    const waiters = interest.waiters;
    interest.waiters = [];
    for (const waiter of waiters)
      safely(() => (refusal ? waiter.reject(refusal) : waiter.resolve()));
  }

  /**
   * The server will send nothing more of `topic`: the app stops treating it
   * as live (it is not named in the next Hello either, so a reconnect does
   * not ask for it again) and its followers are told once.
   */
  private revoke(topic: string, reason: string): void {
    if (!REVOCATION_REASONS.has(reason)) return;
    const interest = this.topics.get(topic);
    if (!interest) return;
    this.topics.delete(topic);
    this.stopRetry(interest);
    console.info(`realtime: the server ended the subscription to ${topic} (${reason})`);
    this.settle(interest, new SubscriptionRefused(topic, reason, true));
    for (const listener of [...interest.revokedListeners]) safely(() => listener(reason));
    for (const listener of [...this.revokedListeners]) safely(() => listener(topic, reason));
  }

  private async onServerError(error: realtime.Error): Promise<void> {
    if (error.message === "SHUTTING_DOWN") this.gatewayShutDown = true;
    if (error.code !== "unauthenticated") return;
    // The token expired or was rejected: refresh and say hello again.
    const refreshed = (await this.options.refreshAccessToken?.()) ?? false;
    if (refreshed && this.transport && !error.fatal) {
      const token = await this.options.getAccessToken();
      this.sendHello(token);
    } else if (refreshed) {
      this.restart();
    }
  }

  /** Sends a frame; returns its ID (0 when none was asked for or nothing was sent). */
  private sendFrame(body: realtime.ClientFrame["body"], withId = true, unreliable = false): bigint {
    const transport = this.transport;
    if (!transport) return 0n;
    const frame = create(realtime.ClientFrameSchema, {
      id: withId ? this.nextFrameId++ : 0n,
      body,
    });
    const bytes = toBinary(realtime.ClientFrameSchema, frame);
    if (unreliable) transport.sendUnreliable(bytes);
    else transport.send(bytes);
    return frame.id;
  }

  /**
   * Subscribes to a topic (e.g. `live:<id>`). Subscriptions are counted, so
   * several components may share one topic; `release()` ends this caller's
   * interest. `ready` (and `options.onSubscribed`) say when the server has
   * the subscription in place, which is when a feature reads the state the
   * topic updates. When the server ends the subscription, `options.onRevoked`
   * is called and the topic is not asked for again until somebody subscribes
   * anew.
   */
  subscribe(topic: string, options: SubscribeOptions = {}): Subscription {
    let interest = this.topics.get(topic);
    if (!interest) {
      interest = {
        topic,
        holders: 0,
        active: false,
        revokedListeners: new Set(),
        subscribedListeners: new Set(),
        waiters: [],
        confirmedBefore: false,
        retry: { timer: null, attempts: 0 },
      };
      this.topics.set(topic, interest);
      // Before the Hello, the topic goes out with it; after, it is asked for on its own.
      if (this.helloSent) this.sendSubscribe(interest);
    }
    const held = interest;
    held.holders++;
    // Wrapped, so that one function given by two callers counts as two followers.
    const onRevoked = options.onRevoked && ((reason: string) => options.onRevoked?.(reason));
    const onSubscribed = options.onSubscribed && (() => options.onSubscribed?.());
    if (onRevoked) held.revokedListeners.add(onRevoked);
    let ready: Promise<void>;
    let waiter: Waiter | undefined;
    if (held.active) {
      ready = Promise.resolve();
    } else {
      ready = new Promise<void>((resolve, reject) => {
        waiter = { resolve, reject };
        held.waiters.push(waiter);
      });
      // A caller that does not wait for the answer must not see an unhandled rejection.
      ready.catch(() => {});
    }
    if (onSubscribed) {
      held.subscribedListeners.add(onSubscribed);
      // Already in effect: this caller hears so too, once its own setup is done.
      if (held.active)
        queueMicrotask(() => {
          if (held.active && held.subscribedListeners.has(onSubscribed)) onSubscribed();
        });
    }
    let released = false;
    const release = () => {
      if (released) return;
      released = true;
      if (onRevoked) held.revokedListeners.delete(onRevoked);
      if (onSubscribed) held.subscribedListeners.delete(onSubscribed);
      if (waiter) held.waiters = held.waiters.filter((other) => other !== waiter);
      // Revoked meanwhile (and maybe subscribed anew by someone else): nothing of ours is left.
      if (this.topics.get(topic) !== held) return;
      held.holders--;
      if (held.holders > 0) return;
      this.topics.delete(topic);
      this.stopRetry(held);
      this.sendFrame({
        case: "unsubscribe",
        value: create(realtime.UnsubscribeSchema, { topics: [topic] }),
      });
    };
    return { ready, release };
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
    this.sendFrame(
      { case: "typing", value: create(realtime.TypingSchema, { conversationId, typing }) },
      false,
    );
  }

  sendConversationPresence(conversationId: string, present: boolean): void {
    this.sendFrame({
      case: "conversationPresence",
      value: create(realtime.ConversationPresenceSchema, { conversationId, present }),
    });
  }
}
