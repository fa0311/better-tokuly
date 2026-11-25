import { create, identity } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { canAddAccount } from "./index";

describe("canAddAccount", () => {
  it("stops at the device's limit", () => {
    const list = (count: number) =>
      create(identity.ListDeviceAccountsResponseSchema, {
        accounts: Array.from({ length: count }, (_, index) => ({ sessionId: `s${index}` })),
        limit: 3,
      });
    expect(canAddAccount(list(2))).toBe(true);
    expect(canAddAccount(list(3))).toBe(false);
  });
});
