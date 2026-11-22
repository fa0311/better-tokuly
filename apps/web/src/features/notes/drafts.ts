import type { common } from "@tokuly/api";
import type { ComposeContext } from "./compose-store";

/** What is kept of a note that was started and not posted. Uploads are not kept: they would be gone by the time the draft is opened again. */
export interface Draft {
  text: string;
  visibility?: common.Visibility;
  contentWarning?: boolean;
  /** When the draft was last changed (epoch ms): the oldest ones make room for new ones. */
  savedAt: number;
}

/** Drafts kept per account: the new-note draft plus one per note being answered or quoted. */
const MAX_DRAFTS = 20;
const storageKey = (userId: string) => `tokuly.notes.drafts.${userId}`;

/** Which draft a composer context writes to: one for new notes, one per note answered or quoted. */
export function draftKey(context: ComposeContext): string {
  if (context.replyTo) return `reply:${context.replyTo.id}`;
  if (context.quote) return `quote:${context.quote.id}`;
  return "new";
}

function readAll(userId: string): Record<string, Draft> {
  try {
    const stored: unknown = JSON.parse(window.localStorage.getItem(storageKey(userId)) ?? "{}");
    return stored && typeof stored === "object" ? (stored as Record<string, Draft>) : {};
  } catch {
    return {};
  }
}

function writeAll(userId: string, drafts: Record<string, Draft>): void {
  try {
    if (Object.keys(drafts).length === 0) window.localStorage.removeItem(storageKey(userId));
    else window.localStorage.setItem(storageKey(userId), JSON.stringify(drafts));
  } catch {
    // Private mode or a full store: drafts are a convenience, not a promise.
  }
}

export function loadDraft(userId: string, key: string): Draft | undefined {
  const draft = readAll(userId)[key];
  return draft && typeof draft.text === "string" ? draft : undefined;
}

/** Keeps a draft, or forgets it when there is nothing worth keeping. */
export function saveDraft(userId: string, key: string, draft: Omit<Draft, "savedAt">): void {
  const drafts = readAll(userId);
  if (draft.text.trim()) drafts[key] = { ...draft, savedAt: Date.now() };
  else delete drafts[key];
  const newestFirst = Object.entries(drafts).sort(([, a], [, b]) => b.savedAt - a.savedAt);
  writeAll(userId, Object.fromEntries(newestFirst.slice(0, MAX_DRAFTS)));
}

export function discardDraft(userId: string, key: string): void {
  const drafts = readAll(userId);
  delete drafts[key];
  writeAll(userId, drafts);
}
