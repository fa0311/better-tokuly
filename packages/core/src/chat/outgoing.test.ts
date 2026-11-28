import { chat } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { bigEmojiCount, type OutgoingMessage, previewMessage, SendQueue } from "./outgoing";
import { rowKey } from "./timeline";

const me = {
  $typeName: "tokuly.common.v1.UserSummary" as const,
  id: "me",
  handle: "me",
  displayName: "me",
  avatarVideoUrl: "",
  verified: false,
  deleted: false,
  bot: false,
};
const outgoing = (extra: Partial<OutgoingMessage>): OutgoingMessage => ({
  clientId: "client-1",
  conversationId: "c1",
  createdAt: Date.now(),
  text: "",
  status: "sending",
  progress: 0,
  ...extra,
});

describe("big emoji", () => {
  it("counts one to three emoji standing alone", () => {
    expect(bigEmojiCount("🥰")).toBe(1);
    expect(bigEmojiCount(" 🎉 🎉 ")).toBe(2);
    expect(bigEmojiCount("👍🏽👨‍👩‍👧🇯🇵")).toBe(3);
  });

  it("is zero for text, mixed content and more than three", () => {
    expect(bigEmojiCount("やった🎉")).toBe(0);
    expect(bigEmojiCount("😀😀😀😀")).toBe(0);
    expect(bigEmojiCount("")).toBe(0);
    expect(bigEmojiCount("123")).toBe(0);
  });
});

describe("messages on their way", () => {
  it("are drawn as the message they will become", () => {
    const { message, state } = previewMessage(outgoing({ text: "🥰" }), [], me);
    expect(message).toMatchObject({
      kind: chat.MessageKind.TEXT,
      text: "🥰",
      bigEmojiCount: 1,
      clientId: "client-1",
    });
    expect(message.sender?.id).toBe("me");
    expect(state).toEqual({ status: "sending", progress: undefined, error: undefined });
  });

  it("share their row with the stored message that replaces them", () => {
    const { message } = previewMessage(outgoing({ text: "hi" }), [], me);
    expect(rowKey(message)).toBe(rowKey({ ...message, id: "01a2", seq: 12n }));
  });

  it("carry the quote of the message they answer", () => {
    const answered = previewMessage(outgoing({ text: "a long question ".repeat(20) }), [], me);
    const { message } = previewMessage(
      outgoing({ text: "yes", replyTo: { ...answered.message, id: "m1" } }),
      [],
      me,
    );
    expect(message.replyTo?.messageId).toBe("m1");
    expect(message.replyTo?.text).toHaveLength(120);
  });

  it("report why they failed", () => {
    expect(
      previewMessage(outgoing({ status: "failed", error: "通信できませんでした" }), [], me).state,
    ).toMatchObject({ status: "failed", error: "通信できませんでした" });
  });
});

describe("the send queue", () => {
  it("sends a conversation's messages in the order written, past a failure", async () => {
    const queue = new SendQueue();
    const sent: string[] = [];
    const slow = () => new Promise<void>((resolve) => setTimeout(resolve, 5));
    const first = queue.run("c1", async () => {
      await slow();
      sent.push("photo");
      throw new Error("refused");
    });
    const second = queue.run("c1", async () => {
      sent.push("text");
    });
    await Promise.allSettled([first, second]);
    expect(sent).toEqual(["photo", "text"]);
  });
});
