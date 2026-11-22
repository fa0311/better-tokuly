import { existsSync, readFileSync } from "node:fs";
import { create, toBinary } from "@bufbuild/protobuf";
import { chat, geo, realtime, wallet } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { decodeServerFrame, KNOWN_EVENT_TYPES } from "./events.js";

/** The bytes of a frame as the server sends it. */
function wire(frame: Parameters<typeof create<typeof realtime.ServerFrameSchema>>[1]): Uint8Array {
  return toBinary(realtime.ServerFrameSchema, create(realtime.ServerFrameSchema, frame));
}

/** A frame that carries a domain's event in the envelope. */
function eventFrame(type: string, payload: Uint8Array, topic = ""): Uint8Array {
  return wire({ topic, body: { case: "event", value: { type, payload } } });
}

describe("server frames", () => {
  it("hands a domain's event to listeners under its own case, decoded", () => {
    const event = create(wallet.WalletEventSchema, {
      event: { case: "balanceChanged", value: { balance: { coins: 1200n, revision: 7n } } },
    });
    const frame = decodeServerFrame(
      eventFrame("tokuly.wallet.v1.WalletEvent", toBinary(wallet.WalletEventSchema, event)),
    );

    expect(frame?.body.case).toBe("wallet");
    // The union narrows the way the generated oneof did.
    if (frame?.body.case !== "wallet") throw new Error("not a wallet frame");
    const change = frame.body.value.event;
    expect(change.case).toBe("balanceChanged");
    expect(change.case === "balanceChanged" && change.value.balance?.coins).toBe(1200n);
    expect(frame.topic).toBe("");
    expect(frame.replyTo).toBe(0n);
  });

  it("keeps the topic of a topic-scoped event", () => {
    const event = create(chat.ChatEventSchema, {
      event: { case: "typing", value: { conversationId: "c1", userId: "u1", typing: true } },
    });
    const frame = decodeServerFrame(
      eventFrame(chat.ChatEventSchema.typeName, toBinary(chat.ChatEventSchema, event), "live:42"),
    );
    expect(frame?.topic).toBe("live:42");
    expect(frame?.body.case === "chat" && frame.body.value.event.case).toBe("typing");
  });

  it("knows exactly the events the protocol lists", () => {
    // The source is the list at `Event` in `realtime.proto`: lines of the
    // form `//   tokuly.<package>.v1.<Name>   what it is about`.
    const protoRoot = new URL("../../../proto/tokuly/", import.meta.url);
    const protocol = readFileSync(new URL("realtime/v1/realtime.proto", protoRoot), "utf8");
    const listed = [...protocol.matchAll(/^\/\/ {3}(tokuly\.[a-z]+\.v1\.[A-Za-z]+)/gm)].map((match) => match[1] ?? "");
    expect(listed.length).toBeGreaterThan(0);
    for (const type of listed) {
      const [, domain, , name] = type.split(".");
      const file = new URL(`${domain}/v1/${domain}.proto`, protoRoot);
      expect(existsSync(file) && readFileSync(file, "utf8").includes(`\nmessage ${name} {`), type).toBe(true);
    }
    expect([...KNOWN_EVENT_TYPES].sort()).toEqual([...listed].sort());
    for (const type of KNOWN_EVENT_TYPES) {
      const domain = type.split(".")[1];
      expect(decodeServerFrame(eventFrame(type, new Uint8Array()))?.body.case, type).toBe(domain);
    }
  });

  it("passes the gateway's own messages through unchanged", () => {
    const welcome = decodeServerFrame(
      wire({
        replyTo: 3n,
        body: { case: "welcome", value: { connectionId: "c", pingIntervalSeconds: 25 } },
      }),
    );
    expect(welcome?.replyTo).toBe(3n);
    expect(welcome?.body.case === "welcome" && welcome.body.value.pingIntervalSeconds).toBe(25);

    const presence = decodeServerFrame(
      wire({ body: { case: "presence", value: { userId: "u1", online: true } } }),
    );
    expect(presence?.body.case === "presence" && presence.body.value.userId).toBe("u1");

    const stats = decodeServerFrame(
      wire({ topic: "live:1", body: { case: "topicStats", value: { subscriberCount: 12n } } }),
    );
    expect(stats?.body.case === "topicStats" && stats.body.value.subscriberCount).toBe(12n);
    expect(stats?.topic).toBe("live:1");
  });

  it("skips events of a type it does not know instead of failing", () => {
    expect(
      decodeServerFrame(eventFrame("tokuly.future.v1.FutureEvent", new Uint8Array([1, 2, 3]))),
    ).toBeNull();
    expect(decodeServerFrame(eventFrame("", new Uint8Array()))).toBeNull();
  });

  it("skips what it cannot read", () => {
    // A known type whose payload is not that message.
    expect(
      decodeServerFrame(eventFrame("tokuly.geo.v1.GeoEvent", new Uint8Array([0xff, 0xff, 0xff]))),
    ).toBeNull();
    // Bytes that are no frame at all.
    expect(decodeServerFrame(new Uint8Array([0xff, 0xff, 0xff]))).toBeNull();
  });

  it("delivers a frame with a body it has no case for as an empty one", () => {
    // Field 99 of ServerFrame does not exist (yet): a newer server's addition to the envelope.
    const frame = decodeServerFrame(new Uint8Array([0x9a, 0x06, 0x00]));
    expect(frame?.body.case).toBeUndefined();
  });
});

describe("location samples", () => {
  it("travel encoded, which on the wire is the sample as a nested message", () => {
    const sample = create(geo.LocationSampleSchema, {
      position: { lat: 35.6595, lng: 139.7005 },
      accuracyMeters: 8,
    });
    const encoded = toBinary(geo.LocationSampleSchema, sample);
    const frame = toBinary(
      realtime.ClientFrameSchema,
      create(realtime.ClientFrameSchema, { body: { case: "location", value: encoded } }),
    );
    // Tag of field 4 with wire type 2 (length-delimited), the length, then the sample itself.
    expect([...frame]).toEqual([0x22, encoded.length, ...encoded]);
  });
});
