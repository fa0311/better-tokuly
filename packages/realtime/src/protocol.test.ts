import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { clientIsOutdated, PROTOCOL_VERSION } from "./protocol.js";

describe("protocol version", () => {
  it("is the one the gateway announces", () => {
    const gateway = readFileSync(new URL("../../../backend/crates/realtime/src/frames.rs", import.meta.url), "utf8");
    const announced = /const PROTOCOL_VERSION: u32 = (\d+);/.exec(gateway)?.[1];
    expect(Number(announced)).toBe(PROTOCOL_VERSION);
  });

  it("asks for a reload only when the server has moved ahead", () => {
    expect(clientIsOutdated(PROTOCOL_VERSION + 1)).toBe(true);
    expect(clientIsOutdated(PROTOCOL_VERSION)).toBe(false);
    // A server from before versions existed, or one still being deployed: reloading changes nothing.
    expect(clientIsOutdated(0)).toBe(false);
    expect(clientIsOutdated(PROTOCOL_VERSION - 1)).toBe(false);
  });
});
