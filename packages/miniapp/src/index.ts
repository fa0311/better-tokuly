/**
 * The SDK a mini app runs inside Tokuly.
 *
 *   import { connect } from "@tokuly/miniapp";
 *   const tokuly = await connect({ hostOrigins: ["https://tokuly.app"] });
 *   const me = await tokuly.getProfile();          // asks the person once
 *   const friends = await tokuly.pickFriends({ max: 5 });
 *   await tokuly.shareCard({ title: "割り勘", text: "1人 ¥1,200", path: "/?total=3600" });
 *
 * It talks only to the Tokuly that framed it: on the web, the parent window
 * whose origin is one of `hostOrigins` (read from `document.referrer`, which
 * Tokuly sends as its origin), and only messages from that window and
 * origin are read; in the mobile app, the native bridge of its WebView.
 */
import {
  type Call,
  type ErrorCode,
  type Launch,
  type Method,
  type Person,
  PROTOCOL,
  type Request,
  type Results,
  readResponse,
} from "./protocol.ts";

export type { Card, ErrorCode, Launch, Person, Scope } from "./protocol.ts";

/** A call Tokuly refused, or one that could not be made. */
export class BridgeError extends Error {
  readonly code: ErrorCode | "NOT_IN_TOKULY" | "TIMEOUT";

  constructor(code: BridgeError["code"], message: string) {
    super(message);
    this.code = code;
    this.name = "BridgeError";
  }
}

export interface ConnectOptions {
  /** The Tokuly origins this mini app accepts as its host, e.g. `["https://tokuly.app"]`. */
  hostOrigins: string[];
  /** How long to wait for an answer that needs no person (default 10 s). */
  timeoutMs?: number;
}

export interface MiniApp {
  launch: Launch;
  /** Who the person is. Needs the `profile` scope; Tokuly asks the person the first time. */
  getProfile(): Promise<Person>;
  /** Friends the person picks, each time. Needs the `friends` scope. */
  pickFriends(options?: { max?: number }): Promise<Person[]>;
  /** Shares a card to the conversation the mini app was opened from; the person confirms it. */
  shareCard(card: { title: string; text?: string; path?: string }): Promise<void>;
  /** Asks Tokuly to close the mini app. */
  close(): void;
}

/** What the SDK needs of the page it runs in; the browser's by default. */
export interface Environment {
  /** Sends a request to the host. */
  send(request: Request): void;
  /** Calls `receive` with every message from the host, and nothing else. */
  listen(receive: (data: unknown) => void): void;
}

interface NativeBridge {
  postMessage(message: string): void;
}

/** The page's way to Tokuly: the native bridge in the mobile app, the parent window on the web. */
export function browserEnvironment(hostOrigins: string[]): Environment {
  const native = (window as unknown as { ReactNativeWebView?: NativeBridge }).ReactNativeWebView;
  if (native)
    return {
      send: (request) => native.postMessage(JSON.stringify(request)),
      // The app hands answers to the page itself: no window sent them.
      listen: (receive) =>
        window.addEventListener("message", (event) => {
          if (event.source === null && event.origin === "") receive(event.data);
        }),
    };
  const host = originOf(document.referrer);
  if (window.parent === window || !host || !hostOrigins.includes(host))
    throw new BridgeError("NOT_IN_TOKULY", "this page is not open inside Tokuly");
  return {
    send: (request) => window.parent.postMessage(request, host),
    listen: (receive) =>
      window.addEventListener("message", (event) => {
        if (event.source === window.parent && event.origin === host) receive(event.data);
      }),
  };
}

function originOf(url: string): string | undefined {
  try {
    return new URL(url).origin;
  } catch {
    return undefined;
  }
}

/** Starts talking to Tokuly; resolves once Tokuly answered with the launch details. */
export async function connect(
  options: ConnectOptions,
  environment: Environment = browserEnvironment(options.hostOrigins),
): Promise<MiniApp> {
  const timeoutMs = options.timeoutMs ?? 10_000;
  const waiting = new Map<string, { resolve(result: unknown): void; reject(error: Error): void }>();
  environment.listen((data) => {
    const response = readResponse(data);
    const pending = response && waiting.get(response.id);
    if (!response || !pending) return;
    waiting.delete(response.id);
    if (response.ok) pending.resolve(response.result);
    else pending.reject(new BridgeError(response.error.code, response.error.message));
  });

  let counter = 0;
  /** Calls that wait for the person (consent, picker, share sheet) are not timed. */
  const call = <M extends Method>(request: Extract<Call, { method: M }>, timed: boolean) =>
    new Promise<Results[M]>((resolve, reject) => {
      counter += 1;
      const id = `${Date.now().toString(36)}-${counter}`;
      const timer = timed
        ? setTimeout(() => {
            waiting.delete(id);
            reject(new BridgeError("TIMEOUT", `${request.method} got no answer`));
          }, timeoutMs)
        : undefined;
      waiting.set(id, {
        resolve: (result) => {
          clearTimeout(timer);
          resolve(result as Results[M]);
        },
        reject: (error) => {
          clearTimeout(timer);
          reject(error);
        },
      });
      environment.send({ protocol: PROTOCOL, id, call: request });
    });

  const launch = await call({ method: "hello" }, true);
  return {
    launch,
    getProfile: () => call({ method: "getProfile" }, false),
    pickFriends: ({ max = 20 } = {}) => call({ method: "pickFriends", max }, false),
    shareCard: async ({ title, text = "", path = "/" }) => {
      await call({ method: "shareCard", card: { title, text, path } }, false);
    },
    close: () => void call({ method: "close" }, true).catch(() => undefined),
  };
}
