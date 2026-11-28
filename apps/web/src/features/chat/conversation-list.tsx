import { Link } from "@tanstack/react-router";
import { type chat, toMillis } from "@tokuly/api";
import { isGroup, isMuted } from "@tokuly/core/chat";
import { AtSign, BellOff, Check, MoreHorizontal, PhoneCall, Pin, SquarePen } from "lucide-react";
import { useState } from "react";
import { Avatar } from "#/components/ui/avatar";
import { CountBadge } from "#/components/ui/badge";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { EmojiText } from "#/features/emoji";
import { FreeFriendsStrip } from "#/features/plans";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { useMe } from "#/stores/session";
import { useConversationActions } from "./actions";
import type { ListFilter } from "./api";
import { ConversationAvatar } from "./conversation-avatar";
import { type ConversationDialog, ConversationDialogs } from "./conversation-dialogs";
import { useConversationMenu } from "./conversation-menu";
import { useConversationList, useUnread } from "./conversations";
import { useTyping } from "./live";
import { NewChatDialog } from "./new-chat-dialog";
import { useOpenDirect } from "./open-direct";
import { ContextActions, SearchField, SectionTitle } from "./parts";
import { displayName, matchesQuery, titleOf, useFriends } from "./people";
import { previewText } from "./preview";
import { listTime } from "./text";

const EMPTY: Record<ListFilter, { emoji: string; key: string }> = {
  inbox: { emoji: "💬", key: "inbox" },
  requests: { emoji: "📮", key: "requests" },
  archived: { emoji: "🗄️", key: "archived" },
};

/**
 * The list pane: the viewer's conversations (inbox, message requests,
 * archive), a search over them and their friends, and the way to start a
 * new chat or group.
 */
export function ConversationList({
  activeId,
  onOpen,
}: {
  activeId?: string;
  onOpen(conversationId: string): void;
}) {
  const { t } = useTranslation("chat");
  const me = useMe();
  const myId = me?.user?.id ?? "";
  const [filter, setFilter] = useState<ListFilter>("inbox");
  const [query, setQuery] = useState("");
  const [creating, setCreating] = useState(false);
  const [dialog, setDialog] = useState<ConversationDialog>(null);
  const unread = useUnread();
  const list = useConversationList(filter);
  const conversations = list.data?.pages.flatMap((page) => page.conversations) ?? [];
  const searching = query.trim().length > 0;
  const shown = searching
    ? conversations.filter((conversation) => matchesQuery(conversation, myId, query))
    : conversations;

  return (
    <div className="flex h-full flex-col bg-bg">
      <header className="flex h-16 shrink-0 items-center gap-2 px-4">
        <h1 className="min-w-0 flex-1 truncate text-2xl font-black">{t("list.title")}</h1>
        <Link
          to="/chat/calls"
          aria-label={t("calls.history")}
          title={t("calls.history")}
          className="grid size-10 place-items-center rounded-full text-ink-soft hover:bg-surface-2 hover:text-ink"
        >
          <PhoneCall className="size-5" />
        </Link>
        <Button
          size="icon-sm"
          aria-label={t("new.title")}
          title={t("new.title")}
          onClick={() => setCreating(true)}
        >
          <SquarePen className="size-[18px]" />
        </Button>
      </header>

      <div className="flex shrink-0 flex-col gap-3 px-4 pb-3">
        <SearchField value={query} onChange={setQuery} placeholder={t("list.search")} />
        <Tabs<ListFilter>
          value={filter}
          onChange={setFilter}
          items={[
            { value: "inbox", label: t("list.tabs.inbox") },
            { value: "requests", label: t("list.tabs.requests"), badge: unread.pendingRequests },
            { value: "archived", label: t("list.tabs.archived"), badge: unread.archivedUnread },
          ]}
        />
      </div>

      <div
        className="min-h-0 flex-1 overflow-y-auto px-2 pb-32 md:pb-4"
        data-testid="conversation-list"
      >
        {/* Who is free right now: "あそぼ!" starts right where the conversations are. */}
        {filter === "inbox" && !searching && <FreeFriendsStrip bare quiet className="px-2 pb-3" />}
        {list.isPending ? (
          <div className="flex flex-col gap-2 px-2">
            {[0, 1, 2, 3, 4].map((index) => (
              <Skeleton key={index} className="h-[4.25rem]" />
            ))}
          </div>
        ) : shown.length > 0 ? (
          <ul className="flex flex-col gap-0.5">
            {shown.map((conversation) => (
              <li key={conversation.id}>
                <ConversationRow
                  conversation={conversation}
                  myId={myId}
                  active={conversation.id === activeId}
                  request={filter === "requests"}
                  onDialog={setDialog}
                />
              </li>
            ))}
          </ul>
        ) : searching ? null : (
          <EmptyState
            emoji={EMPTY[filter].emoji}
            title={t(`list.empty.${EMPTY[filter].key}.title`)}
            body={t(`list.empty.${EMPTY[filter].key}.body`)}
            action={
              filter === "inbox" ? (
                <Button onClick={() => setCreating(true)}>{t("new.start")}</Button>
              ) : undefined
            }
          />
        )}
        {searching ? (
          <FriendMatches
            query={query}
            shownIds={shown.flatMap((conversation) =>
              isGroup(conversation) ? [] : conversation.members.map((member) => member.id),
            )}
            empty={shown.length === 0}
            onOpen={onOpen}
          />
        ) : (
          <LoadMore query={list} />
        )}
      </div>

      <NewChatDialog open={creating} onOpenChange={setCreating} onOpen={onOpen} />
      <ConversationDialogs dialog={dialog} myId={myId} onClose={() => setDialog(null)} />
    </div>
  );
}

function ConversationRow({
  conversation,
  myId,
  active,
  request,
  onDialog,
}: {
  conversation: chat.Conversation;
  myId: string;
  active: boolean;
  request: boolean;
  onDialog(dialog: ConversationDialog): void;
}) {
  const { t } = useTranslation("chat");
  const actions = useConversationActions();
  const menu = useConversationMenu(conversation, onDialog);
  const typing = useTyping(conversation.id, myId);
  const viewer = conversation.viewer;
  const group = isGroup(conversation);
  const unread = viewer?.unreadCount ?? 0;
  const muted = isMuted(viewer);
  const title = titleOf(conversation, myId);
  const preview = conversation.activeCallId
    ? t("list.inCall")
    : typing.length > 0
      ? t("typing.short")
      : previewText(conversation.lastMessage, myId, group);

  return (
    <div className="group relative">
      <ContextActions actions={menu}>
        <Link
          to="/chat/$conversationId"
          params={{ conversationId: conversation.id }}
          aria-current={active ? "page" : undefined}
          data-testid="conversation-row"
          className={cn(
            "flex items-center gap-3 rounded-2xl px-3 py-2.5 transition-colors hover:bg-surface-2",
            active && "bg-brand-soft hover:bg-brand-soft",
          )}
        >
          <ConversationAvatar conversation={conversation} myId={myId} />
          <span className="min-w-0 flex-1">
            <span className="flex items-center gap-1.5">
              <span className="truncate font-black">{title}</span>
              {group && (
                <span className="shrink-0 text-xs font-bold text-ink-faint">
                  {conversation.memberCount}
                </span>
              )}
              {muted && (
                <BellOff
                  className="size-3.5 shrink-0 text-ink-faint"
                  aria-label={t("list.muted")}
                />
              )}
              {viewer?.pinned && (
                <Pin className="size-3.5 shrink-0 text-ink-faint" aria-label={t("list.pinned")} />
              )}
              <span className="ml-auto shrink-0 pl-2 text-xs font-bold text-ink-faint md:group-hover:invisible">
                {listTime(toMillis(conversation.lastActivityAt))}
              </span>
            </span>
            <span className="mt-0.5 flex items-center gap-1.5">
              <span
                className={cn(
                  "min-w-0 flex-1 truncate text-sm",
                  conversation.activeCallId
                    ? "font-bold text-success"
                    : typing.length > 0
                      ? "font-bold text-brand"
                      : unread > 0
                        ? "font-bold text-ink"
                        : "text-ink-soft",
                )}
              >
                {preview || " "}
              </span>
              {(viewer?.unreadMentionCount ?? 0) > 0 && (
                <span
                  className="grid size-5 shrink-0 place-items-center rounded-full bg-brand text-on-brand"
                  role="img"
                  aria-label={t("list.mentioned")}
                >
                  <AtSign className="size-3" />
                </span>
              )}
              <CountBadge
                count={unread}
                className={cn("shrink-0 border-0", muted && "bg-ink-faint")}
              />
            </span>
          </span>
        </Link>
      </ContextActions>
      {menu.length > 0 && (
        <div className="absolute top-1.5 right-2 hidden md:group-focus-within:block md:group-hover:block">
          <Menu
            actions={menu}
            trigger={
              <button
                type="button"
                aria-label={t("menu.more", { name: title })}
                className="grid size-7 place-items-center rounded-full bg-surface text-ink-soft shadow-pop-sm hover:text-ink"
              >
                <MoreHorizontal className="size-4" />
              </button>
            }
          />
        </div>
      )}
      {request && (
        <div className="flex gap-2 px-3 pb-2 pl-[4.25rem]">
          <Button
            size="sm"
            variant="lime"
            icon={<Check className="size-4" />}
            onClick={() => void actions.respond(conversation, true)}
          >
            {t("request.accept")}
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => void actions.respond(conversation, false)}
          >
            {t("request.decline")}
          </Button>
        </div>
      )}
    </div>
  );
}

/** While searching: friends who match and are not in the results already, one tap from a chat. */
function FriendMatches({
  query,
  shownIds,
  empty,
  onOpen,
}: {
  query: string;
  shownIds: string[];
  empty: boolean;
  onOpen(conversationId: string): void;
}) {
  const { t } = useTranslation("chat");
  const friends = useFriends(query.trim().replace(/^@/, ""));
  const openDirect = useOpenDirect();
  const [starting, setStarting] = useState<string | null>(null);
  const matches = (friends.data?.friends ?? []).filter(
    (friend) => friend.user && !shownIds.includes(friend.user.id),
  );

  const start = async (userId: string) => {
    setStarting(userId);
    const conversation = await openDirect(userId);
    setStarting(null);
    if (conversation) onOpen(conversation.id);
  };

  if (matches.length === 0)
    return empty && !friends.isPending ? (
      <EmptyState
        emoji="🔍"
        title={t("list.empty.search.title")}
        body={t("list.empty.search.body")}
      />
    ) : null;
  return (
    <>
      <SectionTitle>{t("list.friends")}</SectionTitle>
      <ul>
        {matches.map(
          (friend) =>
            friend.user && (
              <li key={friend.user.id}>
                <button
                  type="button"
                  disabled={starting !== null}
                  onClick={() => void start(friend.user!.id)}
                  className="flex w-full items-center gap-3 rounded-2xl px-3 py-2.5 text-left transition-colors hover:bg-surface-2"
                >
                  <Avatar user={friend.user} outlined />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-black">
                      <EmojiText text={displayName(friend.user)} />
                    </span>
                    <span className="block truncate text-sm text-ink-soft">
                      @{friend.user.handle}
                    </span>
                  </span>
                </button>
              </li>
            ),
        )}
      </ul>
    </>
  );
}
