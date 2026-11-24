import { engagement } from "@tokuly/api";
import { motion } from "motion/react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { motionFactor } from "./burst";
import type { Subject, ViewerReaction } from "./model";
import { ReactionEmoji } from "./reaction-emoji";

// Reactions from other people, popping up over a subject's reaction bar as they arrive (the realtime `reaction_added`).

type Listener = (reaction: ViewerReaction) => void;

const listeners = new Map<string, Set<Listener>>();
/** What this browser itself just added, so its own reaction coming back over the topic does not pop a second time. */
const own = new Map<string, number>();
const OWN_ECHO_MS = 4000;
/** Pops on screen at once per bar; more are skipped (the counts follow anyway). */
const MAX_POPS = 6;
const POP_SECONDS = 1.3;

const keyOf = (type: Subject["type"], uuid: string) => `${type}:${uuid}`;

/** Called by the feature's background component for every `reaction_added`. */
export function emitPop(type: Subject["type"], uuid: string, reaction: ViewerReaction): void {
  const echo = own.get(reaction.emoji);
  if (echo && Date.now() - echo < OWN_ECHO_MS) {
    own.delete(reaction.emoji);
    return;
  }
  for (const listener of listeners.get(keyOf(type, uuid)) ?? []) listener(reaction);
}

/** The caller just added this reaction somewhere: its echo is not shown. */
export function noteOwnReaction(reaction: ViewerReaction): void {
  own.set(reaction.emoji, Date.now());
}

interface Pop {
  id: number;
  reaction: ViewerReaction;
  x: number;
}

/**
 * Pops for one subject (its UUID): emoji floating up and fading, out of a
 * `relative` box (the "add a reaction" button). Nothing moves, and nothing is shown, when the person asked
 * for less movement or turned ドパガキ度 down to 0.
 */
export function useReactionPops(type: Subject["type"], uuid: string | undefined): ReactNode {
  const [pops, setPops] = useState<Pop[]>([]);
  const next = useRef(0);
  useEffect(() => {
    if (!uuid) return;
    const key = keyOf(type, uuid);
    const timers = new Set<ReturnType<typeof setTimeout>>();
    const listener: Listener = (reaction) => {
      if (motionFactor() <= 0) return;
      const id = ++next.current;
      setPops((current) =>
        current.length >= MAX_POPS ? current : [...current, { id, reaction, x: Math.random() }],
      );
      const timer = setTimeout(
        () => {
          timers.delete(timer);
          setPops((current) => current.filter((pop) => pop.id !== id));
        },
        POP_SECONDS * 1000 + 100,
      );
      timers.add(timer);
    };
    const set = listeners.get(key) ?? new Set<Listener>();
    set.add(listener);
    listeners.set(key, set);
    return () => {
      set.delete(listener);
      if (set.size === 0) listeners.delete(key);
      for (const timer of timers) clearTimeout(timer);
    };
  }, [type, uuid]);

  if (pops.length === 0) return null;
  return (
    <span aria-hidden className="pointer-events-none absolute inset-x-0 bottom-full z-10 h-0">
      {pops.map((pop) => (
        <motion.span
          key={pop.id}
          data-testid="reaction-pop"
          className="absolute bottom-0 text-2xl leading-none"
          style={{ left: `${-60 + pop.x * 160}%` }}
          initial={{ y: 8, opacity: 0, scale: 0.6 }}
          animate={{ y: -36, opacity: [0, 1, 1, 0], scale: 1 }}
          transition={{ duration: POP_SECONDS, ease: "easeOut" }}
        >
          {pop.reaction.kind === engagement.ReactionKind.LIKE ? (
            "💜"
          ) : (
            <ReactionEmoji emoji={pop.reaction.emoji} custom={pop.reaction.custom} size={28} />
          )}
        </motion.span>
      ))}
    </span>
  );
}
