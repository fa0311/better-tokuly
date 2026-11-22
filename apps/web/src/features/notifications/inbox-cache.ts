import type { InfiniteData } from "@tanstack/react-query";
import type { notify } from "@tokuly/api";

/**
 * The inbox as it sits in the query cache, and the pure edits that keep it in
 * step with what the user does and what the realtime connection reports.
 */
export type InboxPages = InfiniteData<notify.ListNotificationsResponse>;

/** Which entries a view of the inbox shows. */
export interface InboxFilter {
  /** Category ID; empty = every category. */
  category: string;
  unreadOnly: boolean;
}

/** Entries fetched per page: a few screens' worth; the next page loads before the end is reached. */
export const PAGE_SIZE = 20;

type Entry = notify.Notification;

function matches(filter: InboxFilter, entry: Entry): boolean {
  return (
    (!filter.category || entry.category === filter.category) && (!filter.unreadOnly || !entry.read)
  );
}

function mapEntries(
  pages: InboxPages,
  edit: (entries: Entry[], pageIndex: number) => Entry[],
): InboxPages {
  return {
    ...pages,
    pages: pages.pages.map((page, index) => ({
      ...page,
      notifications: edit(page.notifications, index),
    })),
  };
}

/** Every entry currently loaded, in display order. */
export function entriesOf(pages: InboxPages | undefined): Entry[] {
  return pages?.pages.flatMap((page) => page.notifications) ?? [];
}

/**
 * A notification arrived. A grouped entry that was replaced by a newer event
 * keeps its ID, so the old copy is dropped wherever it was and the new one
 * goes to the top (when this view shows it at all).
 */
export function withReceived(pages: InboxPages, entry: Entry, filter: InboxFilter): InboxPages {
  const show = matches(filter, entry);
  return mapEntries(pages, (entries, index) => {
    const others = entries.filter((existing) => existing.id !== entry.id);
    return index === 0 && show ? [entry, ...others] : others;
  });
}

/** Entries were read. `ids` = which ones; `"all"` = every entry of `category` (empty = any). */
export function withRead(
  pages: InboxPages,
  ids: ReadonlySet<string> | "all",
  filter: InboxFilter,
  category = "",
): InboxPages {
  const affected = (entry: Entry) =>
    !entry.read && (ids === "all" ? !category || entry.category === category : ids.has(entry.id));
  return mapEntries(pages, (entries) =>
    // A view of unread entries loses them; the others show them as read.
    filter.unreadOnly
      ? entries.filter((entry) => !affected(entry))
      : entries.map((entry) => (affected(entry) ? { ...entry, read: true } : entry)),
  );
}

/** Entries were deleted: specific ones, or (cleared) all / all read ones of a category. */
export function withRemoved(
  pages: InboxPages,
  target: ReadonlySet<string> | { category: string; readOnly: boolean },
): InboxPages {
  const removed = (entry: Entry) =>
    "has" in target
      ? target.has(entry.id)
      : (!target.category || entry.category === target.category) &&
        (!target.readOnly || entry.read);
  return mapEntries(pages, (entries) => entries.filter((entry) => !removed(entry)));
}
