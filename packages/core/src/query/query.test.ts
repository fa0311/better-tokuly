import { Code, ConnectError } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { retryDelay, shouldRetry } from "./index";

describe("shouldRetry", () => {
  it("retries twice for errors that may pass", () => {
    expect(shouldRetry(0, new ConnectError("down", Code.Unavailable))).toBe(true);
    expect(shouldRetry(1, new TypeError("fetch failed"))).toBe(true);
    expect(shouldRetry(2, new ConnectError("down", Code.Unavailable))).toBe(false);
  });

  it("asks a rate-limited query again once, after the wait the server named", () => {
    const limited = (retryAfter?: string) =>
      new ConnectError(
        "slow down",
        Code.ResourceExhausted,
        retryAfter ? { "retry-after": retryAfter } : {},
      );
    expect(shouldRetry(0, limited("3"))).toBe(true);
    expect(retryDelay(0, limited("3"))).toBe(3000);
    expect(shouldRetry(1, limited("3"))).toBe(false);
    // Not told when, or told to wait long: the screen shows the error instead.
    expect(shouldRetry(0, limited())).toBe(false);
    expect(shouldRetry(0, limited("40"))).toBe(false);
    // Anything else backs off the way TanStack Query does by itself.
    expect(retryDelay(1, new ConnectError("down", Code.Unavailable))).toBe(2000);
  });

  it("never retries an answer that will not change", () => {
    expect(shouldRetry(0, new ConnectError("gone", Code.NotFound))).toBe(false);
    expect(shouldRetry(0, new ConnectError("no", Code.PermissionDenied))).toBe(false);
  });
});
