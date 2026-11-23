import { describe, expect, it } from "vitest";
import {
  allowedForBots,
  groupScopes,
  levelOf,
  type ScopeEntry,
  splitScopes,
  withLevel,
  writesWithoutSpending,
} from "./scopes";

const entry = (scope: string, sensitive = false): ScopeEntry => {
  const [domain = "", level] = scope.split(":");
  return { scope, domain, write: level === "write", sensitive };
};

const CATALOGUE = [
  entry("geo:read", true),
  entry("geo:write", true),
  entry("notes:read"),
  entry("notes:write"),
  entry("realtime:read"),
  entry("wallet:read", true),
  entry("wallet:write", true),
];

describe("groupScopes", () => {
  it("makes one group per domain, in catalogue order, with the levels the domain offers", () => {
    const groups = groupScopes(CATALOGUE);
    expect(groups.map((group) => group.domain)).toEqual(["geo", "notes", "realtime", "wallet"]);
    expect(groups[0]).toMatchObject({ sensitive: true });
    expect(groups[1]?.write?.scope).toBe("notes:write");
    expect(groups[2]?.write).toBeUndefined();
  });
});

describe("levels", () => {
  it("reads the level of a domain from a selection", () => {
    expect(levelOf(["notes:write", "geo:read"], "notes")).toBe("write");
    expect(levelOf(["notes:write", "geo:read"], "geo")).toBe("read");
    expect(levelOf(["notes:write"], "chat")).toBe("none");
  });

  it("keeps one scope per domain when a level changes", () => {
    expect(withLevel(["notes:read", "geo:read"], "notes", "write")).toEqual([
      "geo:read",
      "notes:write",
    ]);
    expect(withLevel(["notes:write", "geo:read"], "notes", "none")).toEqual(["geo:read"]);
  });
});

describe("writesWithoutSpending", () => {
  it("is true only for a selection that writes without the wallet", () => {
    expect(writesWithoutSpending(["stamp:write"])).toBe(true);
    expect(writesWithoutSpending(["stamp:write", "wallet:write"])).toBe(false);
    expect(writesWithoutSpending(["stamp:read", "wallet:read"])).toBe(false);
    expect(writesWithoutSpending([])).toBe(false);
  });
});

describe("allowedForBots", () => {
  it("offers bots reading only, and nothing of the wallet", () => {
    const offered = CATALOGUE.filter(allowedForBots).map((scope) => scope.scope);
    expect(offered).toEqual(["geo:read", "notes:read", "realtime:read"]);
  });
});

describe("splitScopes", () => {
  it("splits the space-separated lists of the API", () => {
    expect(splitScopes("feed:read geo:read")).toEqual(["feed:read", "geo:read"]);
    expect(splitScopes("")).toEqual([]);
  });
});
