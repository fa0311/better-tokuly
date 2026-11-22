import { useQueryClient } from "@tanstack/react-query";
import type { chat, common } from "@tokuly/api";
import { Check } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { useMe } from "#/stores/session";
import { conversationClient } from "./api";
import { ConversationAvatar } from "./conversation-avatar";
import { putConversation, useConversationList } from "./conversations";
import { canWrite, isGroup, peerOf } from "./model";
import { SearchField, SectionTitle } from "./parts";
import { displayName, matchesQuery, titleOf, useFriends } from "./people";

/** Somewhere a message can be sent: a conversation, or a friend there is no conversation with yet. */
export interface Target {
  key: string;
  conversation?: chat.Conversation;
  friend?: common.UserSummary;
}

/**
 * Picks where to send something: the viewer's conversations, then the
 * friends they have not talked to yet, with one search over both.
 */
export function TargetPicker({ selected, onChange, limit }: { selected: Target[]; onChange(selected: Target[]): void; limit: number }) {
  const { t } = useTranslation("chat");
  const myId = useMe()?.user?.id ?? "";
  const [query, setQuery] = useState("");
  const list = useConversationList("inbox");
  const friends = useFriends(query.trim().replace(/^@/, ""));
  const conversations = (list.data?.pages.flatMap((page) => page.conversations) ?? []).filter((conversation) => canWrite(conversation) && matchesQuery(conversation, myId, query));
  const talkedTo = new Set(conversations.flatMap((conversation) => (isGroup(conversation) ? [] : [peerOf(conversation, myId)?.id])));
  const others = (friends.data?.friends ?? []).flatMap((friend) => (friend.user && !talkedTo.has(friend.user.id) ? [friend.user] : []));
  const targets: Target[] = [...conversations.map((conversation) => ({ key: conversation.id, conversation })), ...others.map((friend) => ({ key: `user:${friend.id}`, friend }))];
  const isSelected = (target: Target) => selected.some((chosen) => chosen.key === target.key);
  const full = selected.length >= limit;
  const toggle = (target: Target) => {
    if (isSelected(target)) onChange(selected.filter((chosen) => chosen.key !== target.key));
    else if (!full) onChange([...selected, target]);
  };

  const row = (target: Target) => {
    const chosen = isSelected(target);
    const name = target.conversation ? titleOf(target.conversation, myId) : displayName(target.friend);
    return (
      <li key={target.key}>
        <button
          type="button"
          aria-pressed={chosen}
          disabled={!chosen && full}
          onClick={() => toggle(target)}
          className="flex w-full items-center gap-3 rounded-2xl px-3 py-2 text-left transition-colors hover:bg-surface-2 disabled:opacity-50"
        >
          {target.conversation ? <ConversationAvatar conversation={target.conversation} myId={myId} /> : <Avatar user={target.friend} outlined />}
          <span className="min-w-0 flex-1 truncate font-bold">{name}</span>
          <span className={cn("grid size-6 shrink-0 place-items-center rounded-full border-2", chosen ? "border-ink bg-lime text-[#17122b]" : "border-line")}>{chosen && <Check className="size-4" strokeWidth={3} />}</span>
        </button>
      </li>
    );
  };

  return (
    <div className="flex flex-col gap-3">
      <SearchField value={query} onChange={setQuery} placeholder={t("share.search")} />
      {list.isPending ? (
        <div className="flex flex-col gap-2">
          {[0, 1, 2].map((index) => (
            <Skeleton key={index} className="h-14" />
          ))}
        </div>
      ) : targets.length === 0 ? (
        <EmptyState emoji="🫥" title={t("share.empty")} />
      ) : (
        <div className="-mx-2">
          {conversations.length > 0 && <ul>{targets.filter((target) => target.conversation).map(row)}</ul>}
          {others.length > 0 && (
            <>
              <SectionTitle>{t("share.friends")}</SectionTitle>
              <ul>{targets.filter((target) => target.friend).map(row)}</ul>
            </>
          )}
          {!query && <LoadMore query={list} />}
        </div>
      )}
    </div>
  );
}

/** The conversation IDs behind the chosen targets, opening the direct conversation with friends there is none with yet. */
export function useResolveTargets(): (targets: Target[]) => Promise<string[]> {
  const queryClient = useQueryClient();
  return (targets) =>
    Promise.all(
      targets.map(async (target) => {
        if (target.conversation) return target.conversation.id;
        const { conversation } = await conversationClient().openDirect({ userId: target.friend!.id });
        if (conversation) putConversation(queryClient, conversation);
        return conversation!.id;
      }),
    );
}
