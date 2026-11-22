/**
 * Someone who arrives on a public question box without an account writes
 * their question first and signs up second. What they wrote is kept for a
 * quarter of an hour in this tab only (`sessionStorage`), so the app can
 * bring them back to the box, with their words still there, once they are
 * signed in.
 *
 * It is a question meant to be anonymous, so it must not reach anyone else
 * who uses the device: it never leaves the tab it was written in, nothing is
 * kept when nothing was written, and it is removed when it was used and
 * whenever someone signs out.
 */
const KEY = "tokuly.ask.return";
const LIFETIME_MS = 15 * 60_000;

export interface PendingAsk {
  handle: string;
  text: string;
}

export function rememberPendingAsk(pending: PendingAsk, now: number = Date.now()): void {
  try {
    if (!pending.text.trim()) window.sessionStorage.removeItem(KEY);
    else window.sessionStorage.setItem(KEY, JSON.stringify({ ...pending, at: now }));
  } catch {
    // Storage is full or switched off: the person simply starts over.
  }
}

/** Reads what `rememberPendingAsk` stored (when it is still fresh). */
export function parsePendingAsk(raw: string | null, now: number): PendingAsk | null {
  if (!raw) return null;
  try {
    const value = JSON.parse(raw) as Partial<PendingAsk & { at: number }>;
    if (typeof value.handle !== "string" || !/^[A-Za-z0-9_.-]{1,64}$/.test(value.handle)) return null;
    if (typeof value.text !== "string" || !value.text.trim()) return null;
    if (typeof value.at !== "number" || now - value.at > LIFETIME_MS || now < value.at) return null;
    return { handle: value.handle, text: value.text };
  } catch {
    return null;
  }
}

/** Takes the pending question out of storage: it is acted on once. */
export function takePendingAsk(now: number = Date.now()): PendingAsk | null {
  try {
    const pending = parsePendingAsk(window.sessionStorage.getItem(KEY), now);
    window.sessionStorage.removeItem(KEY);
    return pending;
  } catch {
    return null;
  }
}

let draft: PendingAsk | null = null;

/** Hands a draft to the box page that is about to open. */
export function offerDraft(pending: PendingAsk): void {
  draft = pending;
}

/** The draft for this box, if one was just brought back; consumed by reading. */
export function takeDraft(handle: string): string {
  if (!draft || draft.handle.toLowerCase() !== handle.toLowerCase()) return "";
  const text = draft.text;
  draft = null;
  return text;
}

/** Drops everything a visitor wrote: when someone signs out, the next person on this device starts clean. */
export function forgetPendingAsk(): void {
  draft = null;
  try {
    window.sessionStorage.removeItem(KEY);
  } catch {
    // Nothing was stored then.
  }
}
