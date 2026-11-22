/**
 * What was typed but not sent, per person and conversation. Drafts are a
 * client-side concern: they live in this browser's storage and nowhere else.
 * They belong to the account that typed them: the key names it, and signing
 * out removes them.
 */
const PREFIX = "tokuly.chat.draft.";

const keyOf = (userId: string, conversationId: string) => `${PREFIX}${userId}.${conversationId}`;

export function loadDraft(userId: string, conversationId: string): string {
  try {
    return window.localStorage.getItem(keyOf(userId, conversationId)) ?? "";
  } catch {
    return "";
  }
}

export function saveDraft(userId: string, conversationId: string, text: string): void {
  try {
    if (text.trim()) window.localStorage.setItem(keyOf(userId, conversationId), text);
    else window.localStorage.removeItem(keyOf(userId, conversationId));
  } catch {
    // Storage is full or blocked: the draft simply does not survive a reload.
  }
}

/** Removes every draft kept in this browser (on sign-out). */
export function clearDrafts(): void {
  try {
    const storage = window.localStorage;
    const keys = Array.from({ length: storage.length }, (_, index) => storage.key(index)).filter(
      (key): key is string => Boolean(key?.startsWith(PREFIX)),
    );
    for (const key of keys) storage.removeItem(key);
  } catch {
    // Nothing to remove where storage is blocked.
  }
}
