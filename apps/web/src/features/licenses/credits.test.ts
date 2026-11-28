import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { CREDIT_GROUPS } from "./credits";

/** The web app and the packages it bundles. */
const PACKAGES = [
  "apps/web",
  "packages/api",
  "packages/core",
  "packages/i18n",
  "packages/realtime",
];
const repo = new URL("../../../../../", import.meta.url);

function json(path: string) {
  return JSON.parse(readFileSync(new URL(path, repo), "utf8"));
}

/** Every runtime dependency outside the workspace, with the licences it declares. */
function dependencies() {
  const found = new Map<string, string[]>();
  for (const dir of PACKAGES) {
    for (const name of Object.keys(json(`${dir}/package.json`).dependencies ?? {})) {
      if (name.startsWith("@tokuly/")) continue;
      const license: string = json(`${dir}/node_modules/${name}/package.json`).license;
      found.set(name, license.replace(/[()]/g, "").split(" AND "));
    }
  }
  return found;
}

describe("credits", () => {
  const credits = CREDIT_GROUPS.flatMap((group) => group.credits);

  it("list every dependency with the licence it declares", () => {
    for (const [name, licenses] of dependencies()) {
      const credit = credits.find((entry) => entry.package === name);
      expect(credit, name).toBeDefined();
      expect(credit?.licenses, name).toEqual(licenses);
    }
  });

  it("name each package once", () => {
    const packages = credits.flatMap((credit) => credit.package ?? []);
    expect(new Set(packages).size).toBe(packages.length);
  });
});
