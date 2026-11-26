const { areasOf, staleRegistries } = require("../../../scripts/seams.js") as {
  areasOf(seam: string): string[];
  staleRegistries(): string[];
};

/**
 * The registries are written by `scripts/seams.js` before every bundle, type
 * check and test run. Jest started by hand (or watching) after a seam file
 * was added or removed says so here, instead of a feature silently missing.
 */
describe("the seam registries", () => {
  it("are what the features' files say right now (stale: `pnpm seams`)", () => {
    expect(staleRegistries()).toEqual([]);
  });

  it("find the features by their files", () => {
    expect(areasOf("lifecycle")).toEqual(["map", "media", "notifications"]);
    expect(areasOf("tabs")).toEqual([
      "chat",
      "friends",
      "map",
      "notifications",
      "profile",
      "shorts",
    ]);
    expect(areasOf("profile")).toEqual([]);
  });
});
