import type { QueryClient } from "@tanstack/query-core";
import type { chat } from "@tokuly/api";
import { pushAfter } from "../query";
import { chatKeys } from "./keys";
import { readTimeline } from "./message-cache";
import { type Timeline, timelineOf, withNewer, withOlder } from "./timeline";

/** Where a page of messages starts (`ListMessagesRequest.position`). */
export type Position = chat.ListMessagesRequest["position"];

/**
 * Fetches one page of a conversation's messages. The app's own: it also
 * keeps what the answer says beside the messages (the read cursors).
 * `signal` ends the request when the read is given up.
 */
export type FetchPage = (
  conversationId: string,
  position: Position,
  signal?: AbortSignal,
) => Promise<chat.ListMessagesResponse>;

/**
 * How a conversation's messages are paged into its timeline
 * (`chatKeys.timeline`): the first window, older and newer pages as the
 * reader scrolls, catching up after missed events, and jumps. The same page
 * is never asked for twice at once.
 *
 * The first window is the timeline query's own read (`queryFn`, with
 * `persister: journal`); every later page is stored with `pushAfter`, so
 * that what was pushed while it was on its way (an edit, an unsend, a
 * reaction) is applied again after the page's older copies.
 */
export class MessagePager {
  private readonly loading = new Set<string>();

  constructor(private readonly fetchPage: FetchPage) {}

  /**
   * The first window: around `around` when given (a search hit, a
   * notification), else around the first unread message, else the newest.
   * A message to open on that is gone (deleted, from before the viewer
   * joined) opens the conversation normally.
   */
  async first(conversationId: string, around?: string, signal?: AbortSignal): Promise<Timeline> {
    const unread: Position = { case: "aroundUnread", value: true };
    if (!around) return timelineOf(await this.fetchPage(conversationId, unread, signal));
    try {
      return timelineOf(
        await this.fetchPage(conversationId, { case: "aroundMessageId", value: around }, signal),
      );
    } catch (error) {
      if (signal?.aborted) throw error;
      return timelineOf(await this.fetchPage(conversationId, unread, signal));
    }
  }

  /** Fetches a page and stores what `change` makes of it, then what was pushed meanwhile. */
  private store(
    queryClient: QueryClient,
    conversationId: string,
    position: Position,
    change: (timeline: Timeline, page: chat.ListMessagesResponse) => Timeline,
  ): Promise<void> {
    return pushAfter<Timeline>(queryClient, chatKeys.timeline(conversationId), async () => {
      const page = await this.fetchPage(conversationId, position);
      return (timeline) => change(timeline, page);
    });
  }

  private async once(key: string, work: () => Promise<void>): Promise<void> {
    if (this.loading.has(key)) return;
    this.loading.add(key);
    try {
      await work();
    } finally {
      this.loading.delete(key);
    }
  }

  /** Loads the page before the oldest message held. */
  loadOlder(queryClient: QueryClient, conversationId: string): Promise<void> {
    return this.once(`older:${conversationId}`, async () => {
      const timeline = readTimeline(queryClient, conversationId);
      if (!timeline?.hasOlder) return;
      await this.store(
        queryClient,
        conversationId,
        { case: "before", value: timeline.olderCursor },
        withOlder,
      );
    });
  }

  /** Loads the page after the newest message held (on the way back from the history, or to catch up). */
  loadNewer(queryClient: QueryClient, conversationId: string): Promise<void> {
    return this.once(`newer:${conversationId}`, async () => {
      const timeline = readTimeline(queryClient, conversationId);
      if (!timeline) return;
      await this.store(
        queryClient,
        conversationId,
        { case: "after", value: timeline.newerCursor },
        withNewer,
      );
    });
  }

  /**
   * Brings a timeline at the live end up to date after events may have been
   * missed. When too much happened meanwhile, it starts over at the newest
   * messages instead of paging through everything.
   */
  async catchUp(queryClient: QueryClient, conversationId: string): Promise<void> {
    const timeline = readTimeline(queryClient, conversationId);
    if (!timeline || timeline.hasNewer) return;
    await this.loadNewer(queryClient, conversationId);
    if (readTimeline(queryClient, conversationId)?.hasNewer)
      await this.showNewest(queryClient, conversationId);
  }

  /** Replaces the timeline with the newest messages ("jump to latest"). */
  showNewest(queryClient: QueryClient, conversationId: string): Promise<void> {
    return this.once(`newest:${conversationId}`, async () => {
      await this.store(queryClient, conversationId, { case: undefined }, (_, page) =>
        timelineOf(page),
      );
    });
  }

  /**
   * Makes sure a message is in the timeline (a reply's original, a search
   * hit, a pin): when it is not held, the timeline is replaced by the window
   * around it. Rejects with MESSAGE_NOT_FOUND when the viewer cannot see it.
   */
  async showMessage(
    queryClient: QueryClient,
    conversationId: string,
    messageId: string,
  ): Promise<void> {
    if (readTimeline(queryClient, conversationId)?.messages.some((m) => m.id === messageId)) return;
    await this.store(
      queryClient,
      conversationId,
      { case: "aroundMessageId", value: messageId },
      (_, page) => timelineOf(page),
    );
  }
}
