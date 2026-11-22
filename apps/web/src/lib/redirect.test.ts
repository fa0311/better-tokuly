import { describe, expect, it } from "vitest";
import { parseRedirect } from "./redirect";

describe("parseRedirect", () => {
  it("keeps a path inside the app, with its search", () => {
    expect(parseRedirect("/q/mio")).toBe("/q/mio");
    expect(parseRedirect("/join/party/ABC123?x=1")).toBe("/join/party/ABC123?x=1");
  });

  it("refuses anything that would leave the app", () => {
    expect(parseRedirect("https://example.com/")).toBeUndefined();
    expect(parseRedirect("//example.com/")).toBeUndefined();
    expect(parseRedirect("/\\example.com")).toBeUndefined();
    expect(parseRedirect("map")).toBeUndefined();
    expect(parseRedirect(undefined)).toBeUndefined();
    expect(parseRedirect(42)).toBeUndefined();
  });
});
