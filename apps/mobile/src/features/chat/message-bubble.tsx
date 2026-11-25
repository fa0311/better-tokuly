import { chat, type media, toMillis } from "@tokuly/api";
import { isBubble, type PendingState, tokenize } from "@tokuly/core/chat";
import { Image } from "expo-image";
import * as WebBrowser from "expo-web-browser";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { SERVER_URL } from "#/lib/config";
import { clock, contentText, displayName, replyText, systemText } from "./text";

const absolute = (url: string) => (url.startsWith("/") ? `${SERVER_URL}${url}` : url);

/** The text of a message: links open in the in-app browser, mentions stand out. */
function MessageText({ message, mine }: { message: chat.Message; mine: boolean }) {
  const tokens = tokenize(
    message.text,
    message.mentions.map((mention) => mention.handle),
    message.mentionsEveryone,
  );
  const big = message.bigEmojiCount > 0;
  return (
    <Text
      selectable
      className={`${big ? "text-5xl" : "text-base"} ${mine ? "text-on-brand" : "text-ink"}`}
    >
      {tokens.map((token, index) =>
        token.type === "link" ? (
          <Text
            key={index}
            className="underline"
            onPress={() => void WebBrowser.openBrowserAsync(token.href)}
          >
            {token.text}
          </Text>
        ) : token.type === "text" ? (
          token.text
        ) : (
          <Text key={index} className="font-black">
            {token.text}
          </Text>
        ),
      )}
    </Text>
  );
}

function Photos({ attachments }: { attachments: media.Attachment[] }) {
  const images = attachments.filter((attachment) => attachment.image);
  if (images.length === 0) return null;
  return (
    <View className="flex-row flex-wrap gap-1">
      {images.map((attachment, index) => {
        const image = attachment.image!;
        const ratio = image.width && image.height ? image.width / image.height : 1;
        return (
          <Image
            key={attachment.assetId || `${index}`}
            source={absolute(image.url)}
            placeholder={image.thumbhash ? { thumbhash: image.thumbhash } : undefined}
            accessibilityLabel={attachment.fileName || undefined}
            style={{
              width: images.length === 1 ? 220 : 108,
              aspectRatio: images.length === 1 ? Math.min(2, Math.max(0.5, ratio)) : 1,
              borderRadius: 14,
            }}
            contentFit="cover"
          />
        );
      })}
    </View>
  );
}

/**
 * One message as a bubble (or a centred notice for system messages): the
 * sender's face and name at the start of a run, a quoted reply, the text and
 * photos, reactions, and under the last of a run the time, "edited" and
 * the read mark of the viewer's own messages.
 */
export function MessageBubble({
  message,
  myId,
  first,
  last,
  group,
  readLabel,
  pending,
  onLongPress,
  onReact,
  onRetry,
}: {
  message: chat.Message;
  myId: string;
  first: boolean;
  last: boolean;
  group: boolean;
  /** "既読" / "既読 3" for the viewer's own message, when someone read it. */
  readLabel?: string;
  pending?: PendingState;
  onLongPress(): void;
  onReact(emoji: string): void;
  onRetry(): void;
}) {
  const { t } = useTranslation("chat");
  const { t: tm } = useTranslation("mobile");
  if (!isBubble(message))
    return (
      <Text className="px-8 py-2 text-center text-xs font-bold text-ink-soft">
        {message.kind === chat.MessageKind.SYSTEM
          ? systemText(message.system, myId)
          : contentText(message, myId)}
      </Text>
    );

  const mine = message.sender?.id === myId;
  const hasText = message.text.length > 0 && !message.deleted;
  const otherContent = !hasText && message.attachments.length === 0;
  return (
    <View
      className={`flex-row gap-2 px-3 ${first ? "mt-2" : "mt-0.5"} ${mine ? "justify-end" : ""}`}
    >
      {!mine && group && (
        <View style={{ width: 32 }}>{first && <Avatar user={message.sender} size="sm" />}</View>
      )}
      <View className={`max-w-[78%] gap-1 ${mine ? "items-end" : "items-start"}`}>
        {!mine && group && first && (
          <Text className="px-1 text-xs font-bold text-ink-soft">
            {displayName(message.sender)}
          </Text>
        )}
        <Pressable
          testID={`message-${message.clientId || message.id}`}
          onLongPress={pending ? undefined : onLongPress}
          delayLongPress={300}
          className={`gap-1.5 rounded-3xl px-3.5 py-2 ${mine ? "bg-brand" : "bg-surface"} ${
            pending?.status === "sending" ? "opacity-60" : ""
          }`}
        >
          {message.replyTo && (
            <View
              className={`rounded-xl border-l-4 px-2 py-1 ${mine ? "border-on-brand/60 bg-black/10" : "border-brand bg-surface-2"}`}
            >
              <Text
                numberOfLines={1}
                className={`text-xs font-black ${mine ? "text-on-brand" : "text-ink"}`}
              >
                {displayName(message.replyTo.sender)}
              </Text>
              <Text
                numberOfLines={2}
                className={`text-xs ${mine ? "text-on-brand" : "text-ink-soft"}`}
              >
                {replyText(message.replyTo, myId)}
              </Text>
            </View>
          )}
          {!message.deleted && <Photos attachments={message.attachments} />}
          {message.deleted ? (
            <Text className={`text-sm italic ${mine ? "text-on-brand" : "text-ink-soft"}`}>
              {t("message.deleted")}
            </Text>
          ) : hasText ? (
            <MessageText message={message} mine={mine} />
          ) : otherContent ? (
            <Text className={`text-base ${mine ? "text-on-brand" : "text-ink"}`}>
              {contentText(message, myId)}
            </Text>
          ) : null}
        </Pressable>
        {message.reactions.length > 0 && (
          <View className="flex-row flex-wrap gap-1">
            {message.reactions.map((reaction) => (
              <Pressable
                key={reaction.emoji}
                accessibilityRole="button"
                accessibilityLabel={t("message.reactionCount", {
                  emoji: reaction.emoji,
                  count: reaction.count,
                })}
                accessibilityState={{ selected: reaction.reactedByMe }}
                onPress={() => onReact(reaction.emoji)}
                className={`flex-row items-center gap-1 rounded-full border px-2 py-0.5 ${
                  reaction.reactedByMe ? "border-brand bg-brand-soft" : "border-line bg-surface"
                }`}
              >
                <Text className="text-sm">{reaction.emoji}</Text>
                <Text className="text-xs font-black text-ink">{reaction.count}</Text>
              </Pressable>
            ))}
          </View>
        )}
        {pending?.status === "failed" ? (
          <Pressable accessibilityRole="button" onPress={onRetry} testID="message-retry">
            <Text className="px-1 text-xs font-black text-danger">
              {t("message.failed")} · {t("message.retry")}
            </Text>
          </Pressable>
        ) : pending ? (
          <Text className="px-1 text-xs font-bold text-ink-faint">
            {pending.progress !== undefined && pending.progress < 1
              ? tm("chat.uploading", { percent: Math.round(pending.progress * 100) })
              : tm("chat.sending")}
          </Text>
        ) : (
          last && (
            <Text className="px-1 text-xs font-bold text-ink-faint">
              {mine && readLabel ? `${readLabel} · ` : ""}
              {clock(toMillis(message.createdAt))}
              {message.editedAt && !message.deleted ? ` · ${t("message.edited")}` : ""}
            </Text>
          )
        )}
      </View>
    </View>
  );
}
