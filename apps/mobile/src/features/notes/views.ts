import { notes } from "@tokuly/api";
import { type Account, currentAccount, isActive } from "#/features/session/account";

/** A note counts as seen once half of it stayed on screen for a second (FlashList's viewability). */
export const VIEWABILITY = { itemVisiblePercentThreshold: 50, minimumViewTime: 1000 } as const;
/** Views are reported together, this long after the first one of a batch. */
const FLUSH_MS = 2500;
/** `RecordNoteViews` takes at most this many notes. */
const BATCH_LIMIT = 100;

interface Reporter {
  account: Account;
  /** Notes this account already reported here: the server counts a viewer once per note for hours anyway. */
  reported: Set<string>;
  queue: string[];
  timer?: ReturnType<typeof setTimeout>;
}

/** One reporter per session: a view is only ever reported as the account that saw the note. */
const reporters = new Map<string, Reporter>();

function flush(reporter: Reporter): void {
  reporter.timer = undefined;
  // A view seen by an account that has left since is not worth a request as anyone.
  if (!isActive(reporter.account)) {
    reporters.delete(reporter.account.sessionId);
    return;
  }
  const noteIds = reporter.queue.splice(0, BATCH_LIMIT);
  if (noteIds.length > 0)
    void reporter.account.api
      .service(notes.NoteService)
      .recordNoteViews({ noteIds })
      .catch(() => {});
  if (reporter.queue.length > 0) reporter.timer = setTimeout(() => flush(reporter), FLUSH_MS);
}

function report(noteId: string): void {
  const account = currentAccount();
  if (!account) return;
  let reporter = reporters.get(account.sessionId);
  if (!reporter || reporter.account.api !== account.api) {
    reporter = { account, reported: new Set(), queue: [] };
    reporters.set(account.sessionId, reporter);
  }
  if (reporter.reported.has(noteId)) return;
  reporter.reported.add(noteId);
  reporter.queue.push(noteId);
  const pending = reporter;
  pending.timer ??= setTimeout(() => flush(pending), FLUSH_MS);
}

/**
 * The notes a list's `onViewableItemsChanged` found really on screen are
 * reported as viewed, except the viewer's own (the server would not count
 * them).
 */
export function reportViewed(viewed: readonly notes.Note[], myId: string): void {
  for (const note of viewed) if (note.author?.id !== myId) report(note.id);
}
