import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import { common, realtime } from "@tokuly/api";
import { beforeEach, describe, expect, it, vi } from "vitest";

/** What the client sent, decoded, and a way to answer as the gateway. */
const wire = vi.hoisted(() => ({
  sent: [] as Uint8Array[],
  deliver: null as ((frame: Uint8Array) => void) | null,
  close: null as ((reason: string) => void) | null,
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
      close: () => {},
    });
  },
}));

const { RealtimeClient } = await import("./client.js");

function sentFrames(): realtime.ClientFrame[] {
  return wire.sent.map((bytes) => fromBinary(realtime.ClientFrameSchema, bytes));
}

function serverSends(frame: Parameters<typeof create<typeof realtime.ServerFrameSchema>>[1]): void {
  wire.deliver?.(toBinary(realtime.ServerFrameSchema, create(realtime.ServerFrameSchema, frame)));
}

async function connectedClient() {
  const client = new RealtimeClient({
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
  client.start();
  await vi.waitFor(() =>
    expect(sentFrames().some((frame) => frame.body.case === "hello")).toBe(true),
  );
  const hello = sentFrames().find((frame) => frame.body.case === "hello");
  serverSends({
    replyTo: hello?.id ?? 0n,
    body: { case: "welcome", value: { protocolVersion: 2 } },
  });
  wire.sent.length = 0;
  return client;
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
    const releaseFirst = client.subscribe("space:1", { onRevoked: first });
    const releaseSecond = client.subscribe("space:1", { onRevoked: second });
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
    const releaseOld = client.subscribe("live:3");
    serverSends({
      topic: "live:3",
      body: { case: "unsubscribed", value: { topic: "live:3", reason: "TOPIC_CLOSED" } },
    });
    const releaseNew = client.subscribe("live:3");
    expect(subscribeFrames("live:3")).toHaveLength(2);
    // The old holder's release must not end the new interest.
    releaseOld();
    expect(client.isSubscribed("live:3")).toBe(true);
    releaseNew();
    expect(client.isSubscribed("live:3")).toBe(false);
    expect(sentFrames().filter((frame) => frame.body.case === "unsubscribe")).toHaveLength(1);
  });
});
