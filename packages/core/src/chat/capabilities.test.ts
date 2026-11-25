import { chat, create } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { capabilitiesOf, DEFAULT_CAPABILITIES } from "./capabilities";

describe("capabilitiesOf", () => {
  it("keeps the optional features off until the server answered", () => {
    expect(capabilitiesOf(undefined)).toEqual(DEFAULT_CAPABILITIES);
    expect(capabilitiesOf(undefined).callsEnabled).toBe(false);
  });

  it("takes the server's limits and its defaults where it set none", () => {
    const caps = capabilitiesOf(
      create(chat.CapabilitiesSchema, { maxTextLength: 2000, callsEnabled: true }),
    );
    expect(caps.maxTextLength).toBe(2000);
    expect(caps.maxAttachments).toBe(DEFAULT_CAPABILITIES.maxAttachments);
    expect(caps.callsEnabled).toBe(true);
  });
});
