import { useQueryClient } from "@tanstack/react-query";
import type { chat, common } from "@tokuly/api";
import { buildRows, type Row } from "@tokuly/core/chat";
import { useEffect, useMemo, useState } from "react";
import { pager, useTimeline } from "./data";
import { previewMessage, useOutgoing } from "./outbox";

/**
 * The rows of an open conversation and the paging around them: the held
 * timeline plus the messages still on their way, "unread from here" placed
 * where the reader had read to when it opened, older pages at the top and
 * newer ones (on the way back from the history) at the bottom. The pager
 * never asks for the same page twice at once, so scrolling may call freely.
 */
export function useMessageWindow(
  conversationId: string,
  conversation: chat.Conversation | undefined,
  me: common.UserSummary,
) {
  const queryClient = useQueryClient();
  const timeline = useTimeline(conversationId);
  const outgoing = useOutgoing(conversationId);
  const loaded = timeline.data;

  // Where the reader had read to when the conversation opened; settled once, before the first rows.
  const [unreadAfter, setUnreadAfter] = useState<bigint | null | undefined>(undefined);
  const viewer = conversation?.viewer;
  if (unreadAfter === undefined && loaded && viewer)
    setUnreadAfter(viewer.unreadCount > 0 ? viewer.readSeq : null);

  const rows: Row[] = useMemo(
    () =>
      buildRows(loaded?.messages ?? [], {
        // Messages on their way are drawn at the live end only.
        pending: loaded?.hasNewer ? [] : outgoing.map((item) => previewMessage(item, me)),
        unreadAfterSeq: unreadAfter ?? undefined,
        myId: me.id,
      }),
    [loaded, outgoing, unreadAfter, me],
  );

  // A message that arrived while the first page was loading had nowhere to go: fetch it now.
  const newestSeq = loaded?.messages.at(-1)?.seq ?? 0n;
  const lastSeq = conversation?.lastSeq ?? 0n;
  const missingUpTo =
    loaded !== undefined && !loaded.hasNewer && newestSeq < lastSeq ? lastSeq : null;
  useEffect(() => {
    if (missingUpTo !== null) void pager.catchUp(queryClient, conversationId).catch(() => {});
  }, [missingUpTo, conversationId, queryClient]);

  return {
    rows,
    loading: timeline.isPending,
    failed: timeline.isError,
    hasOlder: loaded?.hasOlder ?? false,
    hasNewer: loaded?.hasNewer ?? false,
    newest: loaded?.messages.at(-1),
    loadOlder: () => {
      if (loaded?.hasOlder) void pager.loadOlder(queryClient, conversationId).catch(() => {});
    },
    loadNewer: () => {
      if (loaded?.hasNewer) void pager.loadNewer(queryClient, conversationId).catch(() => {});
    },
  };
}
