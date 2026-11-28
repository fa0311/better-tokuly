import { describe, expect, it } from "vitest";
import { mentionsBot } from "./bot";

describe("mentionsBot", () => {
  it("finds the handle as a mention only", () => {
    expect(mentionsBot("@tokuly_bot 集合何時？", "tokuly_bot")).toBe(true);
    expect(mentionsBot("ねえ @Tokuly_Bot", "tokuly_bot")).toBe(true);
    expect(mentionsBot("@tokuly_bot2 だれ？", "tokuly_bot")).toBe(false);
    expect(mentionsBot("mail@tokuly_bot", "tokuly_bot")).toBe(false);
    expect(mentionsBot("tokuly_bot って何", "tokuly_bot")).toBe(false);
  });
});
