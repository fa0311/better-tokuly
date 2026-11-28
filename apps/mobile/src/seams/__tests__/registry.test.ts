const { areasOf, staleRegistries, strayFiles, strayReason } =
  require("../../../scripts/seams.js") as {
    areasOf(seam: string): string[];
    staleRegistries(): string[];
    strayFiles(): string[];
    strayReason(file: string, source: () => string): string | null;
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
    expect(areasOf("lifecycle")).toEqual([
      "chat",
      "live",
      "map",
      "media",
      "notifications",
      "school",
      "spaces",
      "video",
      "web",
    ]);
    expect(areasOf("tabs")).toEqual(["chat", "map", "notes", "notifications", "profile", "shorts"]);
    expect(areasOf("hub")).toEqual([
      "ask",
      "community",
      "emoji",
      "footprints",
      "friends",
      "history",
      "live",
      "moments",
      "places",
      "plans",
      "quest",
      "school",
      "search",
      "settings",
      "spaces",
      "stamps",
      "video",
      "wallet",
      "web",
    ]);
    expect(areasOf("profile")).toEqual(["ask", "notes", "plans", "quest"]);
  });

  it("stop the build for a seam file they would leave out", () => {
    const hook = () => "export default { beforeLeave };\n";
    expect(strayReason("map/lifecycle.ts", hook)).toBeNull();
    expect(strayReason("map/tabs.tsx", () => "export default [];\n")).toBeNull();
    expect(strayReason("session/__tests__/lifecycle.test.ts", hook)).toBeNull();
    // A privacy hook that would never run: one folder too deep, the wrong kind of file, not exported.
    const misplaced = "a seam file is src/features/<area>/lifecycle.ts(x)";
    expect(strayReason("live/hooks/lifecycle.ts", hook)).toBe(misplaced);
    expect(strayReason("notes/lifecycle.js", hook)).toBe(misplaced);
    expect(strayReason("lifecycle.ts", hook)).toBe(misplaced);
    expect(strayReason("shorts/lifecycle.ts", () => "export const hooks = {};\n")).toBe(
      "no `export default`",
    );
    expect(strayFiles()).toEqual([]);
  });
});
