import { chat, type common, media } from "@tokuly/api";
import type { i18n as I18n } from "i18next";
import { playTime } from "./text";

/**
 * Sentences built from messages, in the language of `i18n`: the list
 * preview, system notices, call outcomes.
 */
export function chatSentences(i18n: I18n) {
  const t = (key: string, options?: Record<string, unknown>) =>
    i18n.t(key, { ns: "chat", ...options });

  /** A person's name as shown in a sentence ("あなた" for the viewer). */
  function nameOf(user: common.UserSummary | undefined, myId: string): string {
    if (!user) return t("people.someone");
    if (user.id === myId) return t("people.you");
    if (user.deleted) return t("people.deleted");
    return user.displayName || `@${user.handle}`;
  }

  function namesOf(users: common.UserSummary[], myId: string): string {
    const names = users.map((user) => nameOf(user, myId));
    if (names.length <= 3) return names.join(t("people.separator"));
    return t("people.andMore", {
      names: names.slice(0, 2).join(t("people.separator")),
      count: names.length - 2,
    });
  }

  /** A notice from the server, in the language the app is shown in (or whichever it has). */
  function noticeText(text: common.LocalizedText | undefined): string {
    if (!text) return "";
    return (i18n.language.startsWith("ja") ? text.ja : text.en) || text.ja || text.en;
  }

  /** The sentence of a system message ("Aoi added Ren"). */
  function systemText(event: chat.SystemEvent | undefined, myId: string): string {
    if (!event) return "";
    const actor = nameOf(event.actor, myId);
    const targets = namesOf(event.targets, myId);
    switch (event.type) {
      case chat.SystemEventType.GROUP_CREATED:
        return t("system.groupCreated", { actor, name: event.text });
      case chat.SystemEventType.MEMBERS_ADDED:
        return t("system.membersAdded", { actor, targets });
      case chat.SystemEventType.MEMBER_LEFT:
        return t("system.memberLeft", { actor });
      case chat.SystemEventType.MEMBER_REMOVED:
        return t("system.memberRemoved", { actor, targets });
      case chat.SystemEventType.GROUP_RENAMED:
        return t("system.renamed", { actor, name: event.text });
      case chat.SystemEventType.ICON_CHANGED:
        return t("system.iconChanged", { actor });
      case chat.SystemEventType.OWNER_CHANGED:
        return event.actor
          ? t("system.ownerHanded", { actor, targets })
          : t("system.ownerInherited", { targets });
      case chat.SystemEventType.ADMIN_ADDED:
        return t("system.adminAdded", { actor, targets });
      case chat.SystemEventType.ADMIN_REMOVED:
        return t("system.adminRemoved", { actor, targets });
      case chat.SystemEventType.MESSAGE_PINNED:
        return t("system.pinned", { actor });
      case chat.SystemEventType.NOTICE:
        return noticeText(event.notice);
      default:
        return "";
    }
  }

  /** How a call turned out, from the viewer's side ("不在着信", "通話 3:25"). */
  function callText(call: chat.CallSummary | undefined, myId: string): string {
    if (!call) return "";
    const outgoing = call.startedById === myId;
    if (call.state !== chat.CallState.ENDED)
      return call.video ? t("call.summary.ongoingVideo") : t("call.summary.ongoing");
    switch (call.endReason) {
      case chat.CallEndReason.COMPLETED:
        return t(call.video ? "call.summary.video" : "call.summary.voice", {
          duration: playTime(call.durationSeconds),
        });
      case chat.CallEndReason.MISSED:
        return outgoing ? t("call.summary.noAnswer") : t("call.summary.missed");
      case chat.CallEndReason.DECLINED:
        return outgoing ? t("call.summary.declinedByThem") : t("call.summary.declined");
      case chat.CallEndReason.CANCELLED:
        return outgoing ? t("call.summary.cancelled") : t("call.summary.missed");
      default:
        return t("call.summary.failed");
    }
  }

  /** What a media message holds, in a word ("写真", "動画 2件"). */
  function attachmentsText(attachments: media.Attachment[]): string {
    const first = attachments[0];
    if (!first) return t("preview.media");
    const count = attachments.length;
    switch (first.kind) {
      case media.AssetKind.IMAGE:
      case media.AssetKind.ANIMATION:
        return count > 1 ? t("preview.photos", { count }) : t("preview.photo");
      case media.AssetKind.VIDEO:
        return count > 1 ? t("preview.videos", { count }) : t("preview.video");
      case media.AssetKind.AUDIO:
        return t("preview.voice");
      default:
        return count > 1 ? t("preview.files", { count }) : first.fileName || t("preview.file");
    }
  }

  /** The content of a message in one line, without saying who sent it. */
  function contentText(
    message: Pick<
      chat.Message,
      "kind" | "text" | "deleted" | "attachments" | "system" | "call" | "povReply" | "stamp"
    >,
    myId: string,
  ): string {
    if (message.deleted) return t("preview.deleted");
    switch (message.kind) {
      case chat.MessageKind.TEXT:
        return message.text.replace(/\s+/g, " ").trim();
      case chat.MessageKind.STAMP:
        return t("preview.stamp");
      case chat.MessageKind.MEDIA: {
        if (message.povReply) return t("preview.povReply");
        const what = attachmentsText(message.attachments);
        const caption = message.text.replace(/\s+/g, " ").trim();
        return caption ? `${what} ${caption}` : what;
      }
      case chat.MessageKind.LOCATION:
        return t("preview.location");
      case chat.MessageKind.HELLO:
        return t("preview.hello");
      case chat.MessageKind.SYSTEM:
        return systemText(message.system, myId);
      case chat.MessageKind.CALL:
        return callText(message.call, myId);
      case chat.MessageKind.POV_REQUEST:
        return t("preview.povRequest");
      default:
        return "";
    }
  }

  /** The preview under a conversation's name: "あなた: …" for own messages, the sender's name in groups. */
  function previewText(message: chat.Message | undefined, myId: string, group: boolean): string {
    if (!message) return "";
    const content = contentText(message, myId);
    if (
      message.kind === chat.MessageKind.SYSTEM ||
      message.kind === chat.MessageKind.CALL ||
      !message.sender
    )
      return content;
    if (message.sender.id === myId) return t("preview.fromYou", { content });
    return group ? t("preview.from", { name: nameOf(message.sender, myId), content }) : content;
  }

  /** The line quoted above a reply. */
  function replyText(reply: chat.ReplyReference, myId: string): string {
    if (reply.unavailable) return t("reply.unavailable");
    if (reply.text) return reply.text;
    switch (reply.kind) {
      case chat.MessageKind.STAMP:
        return t("preview.stamp");
      case chat.MessageKind.MEDIA:
        return t("preview.media");
      case chat.MessageKind.LOCATION:
        return t("preview.location");
      case chat.MessageKind.HELLO:
        return t("preview.hello");
      default:
        return contentText(
          {
            kind: reply.kind,
            text: reply.text,
            deleted: false,
            attachments: [],
            system: undefined,
            call: undefined,
            povReply: undefined,
            stamp: undefined,
          },
          myId,
        );
    }
  }

  return { nameOf, systemText, callText, attachmentsText, contentText, previewText, replyText };
}
