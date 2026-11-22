import { useInfiniteQuery } from "@tanstack/react-query";
import { chat, toMillis } from "@tokuly/api";
import { PinOff, X } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Dialog } from "#/components/ui/dialog";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { errorMessage } from "#/lib/errors";
import { chatKeys, messageClient } from "./api";
import { useConversationContext } from "./conversation-context";
import { Lightbox, type LightboxItem } from "./lightbox";
import { FileRow, VisualTile, VoicePlayer } from "./media-parts";
import { useMessageActions } from "./messages";
import { SearchField, useMediaQuery } from "./parts";
import { displayName } from "./people";
import { contentText } from "./preview";
import { dateTime } from "./text";

/**
 * A panel that goes with the open conversation (details, search, pins,
 * media): a column beside it where the screen is wide enough for three, a
 * dialog (a sheet on phones) everywhere else.
 */
export function SidePanel({ title, onClose, children }: { title: ReactNode; onClose(): void; children: ReactNode }) {
  const { t } = useTranslation("chat");
  const docked = useMediaQuery("(min-width: 1180px)");
  if (!docked) {
    return (
      <Dialog open onOpenChange={(open) => !open && onClose()} title={title} className="h-[85dvh] sm:h-[min(44rem,calc(100dvh-4rem))]">
        {children}
      </Dialog>
    );
  }
  return (
    <aside className="flex h-full w-[22rem] shrink-0 flex-col border-l-2 border-line bg-surface" data-testid="side-panel">
      <header className="flex h-16 shrink-0 items-center gap-2 border-b-2 border-line px-4">
        <h2 className="min-w-0 flex-1 truncate text-lg font-black">{title}</h2>
        <button type="button" aria-label={t("actions.close")} onClick={onClose} className="grid size-9 place-items-center rounded-full hover:bg-surface-2">
          <X className="size-5" />
        </button>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-4">{children}</div>
    </aside>
  );
}

function RowsSkeleton() {
  return (
    <div className="flex flex-col gap-2">
      {[0, 1, 2].map((index) => (
        <Skeleton key={index} className="h-14" />
      ))}
    </div>
  );
}

/** One message as a row of a result list: who, when, and the start of what they said. */
function MessageResult({ message, onOpen, trailing }: { message: chat.Message; onOpen(): void; trailing?: ReactNode }) {
  const { me } = useConversationContext();
  return (
    <div className="flex items-center gap-1 rounded-2xl hover:bg-surface-2" data-testid="message-result">
      <button type="button" onClick={onOpen} className="flex min-w-0 flex-1 items-start gap-3 px-2 py-2 text-left">
        <Avatar user={message.sender} size="sm" />
        <span className="min-w-0 flex-1">
          <span className="flex items-baseline gap-2">
            <span className="truncate text-sm font-black">{displayName(message.sender)}</span>
            <span className="shrink-0 text-[11px] font-bold text-ink-faint">{dateTime(toMillis(message.createdAt))}</span>
          </span>
          <span className="line-clamp-2 text-sm text-ink-soft [overflow-wrap:anywhere]">{contentText(message, me.id)}</span>
        </span>
      </button>
      {trailing}
    </div>
  );
}

/** Search in this conversation: messages whose text contains what was typed, newest first. */
export function SearchPanel({ onOpen }: { onOpen(messageId: string): void }) {
  const { t } = useTranslation("chat");
  const { conversation } = useConversationContext();
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(query.trim()), 300);
    return () => clearTimeout(timer);
  }, [query]);
  const results = useInfiniteQuery({
    queryKey: chatKeys.search(conversation.id, debounced),
    queryFn: ({ pageParam }) => messageClient().searchMessages({ conversationId: conversation.id, query: debounced, page: { cursor: pageParam, limit: 30 } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
    enabled: debounced.length > 0,
  });
  const messages = results.data?.pages.flatMap((page) => page.messages) ?? [];

  return (
    <div className="flex flex-col gap-3">
      <SearchField value={query} onChange={setQuery} placeholder={t("search.placeholder")} autoFocus />
      {!debounced ? (
        <EmptyState emoji="🔎" title={t("search.start")} />
      ) : results.isPending ? (
        <RowsSkeleton />
      ) : results.isError ? (
        <p className="px-2 text-sm text-danger">{errorMessage(results.error)}</p>
      ) : messages.length === 0 ? (
        <EmptyState emoji="🫥" title={t("search.empty")} />
      ) : (
        <div className="-mx-2 flex flex-col">
          {messages.map((message) => (
            <MessageResult key={message.id} message={message} onOpen={() => onOpen(message.id)} />
          ))}
          <LoadMore query={results} />
        </div>
      )}
    </div>
  );
}

/** The pinned messages of the conversation, most recently pinned first. */
export function PinnedPanel({ onOpen }: { onOpen(messageId: string): void }) {
  const { t } = useTranslation("chat");
  const { conversation, accepted } = useConversationContext();
  const actions = useMessageActions();
  const pinned = useInfiniteQuery({
    queryKey: chatKeys.pinned(conversation.id),
    queryFn: ({ pageParam }) => messageClient().listPinnedMessages({ conversationId: conversation.id, page: { cursor: pageParam, limit: 30 } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const messages = pinned.data?.pages.flatMap((page) => page.messages) ?? [];

  if (pinned.isPending) return <RowsSkeleton />;
  if (messages.length === 0) return <EmptyState emoji="📌" title={t("pins.empty")} body={t("pins.emptyBody")} />;
  return (
    <div className="-mx-2 flex flex-col">
      {messages.map((message) => (
        <MessageResult
          key={message.id}
          message={message}
          onOpen={() => onOpen(message.id)}
          trailing={
            accepted && (
              <button
                type="button"
                aria-label={t("message.unpin")}
                title={t("message.unpin")}
                onClick={() => actions.pin(message, false).catch((error) => toast.error(errorMessage(error)))}
                className="mr-1 grid size-9 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-3 hover:text-ink"
              >
                <PinOff className="size-4" />
              </button>
            )
          }
        />
      ))}
      <LoadMore query={pinned} />
    </div>
  );
}

type MediaTab = "visual" | "audio" | "files";

const MEDIA_FILTER: Record<MediaTab, chat.SharedMediaFilter> = {
  visual: chat.SharedMediaFilter.VISUAL,
  audio: chat.SharedMediaFilter.AUDIO,
  files: chat.SharedMediaFilter.FILES,
};

/** Everything shared in the conversation: photos and videos as a grid, voice messages and files as lists. */
export function MediaPanel({ onOpen }: { onOpen(messageId: string): void }) {
  const { t } = useTranslation("chat");
  const { conversation } = useConversationContext();
  const [tab, setTab] = useState<MediaTab>("visual");
  const [viewing, setViewing] = useState<number | null>(null);
  const shared = useInfiniteQuery({
    queryKey: chatKeys.media(conversation.id, MEDIA_FILTER[tab]),
    queryFn: ({ pageParam }) => messageClient().listSharedMedia({ conversationId: conversation.id, filter: MEDIA_FILTER[tab], page: { cursor: pageParam, limit: 30 } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const items = (shared.data?.pages.flatMap((page) => page.items) ?? []).filter((item) => item.attachment);
  const names = new Map(conversation.members.map((member) => [member.id, displayName(member)]));
  const viewer: LightboxItem[] =
    tab === "visual"
      ? items.map((item) => ({
          key: `${item.messageId}:${item.attachment!.assetId}`,
          attachment: item.attachment!,
          caption: [names.get(item.senderId), dateTime(toMillis(item.createdAt))].filter(Boolean).join(" · "),
          messageId: item.messageId,
        }))
      : [];

  return (
    <div className="flex flex-col gap-3">
      <Tabs<MediaTab>
        value={tab}
        onChange={setTab}
        items={[
          { value: "visual", label: t("media.tabs.visual") },
          { value: "audio", label: t("media.tabs.audio") },
          { value: "files", label: t("media.tabs.files") },
        ]}
      />
      {shared.isPending ? (
        <RowsSkeleton />
      ) : items.length === 0 ? (
        <EmptyState emoji={tab === "visual" ? "🖼️" : tab === "audio" ? "🎤" : "📁"} title={t(`media.empty.${tab}`)} />
      ) : tab === "visual" ? (
        <div className="grid grid-cols-3 gap-1">
          {items.map((item, index) => (
            <VisualTile key={`${item.messageId}:${item.attachment!.assetId}`} attachment={item.attachment!} pixels={130} label={t("media.openPhoto")} onOpen={() => setViewing(index)} className="aspect-square rounded-xl" />
          ))}
        </div>
      ) : (
        <ul className="flex flex-col gap-1">
          {items.map((item) => (
            <li key={`${item.messageId}:${item.attachment!.assetId}`} className="rounded-2xl">
              {tab === "audio" ? (
                <div className="flex items-center gap-3 rounded-2xl px-2 py-2">
                  <VoicePlayer attachment={item.attachment!} tone="other" />
                  <span className="min-w-0 flex-1 text-right text-[11px] font-bold text-ink-faint">
                    <span className="block truncate">{names.get(item.senderId)}</span>
                    {dateTime(toMillis(item.createdAt))}
                  </span>
                </div>
              ) : (
                <FileRow attachment={item.attachment!} tone="plain" />
              )}
            </li>
          ))}
        </ul>
      )}
      <LoadMore query={shared} />
      <Lightbox items={viewer} index={viewing} onIndexChange={setViewing} onClose={() => setViewing(null)} onShowInChat={onOpen} />
    </div>
  );
}
