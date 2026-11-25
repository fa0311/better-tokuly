import type { QueryClient } from "@tanstack/query-core";
import type { chat } from "@tokuly/api";
import type { ClientSource } from "../query";
import { chatKeys } from "./keys";
import { adjustUnread, storeUnread, type Unread, unreadOf } from "./unread";

/** How long after the last realtime adjustment the totals are checked against the server. */
const SETTLE_MS = 1500;

/**
 * Chat's unread totals as one app keeps them: answers from the server and
 * adjustments for realtime events, without one undoing the other.
 *
 * An answer reflects the moment it was asked: one asked before an adjustment
 * may predate the message behind it, and replacing the adjusted totals with
 * it would lose that message. So every request that carries totals takes
 * `version()` first and hands it to `apply`. Each app holds one of these.
 */
export class UnreadTotals {
  private adjustments = 0;
  private settling: ReturnType<typeof setTimeout> | undefined;

  /** Taken before asking the server for anything that carries unread totals. */
  version(): number {
    return this.adjustments;
  }

  /**
   * Stores the totals the server just reported (list, mark-read and count
   * responses carry them). When realtime events adjusted the totals since
   * `askedAt`, the answer may be older than what is held: they are read again instead.
   */
  apply(queryClient: QueryClient, unread: chat.UnreadCounts | undefined, askedAt: number): void {
    if (!unread) return;
    if (askedAt !== this.adjustments) {
      this.refresh(queryClient);
      return;
    }
    storeUnread(queryClient, unread);
  }

  /**
   * Adjusts the totals for something that just happened (a realtime event),
   * at once. An event can trail an answer that already counted its message
   * (the answer was stored first, so the version cannot tell), so once events
   * quiet down the server's totals settle it.
   */
  adjust(queryClient: QueryClient, change: (unread: Unread) => Unread): void {
    if (!adjustUnread(queryClient, change)) return;
    this.adjustments += 1;
    clearTimeout(this.settling);
    this.settling = setTimeout(() => this.refresh(queryClient), SETTLE_MS);
  }

  refresh(queryClient: QueryClient): void {
    void queryClient.invalidateQueries({ queryKey: chatKeys.unread, exact: true });
  }

  /** Stops the pending check (the account left). */
  dispose(): void {
    clearTimeout(this.settling);
  }

  /** The totals: fetched once, then kept current by responses and realtime events. */
  query(queryClient: QueryClient, client: ClientSource<typeof chat.ConversationService>) {
    return {
      queryKey: chatKeys.unread,
      queryFn: async (): Promise<Unread> => {
        const askedAt = this.adjustments;
        const fresh = unreadOf((await client().getUnreadCounts({})).unread);
        const held = queryClient.getQueryData<Unread>(chatKeys.unread);
        if (askedAt === this.adjustments || !held) return fresh;
        // A message arrived while this was asked: keep what counts it and ask once more.
        setTimeout(() => this.refresh(queryClient));
        return held;
      },
      staleTime: Number.POSITIVE_INFINITY,
    };
  }
}
