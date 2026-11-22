import { chat } from "@tokuly/api";
import { useEffect, useState } from "react";
import { create } from "zustand";

/** Clients drop a typing indicator that was not refreshed for this long. */
const TYPING_TTL_MS = 6_000;

interface LiveStore {
  /** conversation → user → when their typing indicator runs out. */
  typing: Record<string, Record<string, number>>;
  /** conversation → who has it open right now. */
  present: Record<string, string[]>;
  /** conversation → user → how far they have read (`seq`). */
  cursors: Record<string, Record<string, bigint>>;
  /** conversation → the POV request waiting for an answer. */
  openPov: Record<string, chat.PovRequest | undefined>;
  /** Calls that are ringing or running, by ID. */
  calls: Record<string, chat.Call>;
  /** Who has the app open, as far as presence events told us. */
  online: Record<string, boolean>;
  /** The conversation on screen whose new messages are being read as they arrive. */
  readingId: string | null;
  /** When each piece of per-conversation state was true, so an answer that was under way cannot undo a newer event. */
  asOf: Record<string, number>;
  /** Calls known to be over: nothing may bring one back. */
  endedCalls: Record<string, true>;
  setTyping(conversationId: string, userId: string, typing: boolean): void;
  /** `asOf`: when this was true (when the request that produced it was sent); events are true now. */
  setPresent(conversationId: string, userIds: string[], asOf?: number): void;
  setCursors(conversationId: string, cursors: chat.ReadCursor[]): void;
  setCursor(conversationId: string, userId: string, readSeq: bigint): void;
  setOpenPov(conversationId: string, request: chat.PovRequest | undefined, asOf?: number): void;
  setCall(call: chat.Call): void;
  setOnline(userId: string, online: boolean): void;
  setReading(conversationId: string | null): void;
  reset(): void;
}

const EMPTY = { typing: {}, present: {}, cursors: {}, openPov: {}, calls: {}, online: {}, readingId: null, asOf: {}, endedCalls: {} };

/**
 * What the realtime connection tells us about conversations that is not part
 * of any query: who is typing, who is here, how far everyone has read, the
 * open POV request and the calls in progress.
 */
export const useLive = create<LiveStore>((set) => ({
  ...EMPTY,
  setTyping: (conversationId, userId, typing) =>
    set((state) => {
      const { [userId]: _gone, ...others } = state.typing[conversationId] ?? {};
      return { typing: { ...state.typing, [conversationId]: typing ? { ...others, [userId]: Date.now() + TYPING_TTL_MS } : others } };
    }),
  setPresent: (conversationId, userIds, asOf = Date.now()) =>
    set((state) => {
      const key = `present:${conversationId}`;
      if ((state.asOf[key] ?? 0) > asOf) return state;
      return { present: { ...state.present, [conversationId]: userIds }, asOf: { ...state.asOf, [key]: asOf } };
    }),
  setCursors: (conversationId, cursors) =>
    set((state) => {
      // Cursors only move forward: an answer that was on its way must not take back a newer event.
      const merged = { ...state.cursors[conversationId] };
      for (const cursor of cursors) {
        if ((merged[cursor.userId] ?? -1n) < cursor.readSeq) merged[cursor.userId] = cursor.readSeq;
      }
      return { cursors: { ...state.cursors, [conversationId]: merged } };
    }),
  setCursor: (conversationId, userId, readSeq) =>
    set((state) => {
      const held = state.cursors[conversationId] ?? {};
      if ((held[userId] ?? -1n) >= readSeq) return state;
      return { cursors: { ...state.cursors, [conversationId]: { ...held, [userId]: readSeq } } };
    }),
  setOpenPov: (conversationId, request, asOf = Date.now()) =>
    set((state) => {
      const key = `pov:${conversationId}`;
      if ((state.asOf[key] ?? 0) > asOf) return state;
      return { openPov: { ...state.openPov, [conversationId]: request?.state === chat.PovState.OPEN ? request : undefined }, asOf: { ...state.asOf, [key]: asOf } };
    }),
  setCall: (call) =>
    set((state) => {
      if (call.state === chat.CallState.ENDED) {
        const { [call.id]: _ended, ...calls } = state.calls;
        return { calls, endedCalls: { ...state.endedCalls, [call.id]: true } };
      }
      if (state.endedCalls[call.id]) return state;
      return { calls: { ...state.calls, [call.id]: call } };
    }),
  setOnline: (userId, online) => set((state) => (state.online[userId] === online ? state : { online: { ...state.online, [userId]: online } })),
  setReading: (readingId) => set({ readingId }),
  reset: () => set(EMPTY),
}));

const NOBODY: string[] = [];

/** Who is typing in a conversation right now (never the viewer). Indicators expire by themselves. */
export function useTyping(conversationId: string, myId: string): string[] {
  const entries = useLive((state) => state.typing[conversationId]);
  const [, tick] = useState(0);
  const now = Date.now();
  const live = entries ? Object.entries(entries).filter(([userId, until]) => userId !== myId && until > now) : [];
  const soonest = live.length > 0 ? Math.min(...live.map(([, until]) => until)) : 0;

  useEffect(() => {
    if (!soonest) return;
    const timer = setTimeout(() => tick((value) => value + 1), Math.max(50, soonest - Date.now() + 20));
    return () => clearTimeout(timer);
  }, [soonest]);

  return live.length > 0 ? live.map(([userId]) => userId) : NOBODY;
}

/** Who else has the conversation open right now. */
export function usePresent(conversationId: string, myId: string): string[] {
  const present = useLive((state) => state.present[conversationId]);
  if (!present) return NOBODY;
  const others = present.filter((userId) => userId !== myId);
  return others.length > 0 ? others : NOBODY;
}

/** The live call of a conversation, when one is known. */
export function useLiveCall(callId: string | undefined): chat.Call | undefined {
  return useLive((state) => (callId ? state.calls[callId] : undefined));
}
