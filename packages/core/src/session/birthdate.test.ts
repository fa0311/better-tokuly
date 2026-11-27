import { create, identity } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { asksForBirthdate, asksForBirthdateNow } from "./birthdate";

describe("asksForBirthdate", () => {
  it("asks an account without a birthdate", () => {
    expect(asksForBirthdate(create(identity.MeSchema, { birthdate: "" }))).toBe(true);
  });

  it("does not ask an account that has one, nor a bot", () => {
    expect(asksForBirthdate(create(identity.MeSchema, { birthdate: "2010-04-02" }))).toBe(false);
    expect(asksForBirthdate(create(identity.MeSchema, { bot: true }))).toBe(false);
  });
});

describe("asksForBirthdateNow", () => {
  const without = (id: string) => create(identity.MeSchema, { user: { id }, birthdate: "" });

  it("waits while the account that said later is signed in", () => {
    expect(asksForBirthdateNow(without("u1"), null)).toBe(true);
    expect(asksForBirthdateNow(without("u1"), "u1")).toBe(false);
  });

  it("asks another account, and nobody while signed out", () => {
    expect(asksForBirthdateNow(without("u2"), "u1")).toBe(true);
    expect(asksForBirthdateNow(null, null)).toBe(false);
  });
});
