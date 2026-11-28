/**
 * What was typed but not sent, per person and conversation. Drafts are a
 * client-side concern: they live in this browser's storage and nowhere else.
 * They belong to the account that typed them: the key names it, and signing
 * out removes them.
 */
const DRAFTS_PREFIX = "tokuly.chat.draft.";

const keyOf = (userId: string, conversationId: string) =>
  `${DRAFTS_PREFIX}${userId}.${conversationId}`;

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
