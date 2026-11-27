import { create, identity } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { asksForBirthdate } from "./birthdate";

describe("asksForBirthdate", () => {
  it("asks an account without a birthdate", () => {
    expect(asksForBirthdate(create(identity.MeSchema, { birthdate: "" }))).toBe(true);
  });

  it("does not ask an account that has one, nor a bot", () => {
    expect(asksForBirthdate(create(identity.MeSchema, { birthdate: "2010-04-02" }))).toBe(false);
    expect(asksForBirthdate(create(identity.MeSchema, { bot: true }))).toBe(false);
  });
});
