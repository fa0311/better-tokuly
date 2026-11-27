import { create, geo } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { opaqueHash, shownSuggestions, suggestionMark, withDismissed } from "./suggestions";

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

describe("withDismissed and shownSuggestions", () => {
  const spot = (lat: number) => create(geo.PlaceSuggestionSchema, { center: { lat, lng: 139.7 } });

  it("hides a spot put off, and forgets it once it is no longer suggested", () => {
    const [a, b] = [spot(35.66), spot(35.7)];
    const marks = withDismissed("u", [], a, [a, b]);
    expect(shownSuggestions("u", [a, b], marks)).toEqual([b]);
    // Another account on the device still sees it.
    expect(shownSuggestions("v", [a, b], marks)).toEqual([a, b]);
    expect(withDismissed("u", marks, b, [b])).toEqual([suggestionMark("u", b)]);
  });

  it("never shows a suggestion without a spot", () => {
    expect(shownSuggestions("u", [create(geo.PlaceSuggestionSchema, {})], [])).toEqual([]);
  });
});
