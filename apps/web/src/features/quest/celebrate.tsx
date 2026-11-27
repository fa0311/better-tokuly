import type { quest } from "@tokuly/api";
import { enqueue, type Loudness, loudness } from "@tokuly/core/quest";
import { lazy, Suspense, useEffect } from "react";
import { create } from "zustand";
import { celebrate as rainGlyphs } from "#/features/footprints";
import { dopamineFactor } from "#/lib/appearance";
import { cn } from "#/lib/cn";

/**
 * How lively a celebration may be right now: the ドパガキ度 factor, and 0
 * whenever the device asks for reduced motion (the stylesheet silences CSS
 * animations for that preference, but `--dopa` still reads as the chosen
 * level there).
 */
export function motionFactor(): number {
  if (typeof window === "undefined") return 0;
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return 0;
  return dopamineFactor();
}

/** A moment worth more than a toast. */
export type Reveal =
  | { kind: "badge"; badge: quest.Badge }
  | { kind: "level"; level: number; coins: number }
  | { kind: "streak"; days: number; friendName: string }
  | { kind: "claim"; mission: quest.Mission; reward?: quest.Reward; bonus?: quest.Reward };

export interface Shown {
  id: number;
  reveal: Reveal;
  loud: Exclude<Loudness, "toast">;
}

/** Reveals waiting their turn; the first one is on screen. */
const useReveals = create<{ queue: Shown[] }>(() => ({ queue: [] }));
let nextId = 0;
/** More than this many waiting is a backlog nobody wants to sit through. */
const QUEUE_LIMIT = 6;

const GLYPHS: Record<Reveal["kind"], string[]> = {
  badge: ["🏅", "✨", "🎉", "⭐"],
  level: ["⭐", "🎉", "✨", "🆙"],
  streak: ["🔥", "✨", "🔥", "💜"],
  claim: ["🪙", "✨", "🎉", "⭐"],
};

/**
 * Shows a reveal in proportion to the ドパガキ度 and says how it went:
 * `"toast"` means nothing was drawn (the level is low, or motion is off) and
 * the caller should say it in a toast instead. Reveals queue: each one waits
 * until the one before it has gone.
 */
export function reveal(item: Reveal): Loudness {
  const loud = loudness(motionFactor());
  if (loud === "toast") return loud;
  useReveals.setState(({ queue }) => ({
    queue: enqueue(queue, { id: ++nextId, reveal: item, loud }, QUEUE_LIMIT),
  }));
  return loud;
}

/** Drops everything waiting (the account signed out). */
export function clearReveals(): void {
  useReveals.setState({ queue: [] });
}

export const dismiss = (id: number) =>
  useReveals.setState(({ queue }) => ({ queue: queue.filter((item) => item.id !== id) }));

const RevealCard = lazy(() =>
  import("./reveal-card").then((module) => ({ default: module.RevealCard })),
);

/** How long a reveal stays unless it is tapped away. */
const SHOW_MS = { card: 4200, full: 5200 } as const;

/**
 * Where reveals are drawn. The layer never takes input away from the page
 * under it: only the card's two buttons can be tapped (open what it is
 * about, close) and it leaves on its own.
 */
export function RevealLayer() {
  const current = useReveals((state) => state.queue[0]);
  const waiting = useReveals((state) => Math.max(0, state.queue.length - 1));

  const id = current?.id;
  const loud = current?.loud;
  const kind = current?.reveal.kind;
  useEffect(() => {
    if (id === undefined || !loud || !kind) return;
    rainGlyphs(GLYPHS[kind]);
    const timer = setTimeout(() => dismiss(id), SHOW_MS[loud]);
    return () => clearTimeout(timer);
  }, [id, loud, kind]);

  if (!current) return null;
  const full = current.loud === "full";
  return (
    <div
      className={cn(
        "pointer-events-none fixed inset-0 z-[65] flex justify-center px-4",
        full ? "items-center" : "items-start pt-[max(5rem,env(safe-area-inset-top))]",
      )}
    >
      {full && (
        <div
          aria-hidden
          className="absolute inset-0 animate-fade-in"
          style={{
            background:
              "radial-gradient(circle at 50% 45%, color-mix(in srgb, var(--sun) 55%, transparent) 0%, color-mix(in srgb, var(--brand) 35%, transparent) 45%, transparent 75%)",
          }}
        />
      )}
      <Suspense fallback={null}>
        <RevealCard key={current.id} shown={current} waiting={waiting} />
      </Suspense>
    </div>
  );
}
