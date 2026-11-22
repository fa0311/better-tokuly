import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { chat, toMillis } from "@tokuly/api";
import {
  CircleAlert,
  Copy,
  CornerUpLeft,
  CornerUpRight,
  Eye,
  Flag,
  Languages,
  MoreHorizontal,
  Pencil,
  Pin,
  PinOff,
  RotateCw,
  SmilePlus,
  Trash2,
} from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { Spinner } from "#/components/ui/spinner";
import { EmojiText } from "#/features/emoji";
import { StampImage } from "#/features/stamps";
import { cn } from "#/lib/cn";
import { TranslationBlock, toggleTranslation, useIsTranslated } from "./assist";
import { useConversationContext } from "./conversation-context";
import { QUICK_REACTIONS } from "./emoji";
import { errorMessage } from "./errors";
import { useLive } from "./live";
import { isVisual } from "./media-parts";
import {
  ForwardedLabel,
  LinkPreviewCard,
  LocationCard,
  MediaContent,
  MessageText,
  ReplyQuote,
} from "./message-content";
import { useMessageActions } from "./messages";
import { abilitiesOf, isGroup, type MessageAbilities } from "./model";
import { CallCard, SystemLine } from "./notice-rows";
import { discard, retry, send, useOutgoing } from "./outbox";
import { ContextActions, useHover } from "./parts";
import { displayName } from "./people";
import { PovReplyFooter, PovRequestCard } from "./pov";
import { clock } from "./text";
import type { PendingState, Row } from "./timeline";

type MessageRowData = Extract<Row, { type: "message" }>;

/** One message of the conversation: a centred notice, or a bubble with everything around it. */
export function MessageItem({
  row,
  highlighted,
  onOpenLocation,
}: {
  row: MessageRowData;
  highlighted: boolean;
  onOpenLocation(location: chat.LocationShare): void;
}) {
  const { message } = row;
  if (message.kind === chat.MessageKind.SYSTEM)
    return <Notice id={message.id}>{<SystemLine message={message} />}</Notice>;
  if (message.kind === chat.MessageKind.CALL)
    return <Notice id={message.id}>{<CallCard message={message} />}</Notice>;
  if (message.kind === chat.MessageKind.POV_REQUEST)
    return <Notice id={message.id}>{<PovRequestCard message={message} />}</Notice>;
  return <BubbleRow row={row} highlighted={highlighted} onOpenLocation={onOpenLocation} />;
}

function Notice({ id, children }: { id: string; children: ReactNode }) {
  return (
    <div className="px-3 py-2" data-message-id={id}>
      {children}
    </div>
  );
}

/** How many other members have read up to this message. */
function useReaders(conversationId: string, message: chat.Message, myId: string): number {
  return useLive((state) => {
    const cursors = state.cursors[conversationId];
    if (!cursors) return 0;
    let readers = 0;
    for (const userId in cursors) if (userId !== myId && cursors[userId]! >= message.seq) readers++;
    return readers;
  });
}

/** What the viewer may do with a message right now. */
function useAbilities(message: chat.Message): MessageAbilities {
  const context = useConversationContext();
  return abilitiesOf(message, {
    myId: context.me.id,
    role: context.conversation.viewer?.role,
    writable: context.writable,
    accepted: context.accepted,
    editWindowSeconds: context.capabilities.editWindowSeconds,
  });
}

/** The actions on a message, as menu entries (the same in the hover bar's menu and the long-press menu). */
function useMessageMenu(
  message: chat.Message,
  can: MessageAbilities,
  pending: PendingState | undefined,
): MenuAction[] {
  const { t } = useTranslation("chat");
  const context = useConversationContext();
  const actions = useMessageActions();
  const translated = useIsTranslated(message.id);
  if (pending) return [];
  const own = message.sender?.id === context.me.id;
  const entries: MenuAction[] = [];
  const guarded = (work: () => Promise<void>) => () =>
    void work().catch((error) => toast.error(errorMessage(error)));

  if (can.reply)
    entries.push({
      label: t("message.reply"),
      icon: <CornerUpLeft />,
      onSelect: () => context.reply(message),
    });
  if (can.copy)
    entries.push({
      label: t("message.copy"),
      icon: <Copy />,
      onSelect: guarded(async () => {
        await navigator.clipboard.writeText(message.text);
        toast.success(t("toast.copied"));
      }),
    });
  if (can.edit)
    entries.push({
      label: t("message.edit"),
      icon: <Pencil />,
      onSelect: () => context.edit(message),
    });
  if (can.forward)
    entries.push({
      label: t("message.forward"),
      icon: <CornerUpRight />,
      onSelect: () => context.forward(message),
    });
  if (can.pin)
    entries.push(
      message.pinned
        ? {
            label: t("message.unpin"),
            icon: <PinOff />,
            onSelect: guarded(() => actions.pin(message, false)),
          }
        : {
            label: t("message.pin"),
            icon: <Pin />,
            onSelect: guarded(() => actions.pin(message, true)),
          },
    );
  if (can.translate && context.capabilities.assistEnabled)
    entries.push({
      label: translated ? t("assist.hideTranslation") : t("assist.translate"),
      icon: <Languages />,
      onSelect: () => toggleTranslation(message.id),
    });
  if (own && isGroup(context.conversation) && !message.deleted)
    entries.push({
      label: t("message.readBy"),
      icon: <Eye />,
      onSelect: () => context.showReceipts(message),
    });
  if (message.reactions.length > 0)
    entries.push({
      label: t("message.reactions"),
      icon: <SmilePlus />,
      onSelect: () => context.showReactions(message),
    });
  if (!own && !message.deleted && message.sender)
    entries.push({
      label: t("message.report"),
      icon: <Flag />,
      onSelect: () => context.report(message),
    });
  if (can.delete)
    entries.push({
      label: t("message.delete"),
      icon: <Trash2 />,
      danger: true,
      onSelect: () => context.remove(message),
    });
  return entries;
}

function BubbleRow({
  row,
  highlighted,
  onOpenLocation,
}: {
  row: MessageRowData;
  highlighted: boolean;
  onOpenLocation(location: chat.LocationShare): void;
}) {
  const { message, pending } = row;
  const { t } = useTranslation("chat");
  const context = useConversationContext();
  const { conversation, me } = context;
  const actions = useMessageActions();
  const hover = useHover();
  const own = message.sender?.id === me.id;
  const tone = own ? "own" : "other";
  const group = isGroup(conversation);
  const can = useAbilities(message);
  const menu = useMessageMenu(message, can, pending);
  const readers = useReaders(conversation.id, message, me.id);
  const canReact = !pending && can.react;

  const react = (emoji: string) => {
    actions.react(message, emoji).catch((error) => toast.error(errorMessage(error)));
  };
  const quick = canReact
    ? {
        label: t("message.react"),
        items: [
          ...QUICK_REACTIONS.map((emoji) => ({
            key: emoji,
            label: t("message.reactWith", { emoji }),
            content: emoji as ReactNode,
            onSelect: () => react(emoji),
          })),
          {
            key: "more",
            label: t("message.moreReactions"),
            content: <SmilePlus />,
            onSelect: () => context.pickReaction(message),
          },
        ],
      }
    : undefined;

  return (
    <div
      className={cn(
        "flex gap-2 px-3 transition-colors duration-700",
        own && "flex-row-reverse",
        row.first ? "pt-2.5" : "pt-0.5",
        highlighted && "bg-brand-soft",
      )}
      data-message-id={message.id}
      data-testid="message"
      data-own={own || undefined}
      data-pending={pending?.status}
    >
      {!own && (
        <div className="w-8 shrink-0">
          {row.first && message.sender && (
            <Link
              to="/u/$handle"
              params={{ handle: message.sender.handle }}
              aria-label={displayName(message.sender)}
            >
              <Avatar user={message.sender} size="sm" />
            </Link>
          )}
        </div>
      )}
      <div
        className={cn(
          "flex min-w-0 max-w-[80%] flex-col md:max-w-[70%]",
          own ? "items-end" : "items-start",
        )}
      >
        {group && !own && row.first && (
          <span className="max-w-full truncate px-1 pb-0.5 text-xs font-bold text-ink-soft">
            <EmojiText text={displayName(message.sender)} />
          </span>
        )}
        <div
          className={cn(
            "group/bubble relative flex max-w-full items-end gap-1.5",
            own && "flex-row-reverse",
          )}
        >
          <ContextActions actions={menu} quick={quick}>
            <div className="min-w-0 max-w-full">
              <Bubble
                message={message}
                tone={tone}
                last={row.last}
                onOpenLocation={onOpenLocation}
              />
            </div>
          </ContextActions>
          <Meta
            message={message}
            pending={pending}
            own={own}
            group={group}
            readers={readers}
            show={row.last}
          />
          {hover && (quick || menu.length > 0) && (
            <div
              className={cn(
                // Just above the bubble (touching it, so the pointer can travel there), never over its text.
                "sticker-sm absolute bottom-[calc(100%-0.25rem)] z-10 hidden items-center rounded-full bg-surface p-0.5 group-focus-within/bubble:flex group-hover/bubble:flex",
                own ? "right-2" : "left-2",
              )}
            >
              {canReact &&
                QUICK_REACTIONS.slice(0, 3).map((emoji) => (
                  <button
                    key={emoji}
                    type="button"
                    aria-label={t("message.reactWith", { emoji })}
                    onClick={() => react(emoji)}
                    className="grid size-8 place-items-center rounded-full text-lg transition-transform hover:scale-125"
                  >
                    {emoji}
                  </button>
                ))}
              {canReact && (
                <button
                  type="button"
                  aria-label={t("message.moreReactions")}
                  onClick={() => context.pickReaction(message)}
                  className="grid size-8 place-items-center rounded-full text-ink-soft hover:bg-surface-2"
                >
                  <SmilePlus className="size-[18px]" />
                </button>
              )}
              {!pending && can.reply && (
                <button
                  type="button"
                  aria-label={t("message.reply")}
                  onClick={() => context.reply(message)}
                  className="grid size-8 place-items-center rounded-full text-ink-soft hover:bg-surface-2"
                >
                  <CornerUpLeft className="size-[18px]" />
                </button>
              )}
              {menu.length > 0 && (
                <Menu
                  align={own ? "end" : "start"}
                  actions={menu}
                  trigger={
                    <button
                      type="button"
                      aria-label={t("message.more")}
                      className="grid size-8 place-items-center rounded-full text-ink-soft hover:bg-surface-2"
                    >
                      <MoreHorizontal className="size-[18px]" />
                    </button>
                  }
                />
              )}
            </div>
          )}
        </div>
        {message.reactions.length > 0 && (
          <Reactions message={message} canReact={canReact} onReact={react} />
        )}
        <TranslationBlock message={message} tone={tone} />
        {pending?.status === "failed" && <FailedLine message={message} error={pending.error} />}
      </div>
    </div>
  );
}

/** The content of a message in the shape its kind calls for. */
function Bubble({
  message,
  tone,
  last,
  onOpenLocation,
}: {
  message: chat.Message;
  tone: "own" | "other";
  last: boolean;
  onOpenLocation(location: chat.LocationShare): void;
}) {
  const { t } = useTranslation("chat");
  const context = useConversationContext();
  const queryClient = useQueryClient();
  const own = tone === "own";
  const shape = cn(
    "rounded-[1.25rem] px-3.5 py-2 text-[15px] leading-relaxed",
    own ? "bg-brand text-on-brand" : "border-2 border-line bg-surface text-ink",
    last && (own ? "rounded-br-md" : "rounded-bl-md"),
  );
  const quote = message.replyTo && <ReplyQuote reply={message.replyTo} tone={tone} />;
  const forwarded = message.forwarded && <ForwardedLabel tone={tone} />;

  if (message.deleted)
    return (
      <p className="rounded-[1.25rem] border-2 border-dashed border-line px-3.5 py-2 text-sm text-ink-faint italic">
        {t("message.deleted")}
      </p>
    );

  switch (message.kind) {
    case chat.MessageKind.STAMP:
      return (
        <div className={cn("flex flex-col", own ? "items-end" : "items-start")}>
          {(quote || forwarded) && (
            <div className={cn(shape, "mb-1")}>
              {forwarded}
              {quote}
            </div>
          )}
          {message.stamp ? (
            <button
              type="button"
              onClick={() => context.openStamp(message.stamp!.id)}
              aria-label={t("message.stamp", { name: message.stamp.name })}
              className="block transition-transform active:scale-95"
              data-testid="stamp-message"
            >
              <StampImage stamp={message.stamp} size={136} />
            </button>
          ) : (
            <p className={cn(shape, "text-sm italic opacity-80")}>{t("message.stampGone")}</p>
          )}
        </div>
      );

    case chat.MessageKind.HELLO:
      return (
        <div
          className={cn("flex flex-col gap-1", own ? "items-end" : "items-start")}
          data-testid="hello-message"
        >
          <span className="inline-block origin-[70%_70%] animate-wiggle text-6xl">👋</span>
          <p className="text-sm font-bold text-ink-soft">
            {own ? t("hello.sent") : t("hello.received", { name: displayName(message.sender) })}
          </p>
          {!own && context.writable && (
            <button
              type="button"
              onClick={() => send(queryClient, context.conversation.id, { text: "👋" })}
              className="pressable sticker-sm rounded-full bg-lime px-3 py-1 text-sm font-black text-[#17122b]"
            >
              {t("hello.back")}
            </button>
          )}
        </div>
      );

    case chat.MessageKind.LOCATION:
      return message.location ? (
        <div>
          {(quote || forwarded) && (
            <div className={cn(shape, "mb-1")}>
              {forwarded}
              {quote}
            </div>
          )}
          <LocationCard
            location={message.location}
            onOpen={() => onOpenLocation(message.location!)}
          />
        </div>
      ) : null;

    case chat.MessageKind.MEDIA: {
      if (message.povReply?.expired)
        return (
          <p className="rounded-[1.25rem] border-2 border-dashed border-line px-3.5 py-2 text-sm text-ink-faint italic">
            {t("pov.replyGone")}
          </p>
        );
      const framed = Boolean(
        message.text ||
          quote ||
          forwarded ||
          message.povReply ||
          message.attachments.some((attachment) => !isVisual(attachment)),
      );
      const content = (
        <>
          {forwarded}
          {quote}
          <div className={cn("flex flex-col gap-1.5", own ? "items-end" : "items-start")}>
            <MediaContent message={message} tone={tone} />
          </div>
          {message.text && (
            <div className="mt-1.5">
              <MessageText message={message} tone={tone} />
            </div>
          )}
          {message.povReply && <PovReplyFooter message={message} tone={tone} />}
        </>
      );
      // Pictures alone stand without a bubble; anything said about them puts the lot in one.
      return framed ? <div className={cn(shape, "px-2 pt-2")}>{content}</div> : content;
    }

    default: {
      const big = message.bigEmojiCount > 0 && !message.replyTo && !message.forwarded;
      if (big)
        return (
          <p className={cn("leading-none", message.bigEmojiCount === 1 ? "text-6xl" : "text-5xl")}>
            {message.text}
          </p>
        );
      return (
        <div className={shape}>
          {forwarded}
          {quote}
          <MessageText message={message} tone={tone} />
          {message.linkPreview && <LinkPreviewCard preview={message.linkPreview} tone={tone} />}
        </div>
      );
    }
  }
}

/** Beside the bubble: sending state, "read", "edited" and the time. */
function Meta({
  message,
  pending,
  own,
  group,
  readers,
  show,
}: {
  message: chat.Message;
  pending: PendingState | undefined;
  own: boolean;
  group: boolean;
  readers: number;
  show: boolean;
}) {
  const { t } = useTranslation("chat");
  const { showReceipts } = useConversationContext();
  if (pending) {
    return (
      <span className="shrink-0 pb-0.5 text-ink-faint">
        {pending.status === "failed" ? (
          <CircleAlert className="size-4 text-danger" aria-label={t("message.failed")} />
        ) : pending.progress !== undefined && pending.progress < 1 ? (
          <span className="text-[10px] font-bold tabular-nums" role="status">
            {Math.round(pending.progress * 100)}%
          </span>
        ) : (
          <Spinner className="size-3.5" />
        )}
      </span>
    );
  }
  const read = own && readers > 0 && !message.deleted;
  if (!show && !read && !message.editedAt && !message.pinned) return null;
  return (
    <span
      className={cn(
        "flex shrink-0 flex-col pb-0.5 text-[10px] leading-tight font-bold text-ink-faint",
        own ? "items-end" : "items-start",
      )}
    >
      {message.pinned && <Pin className="size-3" aria-label={t("message.pinned")} />}
      {read &&
        (group ? (
          <button
            type="button"
            onClick={() => showReceipts(message)}
            className="text-brand hover:underline"
            data-testid="read-mark"
          >
            {t("message.readCount", { count: readers })}
          </button>
        ) : (
          <span className="text-brand" data-testid="read-mark">
            {t("message.read")}
          </span>
        ))}
      {message.editedAt && !message.deleted && <span>{t("message.edited")}</span>}
      {show && (
        <time dateTime={new Date(toMillis(message.createdAt)).toISOString()}>
          {clock(toMillis(message.createdAt))}
        </time>
      )}
    </span>
  );
}

function Reactions({
  message,
  canReact,
  onReact,
}: {
  message: chat.Message;
  canReact: boolean;
  onReact(emoji: string): void;
}) {
  const { t } = useTranslation("chat");
  const { showReactions } = useConversationContext();
  return (
    <div className="mt-1 flex max-w-full flex-wrap gap-1" data-testid="reactions">
      {message.reactions.map((reaction) => (
        <button
          key={reaction.emoji}
          type="button"
          aria-pressed={reaction.reactedByMe}
          aria-label={t("message.reactionCount", { emoji: reaction.emoji, count: reaction.count })}
          onClick={() => (canReact ? onReact(reaction.emoji) : showReactions(message))}
          onContextMenu={(event) => {
            event.preventDefault();
            event.stopPropagation();
            showReactions(message);
          }}
          className={cn(
            "flex h-7 animate-pop-in items-center gap-1 rounded-full border-2 px-2 text-sm font-bold tabular-nums transition-transform active:scale-90",
            reaction.reactedByMe
              ? "border-brand bg-brand-soft text-brand-strong"
              : "border-line bg-surface text-ink-soft",
          )}
        >
          <span className="text-base leading-none">{reaction.emoji}</span>
          {reaction.count}
        </button>
      ))}
    </div>
  );
}

/** Under a message the server did not take: why, and "try again" / "discard". */
function FailedLine({ message, error }: { message: chat.Message; error?: string }) {
  const { t } = useTranslation("chat");
  const queryClient = useQueryClient();
  const outgoing = useOutgoing(message.conversationId).find(
    (item) => item.clientId === message.clientId,
  );
  if (!outgoing) return null;
  return (
    <div
      className="mt-1 flex flex-wrap items-center justify-end gap-x-3 gap-y-1 text-xs font-bold"
      role="alert"
    >
      <span className="text-danger">{error || t("message.failed")}</span>
      <button
        type="button"
        onClick={() => retry(queryClient, outgoing)}
        className="flex items-center gap-1 text-brand"
      >
        <RotateCw className="size-3.5" />
        {t("message.retry")}
      </button>
      <button type="button" onClick={() => discard(outgoing)} className="text-ink-soft">
        {t("message.discard")}
      </button>
    </div>
  );
}
