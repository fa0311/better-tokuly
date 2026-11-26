import { create } from "@bufbuild/protobuf";
import { ask } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { answerShareUrl, boxShareUrl, charCount, hasEngagement } from "./model";

describe("charCount", () => {
  it("counts characters the way the server does, not UTF-16 units", () => {
    expect(charCount("質問箱")).toBe(3);
    expect(charCount("🔥🔥")).toBe(2);
    expect(charCount("")).toBe(0);
  });
});

describe("share links", () => {
  it("point at the public page of the box", () => {
    expect(boxShareUrl("alice", "https://tokuly.example")).toBe("https://tokuly.example/q/alice");
    expect(answerShareUrl("alice", "q1", "https://tokuly.example")).toBe(
      "https://tokuly.example/q/alice?a=q1",
    );
  });
});

describe("hasEngagement", () => {
  const answered = (audience: ask.AnswerAudience) =>
    create(ask.QuestionSchema, { id: "q1", answer: { audience } });
  it("is there for answers on the Q&A page only", () => {
    expect(hasEngagement(answered(ask.AnswerAudience.PUBLIC))).toBe(true);
    expect(hasEngagement(answered(ask.AnswerAudience.FRIENDS))).toBe(true);
    expect(hasEngagement(answered(ask.AnswerAudience.PRIVATE))).toBe(false);
    expect(hasEngagement(create(ask.QuestionSchema, { id: "q2" }))).toBe(false);
  });
});
