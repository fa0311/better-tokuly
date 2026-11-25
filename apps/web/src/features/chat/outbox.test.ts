import { chat } from "@tokuly/api";
import { rowKey } from "@tokuly/core/chat";
import { describe, expect, it } from "vitest";
import { classify, needsConversion } from "./attachments";
import { bigEmojiCount, type Outgoing, previewMessage } from "./outbox";
import { extensionOf } from "./recorder";

const me = {
  $typeName: "tokuly.common.v1.UserSummary" as const,
  id: "me",
  handle: "me",
  displayName: "me",
  avatarVideoUrl: "",
  verified: false,
  deleted: false,
};
const outgoing = (extra: Partial<Outgoing>): Outgoing => ({
  clientId: "client-1",
  conversationId: "c1",
  createdAt: Date.now(),
  text: "",
  files: [],
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
    const { message, state } = previewMessage(outgoing({ text: "🥰" }), me);
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
    const { message } = previewMessage(outgoing({ text: "hi" }), me);
    expect(rowKey(message)).toBe(rowKey({ ...message, id: "01a2", seq: 12n }));
  });

  it("carry the quote of the message they answer", () => {
    const answered = previewMessage(outgoing({ text: "a long question ".repeat(20) }), me).message;
    const { message } = previewMessage(
      outgoing({ text: "yes", replyTo: { ...answered, id: "m1" } }),
      me,
    );
    expect(message.replyTo?.messageId).toBe("m1");
    expect(message.replyTo?.text).toHaveLength(120);
  });

  it("report why they failed", () => {
    expect(
      previewMessage(outgoing({ status: "failed", error: "通信できませんでした" }), me).state,
    ).toMatchObject({ status: "failed", error: "通信できませんでした" });
  });
});

describe("files", () => {
  it("are sent as what they are", () => {
    expect(classify("image/jpeg")).toBe("image");
    expect(classify("image/gif")).toBe("image");
    expect(classify("video/mp4")).toBe("video");
    expect(classify("audio/webm")).toBe("audio");
    // Photos the server cannot decode are still photos: converted before upload, never sent as files.
    for (const type of ["image/heic", "image/heif", "image/avif", "image/bmp"]) {
      expect(classify(type)).toBe("image");
      expect(needsConversion(type)).toBe(true);
    }
    expect(needsConversion("image/jpeg")).toBe(false);
    expect(needsConversion("image/webp")).toBe(false);
    // Pictures that are documents rather than photos go as files.
    expect(classify("image/svg+xml")).toBe("file");
    expect(classify("application/pdf")).toBe("file");
    expect(classify("")).toBe("file");
  });

  it("get the extension of the container that was recorded", () => {
    expect(extensionOf("audio/webm;codecs=opus")).toBe("webm");
    expect(extensionOf("audio/mp4")).toBe("m4a");
    expect(extensionOf("video/mp4")).toBe("mp4");
    expect(extensionOf("audio/ogg;codecs=opus")).toBe("ogg");
  });
});
