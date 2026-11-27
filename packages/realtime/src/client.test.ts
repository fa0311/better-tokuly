import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
// Not the barrel: it would load every domain's schemas, and this file holds chat's back.
import * as common from "@tokuly/api/gen/tokuly/common/v1/common_pb.js";
import * as realtime from "@tokuly/api/gen/tokuly/realtime/v1/realtime_pb.js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/** What the client sent, decoded, and a way to answer as the gateway. */
const wire = vi.hoisted(() => ({
  sent: [] as Uint8Array[],
  deliver: null as ((frame: Uint8Array) => void) | null,
  close: null as ((reason: string) => void) | null,
  /** How often the client closed a connection itself. */
  closes: 0,
}));

vi.mock("./transport.js", () => ({
  webTransportSupported: () => false,
  connectWebTransport: () => Promise.reject(new Error("not in tests")),
  connectWebSocket: (
    _endpoints: unknown,
    handlers: { onFrame(frame: Uint8Array): void; onClose(reason: string): void },
  ) => {
    wire.deliver = handlers.onFrame;
    wire.close = handlers.onClose;
    return Promise.resolve({
      kind: "websocket",
      send: (frame: Uint8Array) => wire.sent.push(frame),
      sendUnreliable: (frame: Uint8Array) => wire.sent.push(frame),
      close: () => {
        wire.closes++;
      },
    });
  },
}));

/** Chat's schemas load when the test says so, like a chunk on a slow network. */
const chatSchemas = vi.hoisted(() => {
  let arrive = () => {};
  const arrived = new Promise<void>((resolve) => {
    arrive = resolve;
  });
  return { arrived, arrive };
});
vi.mock("@tokuly/api/gen/tokuly/chat/v1/chat_pb.js", async (importOriginal) => {
  await chatSchemas.arrived;
  return importOriginal();
});

const { RealtimeClient, SubscriptionRefused } = await import("./client.js");

function sentFrames(): realtime.ClientFrame[] {
  return wire.sent.map((bytes) => fromBinary(realtime.ClientFrameSchema, bytes));
}

function serverSends(frame: Parameters<typeof create<typeof realtime.ServerFrameSchema>>[1]): void {
  wire.deliver?.(toBinary(realtime.ServerFrameSchema, create(realtime.ServerFrameSchema, frame)));
}

function newClient() {
  return new RealtimeClient({
    getEndpoints: async () => ({
      webtransportUrl: "",
      webtransportCertHashes: [],
      websocketUrl: "ws://test",
    }),
    getAccessToken: async () => "token",
    platform: common.Platform.WEB,
    appVersion: "test",
    deviceId: "device",
  });
}

/** The next Hello the client sends (waiting for the connection to be made). */
async function nextHello(): Promise<realtime.ClientFrame & { body: { case: "hello" } }> {
  await vi.waitFor(
    () => expect(sentFrames().some((frame) => frame.body.case === "hello")).toBe(true),
    { timeout: 3_000 },
  );
  return sentFrames().find((frame) => frame.body.case === "hello") as realtime.ClientFrame & {
    body: { case: "hello" };
  };
}

function welcome(replyTo: bigint): void {
  serverSends({ replyTo, body: { case: "welcome", value: { protocolVersion: 2 } } });
}

async function connectedClient() {
  const client = newClient();
  client.start();
  welcome((await nextHello()).id);
  wire.sent.length = 0;
  return client;
}

/** Whether the promise has settled, and how, without waiting for it. */
async function stateOf(promise: Promise<unknown>): Promise<"pending" | "resolved" | "rejected"> {
  const pending = Symbol("pending");
  try {
    const value = await Promise.race([promise, Promise.resolve().then(() => pending)]);
    return value === pending ? "pending" : "resolved";
  } catch {
    return "rejected";
  }
}

function subscribeFrames(topic: string): realtime.ClientFrame[] {
  return sentFrames().filter(
    (frame) => frame.body.case === "subscribe" && frame.body.value.topics.includes(topic),
  );
}

describe("subscriptions the server ends", () => {
  beforeEach(() => {
    wire.sent.length = 0;
    vi.spyOn(console, "info").mockImplementation(() => {});
  });

  it("tells every follower once and stops treating the topic as live", async () => {
    const client = await connectedClient();
    const first = vi.fn();
    const second = vi.fn();
    const everywhere = vi.fn();
    client.onRevoked(everywhere);
    const { release: releaseFirst } = client.subscribe("space:1", { onRevoked: first });
    const { release: releaseSecond } = client.subscribe("space:1", { onRevoked: second });
    expect(subscribeFrames("space:1")).toHaveLength(1);

    serverSends({
      topic: "space:1",
      body: { case: "unsubscribed", value: { topic: "space:1", reason: "TOPIC_FORBIDDEN" } },
    });
    expect(first).toHaveBeenCalledWith("TOPIC_FORBIDDEN");
    expect(second).toHaveBeenCalledWith("TOPIC_FORBIDDEN");
    expect(everywhere).toHaveBeenCalledWith("space:1", "TOPIC_FORBIDDEN");
    expect(client.isSubscribed("space:1")).toBe(false);

    // Releasing afterwards sends nothing: the server dropped it already.
    releaseFirst();
    releaseSecond();
    expect(sentFrames().filter((frame) => frame.body.case === "unsubscribe")).toHaveLength(0);
    // Hearing it twice changes nothing.
    serverSends({
      topic: "space:1",
      body: { case: "unsubscribed", value: { topic: "space:1", reason: "TOPIC_FORBIDDEN" } },
    });
    expect(first).toHaveBeenCalledTimes(1);
  });

  it("does not ask for a revoked topic again on reconnect", async () => {
    const client = await connectedClient();
    client.subscribe("community:1");
    client.subscribe("live:2");
    serverSends({
      topic: "community:1",
      body: { case: "unsubscribed", value: { topic: "community:1", reason: "TOPIC_FORBIDDEN" } },
    });
    wire.close?.("gone");
    await vi.waitFor(
      () => expect(sentFrames().some((frame) => frame.body.case === "hello")).toBe(true),
      { timeout: 3_000 },
    );
    const hello = sentFrames().find((frame) => frame.body.case === "hello");
    expect(hello?.body.case === "hello" && hello.body.value.topics).toEqual(["live:2"]);
  });

  it("treats a refused subscribe like a revocation, without trying again", async () => {
    const client = await connectedClient();
    const revoked = vi.fn();
    client.subscribe("study:9", { onRevoked: revoked });
    const [subscribe] = subscribeFrames("study:9");
    serverSends({
      replyTo: subscribe?.id ?? 0n,
      body: {
        case: "error",
        value: { code: "permission_denied", message: "TOPIC_FORBIDDEN", fatal: false },
      },
    });
    expect(revoked).toHaveBeenCalledWith("TOPIC_FORBIDDEN");
    expect(client.isSubscribed("study:9")).toBe(false);
    expect(subscribeFrames("study:9")).toHaveLength(1);
  });

  it("lets a component follow the topic anew after a revocation", async () => {
    const client = await connectedClient();
    const { release: releaseOld } = client.subscribe("live:3");
    serverSends({
      topic: "live:3",
      body: { case: "unsubscribed", value: { topic: "live:3", reason: "TOPIC_CLOSED" } },
    });
    const { release: releaseNew } = client.subscribe("live:3");
    expect(subscribeFrames("live:3")).toHaveLength(2);
    // The old holder's release must not end the new interest.
    releaseOld();
    expect(client.isSubscribed("live:3")).toBe(true);
    releaseNew();
    expect(client.isSubscribed("live:3")).toBe(false);
    expect(sentFrames().filter((frame) => frame.body.case === "unsubscribe")).toHaveLength(1);
  });
});

describe("refusals that are not revocations", () => {
  beforeEach(() => {
    wire.sent.length = 0;
    vi.spyOn(console, "info").mockImplementation(() => {});
  });

  it("keeps a topic whose guard could not decide, and asks for it again on reconnect", async () => {
    const client = await connectedClient();
    const revoked = vi.fn();
    client.subscribe("overlay:k", { onRevoked: revoked });
    const [subscribe] = subscribeFrames("overlay:k");
    serverSends({
      replyTo: subscribe?.id ?? 0n,
      body: {
        case: "error",
        value: { code: "unavailable", message: "TOPIC_CHECK_FAILED", fatal: false },
      },
    });
    // An unknown reason in an Unsubscribed frame is not taken for one either.
    serverSends({
      topic: "overlay:k",
      body: { case: "unsubscribed", value: { topic: "overlay:k", reason: "TOPIC_CHECK_FAILED" } },
    });
    expect(revoked).not.toHaveBeenCalled();
    expect(client.isSubscribed("overlay:k")).toBe(true);
    wire.close?.("gone");
    await vi.waitFor(
      () => expect(sentFrames().some((frame) => frame.body.case === "hello")).toBe(true),
      { timeout: 3_000 },
    );
    const hello = sentFrames().find((frame) => frame.body.case === "hello");
    expect(hello?.body.case === "hello" && hello.body.value.topics).toContain("overlay:k");
  });
});

describe("the answer to a subscription", () => {
  beforeEach(() => {
    wire.sent.length = 0;
    vi.spyOn(console, "info").mockImplementation(() => {});
  });

  it("resolves when the server acknowledges the subscribe, and not before", async () => {
    const client = await connectedClient();
    const subscribed = vi.fn();
    const { ready } = client.subscribe("live:1", { onSubscribed: subscribed });
    const [subscribe] = subscribeFrames("live:1");
    expect(subscribe?.id).not.toBe(0n);
    expect(await stateOf(ready)).toBe("pending");
    expect(subscribed).not.toHaveBeenCalled();

    serverSends({ replyTo: subscribe?.id ?? 0n, body: { case: "ack", value: {} } });
    expect(await stateOf(ready)).toBe("resolved");
    expect(subscribed).toHaveBeenCalledTimes(1);
    // A second follower of a topic in effect is told right away, without another frame.
    const later = vi.fn();
    const second = client.subscribe("live:1", { onSubscribed: later });
    expect(await stateOf(second.ready)).toBe("resolved");
    await Promise.resolve();
    expect(later).toHaveBeenCalledTimes(1);
    expect(subscribed).toHaveBeenCalledTimes(1);
    expect(subscribeFrames("live:1")).toHaveLength(1);
  });

  it("rejects with the guard's reason, as a revocation when access is refused", async () => {
    const client = await connectedClient();
    const revoked = vi.fn();
    const { ready } = client.subscribe("space:2", { onRevoked: revoked });
    const [subscribe] = subscribeFrames("space:2");
    serverSends({
      replyTo: subscribe?.id ?? 0n,
      body: {
        case: "error",
        value: { code: "permission_denied", message: "TOPIC_FORBIDDEN", fatal: false },
      },
    });
    const refusal = await ready.then(
      () => null,
      (error: unknown) => error,
    );
    expect(refusal).toBeInstanceOf(SubscriptionRefused);
    expect(refusal).toMatchObject({ topic: "space:2", reason: "TOPIC_FORBIDDEN", revoked: true });
    expect(revoked).toHaveBeenCalledWith("TOPIC_FORBIDDEN");
  });

  it("rejects without giving the topic up when the guard could not decide", async () => {
    const client = await connectedClient();
    const subscribed = vi.fn();
    const { ready } = client.subscribe("community:3", { onSubscribed: subscribed });
    const [subscribe] = subscribeFrames("community:3");
    serverSends({
      replyTo: subscribe?.id ?? 0n,
      body: {
        case: "error",
        value: { code: "unavailable", message: "TOPIC_CHECK_FAILED", fatal: false },
      },
    });
    await expect(ready).rejects.toMatchObject({ reason: "TOPIC_CHECK_FAILED", revoked: false });
    expect(client.isSubscribed("community:3")).toBe(true);

    // The next connection asks for it again, and its Welcome puts it in effect.
    wire.sent.length = 0;
    wire.close?.("gone");
    const hello = await nextHello();
    expect(hello.body.value.topics).toContain("community:3");
    welcome(hello.id);
    expect(subscribed).toHaveBeenCalledTimes(1);
  });

  it("is answered by the Welcome when the topic went out with the Hello", async () => {
    const client = newClient();
    const subscribed = vi.fn();
    const open = client.subscribe("live:4", { onSubscribed: subscribed });
    const closed = client.subscribe("space:4");
    client.start();
    const hello = await nextHello();
    expect(hello.body.value.topics).toEqual(["live:4", "space:4"]);
    // Nothing was sent on its own before the Hello.
    expect(sentFrames().filter((frame) => frame.body.case === "subscribe")).toHaveLength(0);
    expect(await stateOf(open.ready)).toBe("pending");

    // A refused topic is told before the Welcome, which then confirms the rest.
    serverSends({
      topic: "space:4",
      body: { case: "unsubscribed", value: { topic: "space:4", reason: "TOPIC_FORBIDDEN" } },
    });
    welcome(hello.id);
    expect(await stateOf(open.ready)).toBe("resolved");
    await expect(closed.ready).rejects.toMatchObject({ reason: "TOPIC_FORBIDDEN", revoked: true });
    expect(subscribed).toHaveBeenCalledTimes(1);
  });

  it("is told again after every reconnect, and waits across one", async () => {
    const client = await connectedClient();
    const subscribed = vi.fn();
    client.subscribe("party:5", { onSubscribed: subscribed });
    const [subscribe] = subscribeFrames("party:5");
    serverSends({ replyTo: subscribe?.id ?? 0n, body: { case: "ack", value: {} } });
    expect(subscribed).toHaveBeenCalledTimes(1);

    // The connection drops: a topic subscribed meanwhile waits for the next Welcome.
    wire.close?.("gone");
    wire.sent.length = 0;
    const meanwhile = client.subscribe("live:5");
    const hello = await nextHello();
    expect(hello.body.value.topics).toEqual(["party:5", "live:5"]);
    expect(await stateOf(meanwhile.ready)).toBe("pending");
    // A late Ack from the old connection is not an answer for the new one.
    serverSends({ replyTo: subscribe?.id ?? 0n, body: { case: "ack", value: {} } });
    expect(subscribed).toHaveBeenCalledTimes(1);

    welcome(hello.id);
    expect(await stateOf(meanwhile.ready)).toBe("resolved");
    // Told again, at a random moment within two seconds rather than with everyone else at once.
    expect(subscribed).toHaveBeenCalledTimes(1);
    await vi.waitFor(() => expect(subscribed).toHaveBeenCalledTimes(2), { timeout: 2_500 });
    // Welcome again on the same connection (a refreshed token) is no new start.
    welcome(hello.id);
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(subscribed).toHaveBeenCalledTimes(2);
  });

  it("does not take the answer for a subscription that was let go for this one", async () => {
    const client = await connectedClient();
    const old = client.subscribe("study:6");
    const [first] = subscribeFrames("study:6");
    old.release();
    const subscribed = vi.fn();
    const renewed = client.subscribe("study:6", { onSubscribed: subscribed });
    const [, second] = subscribeFrames("study:6");
    // The first Subscribe's Ack came before the Unsubscribe was handled: it says nothing of the new one.
    serverSends({ replyTo: first?.id ?? 0n, body: { case: "ack", value: {} } });
    expect(await stateOf(renewed.ready)).toBe("pending");
    expect(subscribed).not.toHaveBeenCalled();
    serverSends({ replyTo: second?.id ?? 0n, body: { case: "ack", value: {} } });
    expect(await stateOf(renewed.ready)).toBe("resolved");
    expect(subscribed).toHaveBeenCalledTimes(1);
  });
});

describe("answers that must not be taken for another subscription", () => {
  beforeEach(() => {
    wire.sent.length = 0;
    vi.spyOn(console, "info").mockImplementation(() => {});
    vi.spyOn(console, "error").mockImplementation(() => {});
  });

  it("does not take the Welcome of a Hello for a topic let go and followed anew since", async () => {
    const client = newClient();
    const old = client.subscribe("live:7");
    client.start();
    const hello = await nextHello();
    expect(hello.body.value.topics).toEqual(["live:7"]);
    // Let go and followed again before the Welcome: the new one is asked for on its own.
    old.release();
    const renewed = client.subscribe("live:7");
    const [subscribe] = subscribeFrames("live:7");
    expect(subscribe).toBeDefined();
    welcome(hello.id);
    expect(await stateOf(renewed.ready)).toBe("pending");
    serverSends({ replyTo: subscribe?.id ?? 0n, body: { case: "ack", value: {} } });
    expect(await stateOf(renewed.ready)).toBe("resolved");
  });

  it("drops the ready of a caller that let go, while the others still hear the answer", async () => {
    const client = await connectedClient();
    const leaving = client.subscribe("live:8");
    const staying = client.subscribe("live:8");
    leaving.release();
    const [subscribe] = subscribeFrames("live:8");
    serverSends({ replyTo: subscribe?.id ?? 0n, body: { case: "ack", value: {} } });
    expect(await stateOf(staying.ready)).toBe("resolved");
    expect(await stateOf(leaving.ready)).toBe("pending");
  });
});

describe("listeners", () => {
  beforeEach(() => {
    wire.sent.length = 0;
    vi.spyOn(console, "error").mockImplementation(() => {});
  });

  it("one that throws does not stop the Welcome or the other followers", async () => {
    const client = newClient();
    const reached = vi.fn();
    client.subscribe("live:9", {
      onSubscribed: () => {
        throw new Error("a broken feature");
      },
    });
    client.subscribe("space:9", { onSubscribed: reached });
    const statuses: string[] = [];
    client.onStatus((status) => statuses.push(status));
    client.onFrame(() => {
      throw new Error("another broken feature");
    });
    client.start();
    welcome((await nextHello()).id);
    expect(reached).toHaveBeenCalledTimes(1);
    expect(statuses).toContain("connected");
    expect(console.error).toHaveBeenCalled();
  });
});

describe("refusals that may go away", () => {
  beforeEach(() => {
    wire.sent.length = 0;
    vi.spyOn(console, "info").mockImplementation(() => {});
  });

  it("asks again for a held topic whose guard could not decide, and stops once let go", async () => {
    const client = await connectedClient();
    const subscribed = vi.fn();
    const held = client.subscribe("community:10", { onSubscribed: subscribed });
    const refuse = (frame: realtime.ClientFrame | undefined) =>
      serverSends({
        replyTo: frame?.id ?? 0n,
        body: {
          case: "error",
          value: { code: "unavailable", message: "TOPIC_CHECK_FAILED", fatal: false },
        },
      });
    refuse(subscribeFrames("community:10")[0]);
    await expect(held.ready).rejects.toMatchObject({ revoked: false });
    // Within the first backoff (about a second), on the same connection.
    await vi.waitFor(() => expect(subscribeFrames("community:10")).toHaveLength(2), {
      timeout: 2_000,
    });
    serverSends({
      replyTo: subscribeFrames("community:10")[1]?.id ?? 0n,
      body: { case: "ack", value: {} },
    });
    expect(subscribed).toHaveBeenCalledTimes(1);

    // Refused again for another holder, then let go: nothing more is sent.
    const other = client.subscribe("space:10");
    refuse(subscribeFrames("space:10")[0]);
    other.release();
    await new Promise((resolve) => setTimeout(resolve, 1_200));
    expect(subscribeFrames("space:10")).toHaveLength(1);
  });
});

describe("back in front", () => {
  beforeEach(() => {
    wire.sent.length = 0;
    wire.closes = 0;
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  const pings = () => sentFrames().filter((frame) => frame.body.case === "ping");

  it("makes a connection anew that does not answer a ping", async () => {
    const client = await connectedClient();
    vi.useFakeTimers();
    client.setForeground(false);
    client.setForeground(true);
    expect(pings()).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(4_999);
    expect(wire.closes).toBe(0);
    await vi.advanceTimersByTimeAsync(1);
    expect(wire.closes).toBe(1);
  });

  it("keeps a connection that answers", async () => {
    const client = await connectedClient();
    vi.useFakeTimers();
    client.setForeground(false);
    client.setForeground(true);
    serverSends({ body: { case: "pong", value: {} } });
    await vi.advanceTimersByTimeAsync(5_000);
    expect(wire.closes).toBe(0);
  });

  it("does not wait out the backoff of a connection that dropped meanwhile", async () => {
    const client = await connectedClient();
    vi.useFakeTimers();
    client.setForeground(false);
    wire.close?.("the phone suspended the app");
    expect(client.connectionStatus).toBe("reconnecting");
    client.setForeground(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(sentFrames().filter((frame) => frame.body.case === "hello")).toHaveLength(1);
  });
});

describe("the order of frames", () => {
  beforeEach(() => {
    wire.sent.length = 0;
  });

  it("holds the frames behind a domain's first event until its schemas are loaded", async () => {
    const client = await connectedClient();
    const seen: string[] = [];
    client.onFrame((frame) => seen.push(frame.body.case ?? "none"));
    const chatEvent = () =>
      serverSends({
        body: {
          case: "event",
          value: { type: "tokuly.chat.v1.ChatEvent", payload: new Uint8Array() },
        },
      });
    const presence = () =>
      serverSends({ body: { case: "presence", value: { userId: "u1", online: true } } });

    chatEvent();
    presence();
    chatEvent();
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(seen).toEqual([]);

    chatSchemas.arrive();
    await vi.waitFor(() => expect(seen).toEqual(["chat", "presence", "chat"]));
    // Loaded once, a domain's events are read as they come.
    chatEvent();
    presence();
    expect(seen).toEqual(["chat", "presence", "chat", "chat", "presence"]);
    client.stop();
  });
});
