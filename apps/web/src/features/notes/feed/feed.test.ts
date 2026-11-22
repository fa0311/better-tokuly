import { describe, expect, it } from "vitest";
import { dayKey, groupByDay } from "./days";
import { parseRecapSearch } from "./recap-screen";

describe("groupByDay", () => {
  it("splits a list in time order into runs of the same day", () => {
    const at = (text: string) => new Date(text).getTime();
    const items = [at("2026-11-22T10:00:00"), at("2026-11-22T08:00:00"), at("2026-11-21T23:59:00"), at("2026-11-20T00:00:00")];
    const days = groupByDay(items, (item) => item);
    expect(days.map((day) => [day.key, day.items.length])).toEqual([
      ["2026-11-22", 2],
      ["2026-11-21", 1],
      ["2026-11-20", 1],
    ]);
    expect(dayKey(new Date(2026, 0, 5))).toBe("2026-01-05");
  });
});

describe("parseRecapSearch", () => {
  it("takes a month or a year and nothing else", () => {
    expect(parseRecapSearch({ p: "2026-10" })).toEqual({ p: "2026-10" });
    expect(parseRecapSearch({ p: 2026 })).toEqual({ p: 2026 });
    expect(parseRecapSearch({ p: "2026" })).toEqual({ p: 2026 });
    expect(parseRecapSearch({ p: "2026-13" })).toEqual({});
    expect(parseRecapSearch({ p: "soon" })).toEqual({});
    expect(parseRecapSearch({})).toEqual({});
  });
});
