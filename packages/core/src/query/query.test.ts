import { Code, ConnectError } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { shouldRetry } from "./index";

describe("shouldRetry", () => {
  it("retries twice for errors that may pass", () => {
    expect(shouldRetry(0, new ConnectError("down", Code.Unavailable))).toBe(true);
    expect(shouldRetry(1, new TypeError("fetch failed"))).toBe(true);
    expect(shouldRetry(2, new ConnectError("down", Code.Unavailable))).toBe(false);
  });

  it("never retries an answer that will not change", () => {
    expect(shouldRetry(0, new ConnectError("gone", Code.NotFound))).toBe(false);
    expect(shouldRetry(0, new ConnectError("no", Code.PermissionDenied))).toBe(false);
  });
});
