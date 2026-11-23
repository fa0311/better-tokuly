import { live } from "@tokuly/api";
import { MoreHorizontal, Swords } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { CoinIcon } from "#/components/ui/coins";
import { EmojiText } from "#/features/emoji";
import { StampImage } from "#/features/stamps";
import { cn } from "#/lib/cn";
import { ROLE_KEYS } from "../labels";
import { tierByLevel, tierStyle } from "../model";
import { ChannelFace, formatCount } from "../parts";

const { ChatMessageKind, ChatRole } = live;

/** Who a chat line is from, as text: the channel for the streamer, the person otherwise. */
export function authorName(message: live.ChatMessage): string {
  return message.author?.channel?.name || message.author?.user?.displayName || "";
}

/** The `@handle` a reply to this line starts with; empty for the streamer (a channel has no account to mention). */
export function mentionOf(message: live.ChatMessage): string {
  const user = message.author?.user;
  return user && !user.deleted ? `@${user.handle} ` : "";
}

function AuthorFace({ message }: { message: live.ChatMessage }) {
  const channel = message.author?.channel;
  if (channel) return <ChannelFace channel={channel} size="xs" className="mt-0.5 shrink-0" />;
  return <Avatar user={message.author?.user} size="xs" className="mt-0.5 shrink-0" />;
}

/** The author's name with the badge of their standing on the channel (spelled out, not only coloured). */
function AuthorName({ message, className }: { message: live.ChatMessage; className?: string }) {
  const { t } = useTranslation("live");
  const roleKey = ROLE_KEYS[message.role];
  return (
    <span className={cn("inline-flex max-w-full items-center gap-1 align-baseline", className)}>
      <span
        className={cn(
          "truncate text-[13px] font-black",
          message.role === ChatRole.OWNER
            ? "text-live"
            : message.role === ChatRole.MODERATOR
              ? "text-brand-strong"
              : "text-ink-soft",
        )}
      >
        <EmojiText text={authorName(message)} />
      </span>
      {roleKey && (
        <span
          className={cn(
            "shrink-0 rounded px-1 text-[10px] leading-4 font-black",
            message.role === ChatRole.OWNER
              ? "bg-live text-white"
              : message.role === ChatRole.MODERATOR
                ? "bg-brand text-on-brand"
                : "bg-surface-3 text-ink-soft",
          )}
        >
          {t(`chatRole.${roleKey}`)}
        </span>
      )}
    </span>
  );
}

/** Text with the `@handle`s in it marked; the reader's own stands out. */
function MessageText({ text, selfHandle }: { text: string; selfHandle?: string }) {
  const parts = text.split(/(@[A-Za-z0-9_-]{3,24})/g);
  return (
    <>
      {parts.map((part, index) => {
        if (!part.startsWith("@") || index % 2 === 0) return <EmojiText key={index} text={part} />;
        const mine = selfHandle !== undefined && part.slice(1).toLowerCase() === selfHandle;
        return (
          <span
            key={index}
            className={cn(
              "font-black",
              mine ? "rounded bg-sun px-0.5 text-[#17122b]" : "text-brand-strong",
            )}
          >
            {part}
          </span>
        );
      })}
    </>
  );
}

/** A gift (super chat) as it is drawn in chat, above it and in previews: the tier's colours, the amount, the words. */
export function GiftBubble({
  message,
  tiers,
  compact,
  className,
}: {
  message: live.ChatMessage;
  tiers: live.GiftTier[];
  /** One line: the chip above the chat. */
  compact?: boolean;
  className?: string;
}) {
  const { t, i18n } = useTranslation("live");
  const gift = message.gift;
  const style = tierStyle(tierByLevel(tiers, gift?.tier ?? 0));
  const coins = formatCount(gift?.coins ?? 0n, i18n.language);
  if (compact) {
    return (
      <span
        className={cn(
          "inline-flex h-8 max-w-40 shrink-0 items-center gap-1.5 rounded-full border-2 border-ink pr-3 pl-1 text-xs font-black",
          className,
        )}
        style={style}
        data-testid="gift-highlight"
        data-tier={gift?.tier}
      >
        <Avatar user={message.author?.user} size="xs" />
        <span className="truncate">{coins}</span>
      </span>
    );
  }
  return (
    <div
      className={cn("sticker-sm overflow-hidden rounded-2xl", className)}
      style={style}
      data-testid="gift-message"
      data-tier={gift?.tier}
    >
      <div className="flex items-center gap-2 px-3 py-2">
        <Avatar user={message.author?.user} size="sm" />
        <div className="min-w-0 flex-1">
          <div className="truncate text-[13px] font-black opacity-90">
            <EmojiText text={authorName(message)} />
          </div>
          <div
            className="flex items-center gap-1 text-base font-black"
            title={t("gift.coinsLabel", { coins })}
          >
            <CoinIcon className="size-4 text-[9px]" />
            {coins}
          </div>
        </div>
      </div>
      {message.text && (
        <p className="border-t-2 border-black/15 px-3 py-2 text-sm leading-relaxed font-bold break-words">
          <EmojiText text={message.text} />
        </p>
      )}
    </div>
  );
}

/** One line of the live chat: a message, a stamp, a gift or something the server announces. */
export function ChatLine({
  message,
  tiers,
  selfHandle,
  onMore,
  onMention,
  onOpenStamp,
}: {
  message: live.ChatMessage;
  tiers: live.GiftTier[];
  /** The reader's handle in lower case, to mark lines that mention them. */
  selfHandle?: string;
  /** Opens the actions for this line; left out where there are none (a replay, an overlay). */
  onMore?(message: live.ChatMessage): void;
  onMention?(message: live.ChatMessage): void;
  onOpenStamp?(stampId: string): void;
}) {
  const { t, i18n } = useTranslation("live");

  if (message.kind === ChatMessageKind.NOTICE) {
    const notice = message.notice;
    if (notice?.kind !== live.ChatNoticeKind.RAID) return null;
    return (
      <div className="px-3 py-1.5" data-testid="chat-notice">
        <p className="flex items-center justify-center gap-2 rounded-2xl bg-brand-soft px-3 py-2 text-center text-sm font-black text-brand-strong">
          <Swords className="size-4 shrink-0" />
          {t("chat.raidNotice", {
            name: notice.channel?.name ?? "",
            formatted: formatCount(notice.viewerCount, i18n.language),
          })}
        </p>
      </div>
    );
  }

  const more = onMore && (
    <button
      type="button"
      aria-label={t("chat.actions")}
      onClick={() => onMore(message)}
      className="grid size-7 shrink-0 place-items-center rounded-full text-ink-faint opacity-0 transition-opacity hover:bg-surface-3 hover:text-ink focus-visible:opacity-100 group-hover/line:opacity-100 [@media(hover:none)]:opacity-100"
    >
      <MoreHorizontal className="size-4" />
    </button>
  );

  if (message.kind === ChatMessageKind.GIFT) {
    return (
      <div className="group/line flex items-start gap-1 px-3 py-1.5" data-testid="chat-line">
        <GiftBubble message={message} tiers={tiers} className="min-w-0 flex-1" />
        {more}
      </div>
    );
  }

  const name = (
    <button
      type="button"
      className="max-w-full text-left align-baseline disabled:cursor-default"
      disabled={!onMention || !mentionOf(message)}
      onClick={() => onMention?.(message)}
    >
      <AuthorName message={message} />
    </button>
  );
  return (
    <div
      className="group/line flex items-start gap-2 px-3 py-1 hover:bg-surface-2"
      data-testid="chat-line"
    >
      <AuthorFace message={message} />
      <div className="min-w-0 flex-1 text-[15px] leading-snug">
        {name}
        {message.kind === ChatMessageKind.STAMP ? (
          message.stamp ? (
            <button
              type="button"
              className="mt-0.5 block"
              onClick={() => message.stamp && onOpenStamp?.(message.stamp.id)}
              aria-label={message.stamp.name}
            >
              <StampImage stamp={message.stamp} size={72} />
            </button>
          ) : (
            <span className="ml-1.5 text-sm text-ink-faint">{t("chat.stampGone")}</span>
          )
        ) : (
          <span className="ml-1.5 break-words">
            <MessageText text={message.text} selfHandle={selfHandle} />
          </span>
        )}
      </div>
      {more}
    </div>
  );
}
