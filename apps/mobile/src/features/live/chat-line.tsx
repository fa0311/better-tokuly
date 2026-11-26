import { live } from "@tokuly/api";
import { compactCount } from "@tokuly/core/engagement";
import { authorName, chatTextParts, mentionOf, tierByLevel } from "@tokuly/core/live";
import { Image } from "expo-image";
import { Swords } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { mediaUrl } from "#/lib/config";
import { ChannelFace } from "./parts";

const { ChatMessageKind, ChatRole } = live;

/** A tier's colours for a gift: its flat colour, or the first of the top tier's gradient. */
export function tierColors(tier: live.GiftTier | undefined): { background: string; text: string } {
  if (!tier) return { background: "#e4def5", text: "#17122b" };
  return { background: tier.gradientColors[0] || tier.backgroundColor, text: tier.textColor };
}

const ROLE_KEYS: Partial<Record<live.ChatRole, string>> = {
  [ChatRole.OWNER]: "owner",
  [ChatRole.MODERATOR]: "moderator",
  [ChatRole.SUBSCRIBER]: "subscriber",
};

function AuthorFace({ message }: { message: live.ChatMessage }) {
  const channel = message.author?.channel;
  if (channel) return <ChannelFace channel={channel} size="xs" />;
  return <Avatar user={message.author?.user} size="xs" />;
}

/** The author's name with the badge of their standing on the channel (spelled out, not only coloured). */
function AuthorName({ message }: { message: live.ChatMessage }) {
  const { t } = useTranslation("live");
  const roleKey = ROLE_KEYS[message.role];
  const owner = message.role === ChatRole.OWNER;
  const moderator = message.role === ChatRole.MODERATOR;
  return (
    <View className="flex-row items-center gap-1">
      <Text
        numberOfLines={1}
        className={`shrink text-[13px] font-black ${owner ? "text-live" : moderator ? "text-brand-strong" : "text-ink-soft"}`}
      >
        {authorName(message)}
      </Text>
      {roleKey && (
        <Text
          className={`rounded px-1 text-[10px] font-black ${owner ? "bg-live text-white" : moderator ? "bg-brand text-on-brand" : "bg-surface-3 text-ink-soft"}`}
        >
          {t(`chatRole.${roleKey}`)}
        </Text>
      )}
    </View>
  );
}

/** A gift (super chat) in the tier's colours: who, how many coins, and the words. */
export function GiftBubble({
  message,
  tiers,
  compact,
}: {
  message: live.ChatMessage;
  tiers: live.GiftTier[];
  /** One line: the chip above the chat. */
  compact?: boolean;
}) {
  const { t, i18n } = useTranslation("live");
  const gift = message.gift;
  const colors = tierColors(tierByLevel(tiers, gift?.tier ?? 0));
  const coins = compactCount(Number(gift?.coins ?? 0n), i18n.language);
  if (compact)
    return (
      <View
        testID="gift-highlight"
        className="h-8 max-w-40 flex-row items-center gap-1.5 rounded-full border-2 border-ink pr-3 pl-1"
        style={{ backgroundColor: colors.background }}
      >
        <Avatar user={message.author?.user} size="xs" />
        <Text numberOfLines={1} className="text-xs font-black" style={{ color: colors.text }}>
          🪙 {coins}
        </Text>
      </View>
    );
  return (
    <View
      testID="gift-bubble"
      className="overflow-hidden rounded-2xl border-2 border-ink"
      style={{ backgroundColor: colors.background }}
    >
      <View className="flex-row items-center gap-2 px-3 py-2">
        <Avatar user={message.author?.user} size="sm" />
        <View className="min-w-0 flex-1">
          <Text numberOfLines={1} className="text-[13px] font-black" style={{ color: colors.text }}>
            {authorName(message)}
          </Text>
          <Text
            accessibilityLabel={t("gift.coinsLabel", { coins })}
            className="text-base font-black"
            style={{ color: colors.text }}
          >
            🪙 {coins}
          </Text>
        </View>
      </View>
      {message.text !== "" && (
        <Text
          className="border-t-2 border-black/15 px-3 py-2 text-sm font-bold"
          style={{ color: colors.text }}
        >
          {message.text}
        </Text>
      )}
    </View>
  );
}

/** Text with the `@handle`s in it marked; the reader's own stands out. */
function MessageText({ text, selfHandle }: { text: string; selfHandle?: string }) {
  return (
    <Text className="text-[15px] leading-snug text-ink">
      {chatTextParts(text, selfHandle).map((part, index) =>
        part.mention ? (
          <Text
            key={index}
            className={`font-black ${part.mine ? "bg-sun text-[#17122b]" : "text-brand-strong"}`}
          >
            {part.text}
          </Text>
        ) : (
          part.text
        ),
      )}
    </Text>
  );
}

/**
 * One line of the live chat: a message, a stamp, a gift or something the
 * server announces. A long press opens what can be done with it; a tap on
 * the name starts a reply.
 */
export function ChatLine({
  message,
  tiers,
  selfHandle,
  onMention,
  onMore,
}: {
  message: live.ChatMessage;
  tiers: live.GiftTier[];
  /** The reader's handle in lower case, to mark lines that mention them. */
  selfHandle?: string;
  onMention?(message: live.ChatMessage): void;
  onMore?(message: live.ChatMessage): void;
}) {
  const { t, i18n } = useTranslation("live");

  if (message.kind === ChatMessageKind.NOTICE) {
    const notice = message.notice;
    if (notice?.kind !== live.ChatNoticeKind.RAID) return null;
    return (
      <View testID="chat-notice" className="px-3 py-1.5">
        <View className="flex-row items-center justify-center gap-2 rounded-2xl bg-brand-soft px-3 py-2">
          <Swords size={16} color="#5f3df0" />
          <Text className="shrink text-center text-sm font-black text-brand-strong">
            {t("chat.raidNotice", {
              name: notice.channel?.name ?? "",
              formatted: compactCount(Number(notice.viewerCount), i18n.language),
            })}
          </Text>
        </View>
      </View>
    );
  }

  if (message.kind === ChatMessageKind.GIFT)
    return (
      <Pressable
        testID="chat-line"
        onLongPress={onMore && (() => onMore(message))}
        className="px-3 py-1.5"
      >
        <GiftBubble message={message} tiers={tiers} />
      </Pressable>
    );

  const stamp = message.stamp;
  const stampUrl = mediaUrl(stamp?.imageUrl);
  return (
    <Pressable
      testID="chat-line"
      onLongPress={onMore && (() => onMore(message))}
      className="flex-row items-start gap-2 px-3 py-1"
    >
      <AuthorFace message={message} />
      <View className="min-w-0 flex-1">
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={t("chat.mention")}
          disabled={!onMention || !mentionOf(message)}
          onPress={() => onMention?.(message)}
        >
          <AuthorName message={message} />
        </Pressable>
        {message.kind === ChatMessageKind.STAMP ? (
          stampUrl ? (
            <Image
              source={stampUrl}
              accessibilityLabel={stamp?.name}
              style={{ width: 72, height: 72 }}
              contentFit="contain"
            />
          ) : (
            <Text className="text-sm text-ink-faint">{t("chat.stampGone")}</Text>
          )
        ) : (
          <MessageText text={message.text} selfHandle={selfHandle} />
        )}
      </View>
    </Pressable>
  );
}
