import { useEffect, useState } from "react";
import { create } from "zustand";

/** An indicator that was not refreshed for this long is dropped. */
const TYPING_TTL_MS = 6_000;

interface LiveStore {
  /** conversation → user → when their typing indicator runs out. */
  typing: Record<string, Record<string, number>>;
  /** conversation → user → how far they have read (`seq`). */
  cursors: Record<string, Record<string, bigint>>;
  /** The conversation on screen whose new messages are being read as they arrive. */
  readingId: string | null;
  setTyping(conversationId: string, userId: string, typing: boolean): void;
  /** Cursors only move forward: an answer that was on its way never takes back a newer event. */
  moveCursor(conversationId: string, userId: string, readSeq: bigint): void;
  setReading(conversationId: string | null): void;
  reset(): void;
}

/**
 * What the realtime connection says about conversations that no query
 * holds: who is typing, how far everyone has read, which one is being read.
 */
export const useLive = create<LiveStore>((set) => ({
  typing: {},
  cursors: {},
  readingId: null,
  setTyping: (conversationId, userId, typing) =>
    set((state) => {
      const { [userId]: _gone, ...others } = state.typing[conversationId] ?? {};
      return {
        typing: {
          ...state.typing,
          [conversationId]: typing ? { ...others, [userId]: Date.now() + TYPING_TTL_MS } : others,
        },
      };
    }),
  moveCursor: (conversationId, userId, readSeq) =>
    set((state) => {
      const held = state.cursors[conversationId] ?? {};
      if ((held[userId] ?? -1n) >= readSeq) return state;
      return { cursors: { ...state.cursors, [conversationId]: { ...held, [userId]: readSeq } } };
    }),
  setReading: (readingId) => set({ readingId }),
  reset: () => set({ typing: {}, cursors: {}, readingId: null }),
}));

const NOBODY: string[] = [];

/** Who else is typing in a conversation right now. Indicators run out by themselves. */
export function useTyping(conversationId: string, myId: string): string[] {
  const entries = useLive((state) => state.typing[conversationId]);
  const [, tick] = useState(0);
  const now = Date.now();
  const live = entries
    ? Object.entries(entries).filter(([userId, until]) => userId !== myId && until > now)
    : [];
  const soonest = live.length > 0 ? Math.min(...live.map(([, until]) => until)) : 0;
  useEffect(() => {
    if (!soonest) return;
    const timer = setTimeout(() => tick((n) => n + 1), Math.max(50, soonest - Date.now() + 20));
    return () => clearTimeout(timer);
  }, [soonest]);
  return live.length > 0 ? live.map(([userId]) => userId) : NOBODY;
}
