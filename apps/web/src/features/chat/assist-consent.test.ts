import { chat } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { consentStorageKey } from "./assist-consent";

describe("consentStorageKey", () => {
  const key = (effectiveProvider: chat.AiProvider, effectiveRecipient: string) =>
    consentStorageKey({ effectiveProvider, effectiveRecipient });

  it("asks again when the text would go somewhere else", () => {
    const anthropic = key(chat.AiProvider.ANTHROPIC, "Anthropic");
    expect(anthropic).not.toBe("");
    // The same provider behind a proxy is another recipient.
    expect(key(chat.AiProvider.ANTHROPIC, "openrouter.ai")).not.toBe(anthropic);
    expect(key(chat.AiProvider.OPENAI, "openrouter.ai")).not.toBe(
      key(chat.AiProvider.ANTHROPIC, "openrouter.ai"),
    );
    expect(key(chat.AiProvider.ANTHROPIC, "ANTHROPIC")).toBe(anthropic);
  });

  it("remembers nothing when nothing receives the text", () => {
    expect(consentStorageKey(undefined)).toBe("");
    expect(key(chat.AiProvider.UNSPECIFIED, "Anthropic")).toBe("");
    expect(key(chat.AiProvider.ANTHROPIC, "")).toBe("");
  });
});
