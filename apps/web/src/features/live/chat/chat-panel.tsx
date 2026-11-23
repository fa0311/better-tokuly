import { useMutation } from "@tanstack/react-query";
import { live } from "@tokuly/api";
import { BarChart3, Pin, Settings2, Users, X } from "lucide-react";
import { type ReactNode, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { EmptyState } from "#/components/ui/page";
import { PaneSpinner } from "#/components/ui/spinner";
import { EmojiText } from "#/features/emoji";
import { StampSheet } from "#/features/stamps";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { chatClient, useBlockedIds, useGiftTiers } from "../api";
import { GiftDialog } from "../gifts/gift-dialog";
import { activeHighlights } from "../model";
import { useNow } from "../parts";
import { CreatePollDialog, PollCard, pollVisible } from "../polls";
import { useChatMessages } from "../room";
import { authorName, GiftBubble, mentionOf } from "./chat-line";
import { ChatList } from "./chat-list";
import { ChatSettingsDialog } from "./chat-settings";
import { ChatComposer, type ComposerHandle } from "./composer";
import { MessageActions } from "./moderation";
import { ViewersDialog } from "./viewers";

const { StreamStatus } = live;

/** The message a streamer or moderator pinned to the top of the chat. */
function PinnedMessage({ stream }: { stream: live.Stream }) {
  const { t } = useTranslation("live");
  // A pinned line of someone the viewer blocked stays hidden (also when the page was loaded with it).
  const blocked = useBlockedIds(Boolean(stream.viewer)).data;
  const pinnedBy = stream.pinnedMessage?.author?.user?.id ?? "";
  const pinned = pinnedBy && blocked?.has(pinnedBy) ? undefined : stream.pinnedMessage;
  const unpin = useMutation({
    mutationFn: () => chatClient().unpinChatMessage({ streamId: stream.summary?.id ?? "" }),
    onError: (error) => toast.error(errorMessage(error, { namespace: "live" })),
  });
  if (!pinned) return null;
  return (
    <div
      className="mx-3 mt-2 flex items-start gap-2 rounded-2xl bg-brand-soft px-3 py-2"
      data-testid="pinned-message"
    >
      <Pin
        className="mt-0.5 size-4 shrink-0 text-brand-strong"
        aria-label={t("chat.pinnedLabel")}
      />
      <p className="min-w-0 flex-1 text-sm leading-snug break-words">
        <span className="mr-1.5 font-black text-brand-strong">
          <EmojiText text={authorName(pinned)} />
        </span>
        <EmojiText text={pinned.text} />
      </p>
      {stream.viewer?.isModerator && (
        <button
          type="button"
          aria-label={t("chat.unpin")}
          disabled={unpin.isPending}
          onClick={() => unpin.mutate()}
          className="grid size-7 shrink-0 place-items-center rounded-full hover:bg-surface-2"
        >
          <X className="size-4" />
        </button>
      )}
    </div>
  );
}

/** The gifts that are still highlighted, as chips above the chat; a tap shows the whole gift. */
function Highlights({ stream, tiers }: { stream: live.Stream; tiers: live.GiftTier[] }) {
  const { t } = useTranslation("live");
  const now = useNow(5000);
  const [open, setOpen] = useState<string | null>(null);
  const gifts = activeHighlights(stream.highlightedGifts, now);
  if (gifts.length === 0) return null;
  const shown = gifts.find((gift) => gift.id === open);
  return (
    <div className="mt-2" data-testid="gift-highlights">
      <ul
        className="scrollbar-none flex gap-1.5 overflow-x-auto px-3"
        aria-label={t("gift.highlights")}
      >
        {gifts.map((gift) => (
          <li key={gift.id} className="shrink-0">
            <button
              type="button"
              aria-expanded={open === gift.id}
              aria-label={t("gift.highlightBy", { name: authorName(gift) })}
              onClick={() => setOpen(open === gift.id ? null : gift.id)}
            >
              <GiftBubble message={gift} tiers={tiers} compact />
            </button>
          </li>
        ))}
      </ul>
      {shown && <GiftBubble message={shown} tiers={tiers} className="mx-3 mt-2" />}
    </div>
  );
}

function HeaderButton({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick(): void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className="grid size-9 place-items-center rounded-full text-ink-soft hover:bg-surface-2 hover:text-ink"
    >
      {children}
    </button>
  );
}

/**
 * The live chat of a stream: the pinned message, highlighted gifts and the
 * open poll on top, the scrolling lines, and the composer. The streamer and
 * moderators get the viewer list, the chat rules and polls from its header.
 */
export function LiveChat({
  stream,
  className,
  header = true,
}: {
  stream: live.Stream;
  className?: string;
  /** The title row; off where the page around it has one already. */
  header?: boolean;
}) {
  const { t } = useTranslation("live");
  const me = useMe();
  const streamId = stream.summary?.id ?? "";
  const status = stream.summary?.status;
  const chat = useChatMessages(streamId);
  const tiers = useGiftTiers().data?.tiers ?? [];
  const now = useNow(1000);
  const composer = useRef<ComposerHandle>(null);
  const [selected, setSelected] = useState<live.ChatMessage | null>(null);
  const [gifting, setGifting] = useState(false);
  const [settings, setSettings] = useState(false);
  const [viewers, setViewers] = useState(false);
  const [polling, setPolling] = useState(false);
  const [stampId, setStampId] = useState<string>();

  const viewer = stream.viewer;
  const writable = status === StreamStatus.UPCOMING || status === StreamStatus.LIVE;
  const canGift = status === StreamStatus.LIVE && stream.giftsEnabled && !viewer?.isOwner;
  const poll = pollVisible(stream.activePoll, now) ? stream.activePoll : undefined;
  const mention = (message: live.ChatMessage) => composer.current?.insert(mentionOf(message));

  return (
    <section
      className={cn("flex min-h-0 flex-col bg-surface", className)}
      aria-label={t("chat.title")}
      data-testid="live-chat"
    >
      {header && (
        <header className="flex h-12 shrink-0 items-center gap-1 border-b-2 border-line pr-1.5 pl-4">
          <h2 className="min-w-0 flex-1 truncate font-black">{t("chat.title")}</h2>
          {stream.chat && stream.chat.slowModeSeconds > 0 && (
            <span className="mr-1 rounded-full bg-surface-2 px-2 py-0.5 text-xs font-bold text-ink-soft">
              {t("chat.slowBadge", { count: stream.chat.slowModeSeconds })}
            </span>
          )}
          {viewer?.isModerator && writable && (
            <>
              <HeaderButton label={t("poll.create")} onClick={() => setPolling(true)}>
                <BarChart3 className="size-5" />
              </HeaderButton>
              <HeaderButton label={t("viewersList.open")} onClick={() => setViewers(true)}>
                <Users className="size-5" />
              </HeaderButton>
            </>
          )}
          {viewer?.isOwner && writable && (
            <HeaderButton label={t("chatSettings.open")} onClick={() => setSettings(true)}>
              <Settings2 className="size-5" />
            </HeaderButton>
          )}
        </header>
      )}
      <PinnedMessage stream={stream} />
      <Highlights stream={stream} tiers={tiers} />
      {poll && (
        <PollCard
          poll={poll}
          canVote={Boolean(viewer) && writable}
          canManage={Boolean(viewer?.isModerator)}
          className="mx-3 mt-2"
        />
      )}
      {chat.isPending ? (
        <div className="min-h-0 flex-1">
          <PaneSpinner />
        </div>
      ) : (
        <ChatList
          className="flex-1"
          messages={chat.data ?? []}
          tiers={tiers}
          selfHandle={me?.user?.handle.toLowerCase()}
          onMore={setSelected}
          onMention={writable ? mention : undefined}
          onOpenStamp={setStampId}
          empty={
            <EmptyState
              emoji="💬"
              title={t(writable ? "chat.emptyTitle" : "chat.emptyEndedTitle")}
              body={writable ? t("chat.emptyBody") : undefined}
            />
          }
        />
      )}
      {writable && (
        <ChatComposer
          ref={composer}
          stream={stream}
          onGift={canGift ? () => setGifting(true) : undefined}
        />
      )}

      <MessageActions
        message={selected}
        stream={stream}
        selfId={me?.user?.id}
        onClose={() => setSelected(null)}
        onMention={mention}
      />
      {canGift && <GiftDialog stream={stream} open={gifting} onOpenChange={setGifting} />}
      {viewer?.isOwner && (
        <ChatSettingsDialog stream={stream} open={settings} onOpenChange={setSettings} />
      )}
      {viewer?.isModerator && (
        <>
          <ViewersDialog stream={stream} open={viewers} onOpenChange={setViewers} />
          <CreatePollDialog streamId={streamId} open={polling} onOpenChange={setPolling} />
        </>
      )}
      <StampSheet stampId={stampId} onClose={() => setStampId(undefined)} />
    </section>
  );
}
