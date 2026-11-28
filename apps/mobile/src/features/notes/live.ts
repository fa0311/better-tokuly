import { type QueryClient, useQueryClient } from "@tanstack/react-query";
import { common } from "@tokuly/api";
import { subjectTopic } from "@tokuly/core/engagement";
import { applyNotesEvent, isNoteNotification, patchNote } from "@tokuly/core/notes";
import { useEffect } from "react";
import { AppState } from "react-native";
import { realtime } from "#/lib/realtime";
import { markNoteDeleted, noteClient } from "./data";

/**
 * "Something new may belong on top of a timeline": timelines on screen
 * listen and look. The connection has no event for a friend's new note, so
 * the hints come from what it does push (a notification about a note, a
 * reply, quote or renote count going up on a note on screen, the connection
 * opening) and from the app returning to the front.
 */
const hintListeners = new Set<() => void>();

export function onTimelineHint(listener: () => void): () => void {
  hintListeners.add(listener);
  return () => hintListeners.delete(listener);
}

/** Tells every timeline on screen to look (the realtime listener below, and tests). */
export function hintTimelines(): void {
  for (const listener of hintListeners) listener();
}

/** Puts a note as the server shows it now into every cached list (its counters move on their own). */
async function refreshNote(queryClient: QueryClient, noteId: string): Promise<void> {
  const client = noteClient();
  try {
    const { note: fresh } = await client.getNote({ noteId });
    if (fresh)
      patchNote(queryClient, noteId, (note) => ({
        ...fresh,
        counters: note.counters ?? fresh.counters,
      }));
  } catch {
    // Gone or hidden meanwhile: the other pushes say so.
  }
}

/** Keeps the cached notes current from the realtime connection while someone is signed in. */
export function useNotesSync(): void {
  const queryClient = useQueryClient();
  useEffect(() => {
    const client = realtime();
    const off = client.onFrame((frame) => {
      const body = frame.body;
      if (body.case === "welcome") {
        // Pushes arrive from `Welcome` on: whatever was posted before it — while a timeline was
        // already loading or on screen, or while the connection was down — was not announced.
        hintTimelines();
        return;
      }
      if (body.case === "notify") {
        const event = body.value.event;
        if (event.case === "received" && isNoteNotification(event.value.notification?.kind ?? ""))
          hintTimelines();
        return;
      }
      if (body.case !== "notes") return;
      const effect = applyNotesEvent(queryClient, body.value.event);
      if (effect.hint) hintTimelines();
      if (effect.deleted) markNoteDeleted(queryClient, effect.deleted);
      if (effect.refresh) void refreshNote(queryClient, effect.refresh);
    });
    client.start();
    const state = AppState.addEventListener("change", (next) => {
      if (next === "active") hintTimelines();
    });
    return () => {
      off();
      state.remove();
    };
  }, [queryClient]);
}

/**
 * A connection may follow 64 topics, shared with whatever else the app
 * follows: notes stop a little short of that, and a card beyond the budget
 * shows the numbers it loaded.
 */
const TOPIC_BUDGET = 44;
/** A card must stay this long before it follows its note: flicking through a list follows nothing. */
const SETTLE_MS = 350;
const followed = new Map<string, number>();

/**
 * Follows a note's realtime topic while the calling card is mounted, so its
 * counters, edits, poll and deletion arrive live. Several cards may follow
 * the same note.
 */
export function useNoteTopic(noteId: string, enabled = true): void {
  useEffect(() => {
    if (!noteId || !enabled) return;
    const topic = subjectTopic(common.SubjectType.NOTE, noteId);
    let release: (() => void) | undefined;
    const timer = setTimeout(() => {
      const count = followed.get(topic) ?? 0;
      if (count === 0 && followed.size >= TOPIC_BUDGET) return;
      followed.set(topic, count + 1);
      const client = realtime();
      const subscription = client.subscribe(topic);
      client.start();
      release = () => {
        subscription.release();
        const remaining = (followed.get(topic) ?? 1) - 1;
        if (remaining > 0) followed.set(topic, remaining);
        else followed.delete(topic);
      };
    }, SETTLE_MS);
    return () => {
      clearTimeout(timer);
      release?.();
    };
  }, [noteId, enabled]);
}
