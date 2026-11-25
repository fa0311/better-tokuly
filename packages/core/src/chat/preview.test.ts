import { chat, create, type MessageInitShape, media } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { testI18n } from "../testing/i18n";
import { make } from "../testing/make";
import { chatSentences } from "./preview";

const { callText, previewText, replyText, systemText } = chatSentences(testI18n());

const ME = "me";
const user = (id: string, displayName = id) => ({ id, handle: id, displayName });
const message = (extra: MessageInitShape<typeof chat.MessageSchema>) =>
  make(
    chat.MessageSchema,
    { id: "m", kind: chat.MessageKind.TEXT, sender: user("aoi", "あおい") },
    extra,
  );
const attachment = (kind: media.AssetKind, fileName = "") =>
  create(media.AttachmentSchema, { kind, fileName });

describe("list previews", () => {
  it("says who wrote the last message: you, or the sender in a group", () => {
    expect(previewText(message({ text: "おはよ" }), ME, false)).toBe("おはよ");
    expect(previewText(message({ text: "おはよ" }), ME, true)).toBe("あおい: おはよ");
    expect(previewText(message({ text: "おはよ", sender: user(ME) }), ME, true)).toBe(
      "あなた: おはよ",
    );
    expect(previewText(undefined, ME, false)).toBe("");
  });

  it("puts a message with line breaks on one line", () => {
    expect(previewText(message({ text: "一行目\n\n二行目" }), ME, false)).toBe("一行目 二行目");
  });

  it("describes what cannot be shown as text", () => {
    expect(previewText(message({ kind: chat.MessageKind.STAMP, text: "" }), ME, false)).toBe(
      "スタンプを送信しました",
    );
    expect(previewText(message({ kind: chat.MessageKind.LOCATION, text: "" }), ME, false)).toBe(
      "📍 位置情報",
    );
    expect(previewText(message({ kind: chat.MessageKind.HELLO, text: "" }), ME, false)).toBe(
      "👋 手を振りました",
    );
    expect(previewText(message({ deleted: true, text: "" }), ME, false)).toBe(
      "メッセージの送信を取り消しました",
    );
  });

  it("names the attachments of a media message, with its caption", () => {
    const media1 = (attachments: media.Attachment[], text = "") =>
      previewText(message({ kind: chat.MessageKind.MEDIA, attachments, text }), ME, false);
    expect(media1([attachment(media.AssetKind.IMAGE)])).toBe("📷 写真");
    expect(
      media1([attachment(media.AssetKind.IMAGE), attachment(media.AssetKind.IMAGE)], "きれい"),
    ).toBe("📷 写真 2枚 きれい");
    expect(media1([attachment(media.AssetKind.VIDEO)])).toBe("🎬 動画");
    expect(media1([attachment(media.AssetKind.AUDIO)])).toBe("🎤 ボイスメッセージ");
    expect(media1([attachment(media.AssetKind.FILE, "宿題.pdf")])).toBe("宿題.pdf");
  });
});

describe("system notices", () => {
  const event = (
    type: chat.SystemEventType,
    extra: MessageInitShape<typeof chat.SystemEventSchema> = {},
  ) => make(chat.SystemEventSchema, { type, actor: user("aoi", "あおい") }, extra);

  it("builds the sentence from who did what to whom", () => {
    expect(systemText(event(chat.SystemEventType.GROUP_CREATED, { text: "放課後" }), ME)).toBe(
      "あおいがグループ「放課後」をつくりました",
    );
    expect(
      systemText(
        event(chat.SystemEventType.MEMBERS_ADDED, { targets: [user("ren", "れん"), user(ME)] }),
        ME,
      ),
    ).toBe("あおいがれん、あなたを追加しました");
    expect(systemText(event(chat.SystemEventType.MEMBER_LEFT, { actor: user(ME) }), ME)).toBe(
      "あなたが退出しました",
    );
  });

  it("tells a hand-over from an inherited ownership", () => {
    expect(
      systemText(event(chat.SystemEventType.OWNER_CHANGED, { targets: [user("ren", "れん")] }), ME),
    ).toBe("あおいがれんにオーナーをゆずりました");
    expect(
      systemText(
        event(chat.SystemEventType.OWNER_CHANGED, {
          actor: undefined,
          targets: [user("ren", "れん")],
        }),
        ME,
      ),
    ).toBe("れんがオーナーになりました");
  });

  it("shows a notice from the server in the app's language", () => {
    const notice = (ja: string, en: string) =>
      systemText(event(chat.SystemEventType.NOTICE, { actor: undefined, notice: { ja, en } }), ME);
    expect(notice("このボットの持ち主が変わったよ", "This bot has a new owner")).toBe(
      "このボットの持ち主が変わったよ",
    );
    expect(notice("", "This bot has a new owner")).toBe("This bot has a new owner");
    expect(systemText(event(chat.SystemEventType.NOTICE, { actor: undefined }), ME)).toBe("");
  });

  it("shortens long lists of people", () => {
    const targets = ["a", "b", "c", "d", "e"].map((id) => user(id));
    expect(systemText(event(chat.SystemEventType.MEMBERS_ADDED, { targets }), ME)).toBe(
      "あおいがa、b ほか3人を追加しました",
    );
  });
});

describe("call outcomes", () => {
  const call = (
    endReason: chat.CallEndReason,
    startedById: string,
    extra: MessageInitShape<typeof chat.CallSummarySchema> = {},
  ) => make(chat.CallSummarySchema, { state: chat.CallState.ENDED, endReason, startedById }, extra);

  it("reads differently for the caller and the person called", () => {
    expect(callText(call(chat.CallEndReason.MISSED, "aoi"), ME)).toBe("不在着信");
    expect(callText(call(chat.CallEndReason.MISSED, ME), ME)).toBe("応答なし");
    expect(callText(call(chat.CallEndReason.CANCELLED, "aoi"), ME)).toBe("不在着信");
    expect(callText(call(chat.CallEndReason.CANCELLED, ME), ME)).toBe("キャンセルした通話");
  });

  it("gives the length of a call that happened", () => {
    expect(callText(call(chat.CallEndReason.COMPLETED, ME, { durationSeconds: 205 }), ME)).toBe(
      "音声通話 3:25",
    );
    expect(
      callText(call(chat.CallEndReason.COMPLETED, ME, { durationSeconds: 65, video: true }), ME),
    ).toBe("ビデオ通話 1:05");
    expect(
      callText(create(chat.CallSummarySchema, { state: chat.CallState.ACTIVE, video: true }), ME),
    ).toBe("ビデオ通話中");
  });
});

describe("reply quotes", () => {
  const reply = (extra: MessageInitShape<typeof chat.ReplyReferenceSchema>) =>
    make(chat.ReplyReferenceSchema, { messageId: "m", kind: chat.MessageKind.TEXT }, extra);

  it("quotes the text, or says what the message was", () => {
    expect(replyText(reply({ text: "明日ひま？" }), ME)).toBe("明日ひま？");
    expect(replyText(reply({ kind: chat.MessageKind.STAMP }), ME)).toBe("スタンプを送信しました");
    expect(replyText(reply({ unavailable: true, text: "" }), ME)).toBe(
      "メッセージは削除されました",
    );
  });
});
