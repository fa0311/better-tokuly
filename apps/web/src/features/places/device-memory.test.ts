import { create, geo } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { opaqueHash } from "./device-memory";
import { suggestionMark } from "./dismissed";

describe("opaqueHash", () => {
  it("is stable, short and free of the text it came from", () => {
    const hash = opaqueHash("user-1|35.660,139.700");
    expect(hash).toBe(opaqueHash("user-1|35.660,139.700"));
    expect(hash).toMatch(/^[0-9a-z]{1,7}$/);
    expect(hash).not.toContain("35");
    expect(opaqueHash("user-1|35.661,139.700")).not.toBe(hash);
  });
});

describe("suggestionMark", () => {
  const suggestion = (lat: number, lng: number) =>
    create(geo.PlaceSuggestionSchema, { center: { lat, lng } });

  it("is the same for one spot of one account and different for another account", () => {
    expect(suggestionMark("a", suggestion(35.66012, 139.70049))).toBe(
      suggestionMark("a", suggestion(35.66001, 139.70021)),
    );
    expect(suggestionMark("a", suggestion(35.66, 139.7))).not.toBe(
      suggestionMark("b", suggestion(35.66, 139.7)),
    );
    expect(suggestionMark("a", suggestion(35.66, 139.7))).not.toBe(
      suggestionMark("a", suggestion(35.67, 139.7)),
    );
  });
});
