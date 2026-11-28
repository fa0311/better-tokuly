import { type Subject, subjectTopic } from "@tokuly/core/engagement";
import { useEffect, useRef } from "react";
import { realtime } from "#/lib/realtime";
import { useEngagement } from "./api";

/**
 * A connection may follow 64 topics. Content on screen shares them with
 * whatever else the app subscribes to (a live stream, a chat), so subject
 * topics stop a little short of that; a card beyond the budget simply shows
 * the numbers it loaded.
 */
const TOPIC_BUDGET = 44;
/** A card must stay this long before it subscribes: flicking through a list subscribes to nothing. */
const SETTLE_MS = 350;

const followed = new Map<string, number>();

/**
 * Follows a subject's realtime topic for as long as the calling component is
 * mounted, so its counters (and, for notes, its own events) arrive live.
 * Several components may follow the same subject. `uuid` must be the
 * subject's UUID; nothing happens while it is empty. `readAgain` is called
 * whenever the topic takes effect (again after a reconnect) and when it is
 * refused (the viewer may no longer see the subject): what the caller shows
 * of the subject may have changed in between (`docs/realtime-consistency.md`, rule 1).
 */
export function useSubjectTopic(
  type: Subject["type"],
  uuid: string | undefined,
  enabled = true,
  readAgain?: () => void,
): void {
  const latestReadAgain = useRef(readAgain);
  useEffect(() => {
    latestReadAgain.current = readAgain;
  });
  useEffect(() => {
    if (!uuid || !enabled) return;
    const topic = subjectTopic(type, uuid);
    let release: (() => void) | undefined;
    const timer = setTimeout(() => {
      const count = followed.get(topic) ?? 0;
      if (count === 0 && followed.size >= TOPIC_BUDGET) return;
      followed.set(topic, count + 1);
      // The engagement is not read again: every card would read its numbers twice, and the
      // counters a card missed meanwhile come right with the next push.
      const subscription = realtime().subscribe(topic, {
        onSubscribed: () => latestReadAgain.current?.(),
        onRevoked: () => latestReadAgain.current?.(),
      });
      realtime().start();
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
  }, [type, uuid, enabled]);
}

/**
 * Keeps the engagement of `subject` live while the caller is on screen.
 * Works with a short ID too: the topic is followed once the subject's UUID
 * is known from its engagement.
 */
export function useSubjectLive(subject: Subject | undefined, enabled = true): void {
  const { data } = useEngagement(subject);
  useSubjectTopic(subject?.type ?? 0, data?.subjectId, enabled && Boolean(subject));
}
