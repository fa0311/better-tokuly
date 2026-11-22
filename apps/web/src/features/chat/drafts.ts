/**
 * What was typed but not sent, per conversation. Drafts are a client-side
 * concern: they live in this browser's storage and nowhere else.
 */
const PREFIX = "tokuly.chat.draft.";

export function loadDraft(conversationId: string): string {
  try {
    return window.localStorage.getItem(PREFIX + conversationId) ?? "";
  } catch {
    return "";
  }
}

export function saveDraft(conversationId: string, text: string): void {
  try {
    if (text.trim()) window.localStorage.setItem(PREFIX + conversationId, text);
    else window.localStorage.removeItem(PREFIX + conversationId);
  } catch {
    // Storage is full or blocked: the draft simply does not survive a reload.
  }
}
