import { engagement } from "@tokuly/api";
import type { Subject, ViewerReaction } from "@tokuly/core/engagement";
import { motion } from "motion/react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { motionFactor } from "./burst";
import { keyOf, type Listener, listeners, MAX_POPS, POP_SECONDS } from "./pop-events";
import { ReactionEmoji } from "./reaction-emoji";

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
