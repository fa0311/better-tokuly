import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const { areasOf, staleRegistries, strayFiles } = require("../../../scripts/seams.js") as {
  areasOf(seam: string): string[];
  staleRegistries(): string[];
  strayFiles(features?: string): string[];
};

/** A features folder with these files in it. */
function featuresWith(files: Record<string, string>): string {
  const folder = fs.mkdtempSync(path.join(os.tmpdir(), "seams-"));
  for (const [file, source] of Object.entries(files)) {
    fs.mkdirSync(path.dirname(path.join(folder, file)), { recursive: true });
    fs.writeFileSync(path.join(folder, file), source);
  }
  return folder;
}

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

  it("stop the build for a seam file they would leave out", () => {
    const features = featuresWith({
      "map/lifecycle.ts": "export default { beforeLeave };\n",
      "map/tabs.tsx": "export default [];\n",
      "map/links.test.ts": "",
      // A privacy hook that would never run: one folder too deep, the wrong kind of file, not exported.
      "live/hooks/lifecycle.ts": "export default {};\n",
      "notes/lifecycle.js": "export default {};\n",
      "shorts/lifecycle.ts": "export const lifecycle = {};\n",
    });
    try {
      expect(strayFiles(features).sort()).toEqual([
        "live/hooks/lifecycle.ts: a seam file is src/features/<area>/lifecycle.ts(x)",
        "notes/lifecycle.js: a seam file is src/features/<area>/lifecycle.ts(x)",
        "shorts/lifecycle.ts: no `export default`",
      ]);
    } finally {
      fs.rmSync(features, { recursive: true });
    }
    expect(strayFiles()).toEqual([]);
  });
});
