import { describe, expect, it } from "vitest";
import { allTimeZones, searchTimeZones, zoneLabel, zoneOffset } from "./time-zone";

describe("time zones", () => {
  it("formats the current offset of a zone", () => {
    expect(zoneOffset("Asia/Tokyo")).toBe("UTC+9");
    expect(zoneOffset("UTC")).toBe("UTC");
    expect(zoneOffset("Asia/Kolkata")).toBe("UTC+5:30");
  });

  it("gives nothing for a zone the runtime does not know", () => {
    expect(zoneOffset("Mars/Olympus_Mons")).toBe("");
  });

  it("lists UTC next to the regional zones", () => {
    const zones = allTimeZones();
    expect(zones).toContain("UTC");
    expect(zones).toContain("Asia/Tokyo");
  });

  it("finds zones by any part of the name, ignoring case and underscores", () => {
    const zones = ["Asia/Tokyo", "Asia/Ho_Chi_Minh", "America/New_York", "Europe/London"];
    expect(searchTimeZones(zones, "tokyo")).toEqual(["Asia/Tokyo"]);
    expect(searchTimeZones(zones, "new york")).toEqual(["America/New_York"]);
    expect(searchTimeZones(zones, "asia/ho")).toEqual(["Asia/Ho_Chi_Minh"]);
    expect(searchTimeZones(zones, "  ")).toEqual(zones);
    expect(searchTimeZones(zones, "atlantis")).toEqual([]);
  });

  it("shows zone names without underscores", () => {
    expect(zoneLabel("America/Argentina/Buenos_Aires")).toBe("America/Argentina/Buenos Aires");
  });
});
