import { describe, expect, it } from "vitest";
import { retryDelay } from "./live-player";

describe("player retries", () => {
  it("back off and level out", () => {
    expect(retryDelay(0)).toBe(2000);
    expect(retryDelay(1)).toBe(4000);
    expect(retryDelay(3)).toBe(16_000);
    expect(retryDelay(10)).toBe(30_000);
  });
});
