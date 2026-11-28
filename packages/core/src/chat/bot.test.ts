import { chat, create } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { botHint, mentionsBot } from "./bot";

describe("mentionsBot", () => {
  it("finds the handle as a mention only", () => {
    expect(mentionsBot("@tokuly_bot 集合何時？", "tokuly_bot")).toBe(true);
    expect(mentionsBot("ねえ @Tokuly_Bot", "tokuly_bot")).toBe(true);
    expect(mentionsBot("@tokuly_bot2 だれ？", "tokuly_bot")).toBe(false);
    expect(mentionsBot("mail@tokuly_bot", "tokuly_bot")).toBe(false);
    expect(mentionsBot("tokuly_bot って何", "tokuly_bot")).toBe(false);
  });
});

describe("botHint", () => {
  const settings = (change: Partial<chat.AssistSettings>): chat.AssistSettings => ({
    ...create(chat.AssistSettingsSchema, {
      bot: { id: "bot", handle: "tokuly_bot", displayName: "Tokuly Bot", bot: true },
      botEnabled: true,
      effectiveProvider: chat.AiProvider.ANTHROPIC,
    }),
    ...change,
  });

  it("says why the bot will not answer a mention, and nothing otherwise", () => {
    expect(botHint(settings({}), "@tokuly_bot やっほー")).toBeNull();
    expect(botHint(settings({ botEnabled: false }), "@tokuly_bot やっほー")).toBe("botOff");
    expect(
      botHint(settings({ effectiveProvider: chat.AiProvider.UNSPECIFIED }), "@tokuly_bot やっほー"),
    ).toBe("assistOff");
    expect(botHint(settings({ botEnabled: false }), "やっほー")).toBeNull();
    expect(botHint(settings({ bot: undefined }), "@tokuly_bot やっほー")).toBeNull();
    expect(botHint(undefined, "@tokuly_bot")).toBeNull();
  });
});
