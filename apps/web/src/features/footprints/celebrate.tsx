import { motion } from "motion/react";
import { useEffect, useRef, useState } from "react";
import { dopamineFactor } from "#/lib/appearance";

const DEFAULT_GLYPHS = ["🎉", "✨", "👣", "🗺️", "⭐"];
const listeners = new Set<(glyphs: string[]) => void>();

/**
 * Rains glyphs down the whole screen: for a level-up, a region entered for
 * the first time, a recap worth showing off. How much falls follows the
 * ドパガキ度; with motion off nothing does (the toast or card that goes with
 * it says the same thing in words).
 */
export function celebrate(glyphs: string[] = DEFAULT_GLYPHS): void {
  for (const listener of listeners) listener(glyphs);
}

interface Piece {
  id: number;
  glyph: string;
  left: number;
  delay: number;
  drift: number;
  spin: number;
  size: number;
}

interface Shower {
  id: number;
  seconds: number;
  pieces: Piece[];
}

/** Where `celebrate()` draws. Mounted once for the signed-in app. */
export function CelebrationLayer() {
  const [shower, setShower] = useState<Shower | null>(null);
  const count = useRef(0);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);

  useEffect(() => {
    const listener = (glyphs: string[]) => {
      const factor = window.matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : dopamineFactor();
      if (factor <= 0 || glyphs.length === 0) return;
      const amount = Math.round(14 + 16 * factor);
      const seconds = 1.6 + 0.6 * factor;
      setShower({
        id: ++count.current,
        seconds,
        pieces: Array.from({ length: amount }, (_, index) => ({
          id: index,
          glyph: glyphs[index % glyphs.length]!,
          left: Math.random() * 100,
          delay: Math.random() * 0.5,
          drift: (Math.random() - 0.5) * 120,
          spin: (Math.random() - 0.5) * 540,
          size: 1.2 + Math.random() * 1.4,
        })),
      });
      clearTimeout(timer.current);
      timer.current = setTimeout(() => setShower(null), (seconds + 0.6) * 1000);
    };
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
      clearTimeout(timer.current);
    };
  }, []);

  if (!shower) return null;
  return (
    <div key={shower.id} className="pointer-events-none fixed inset-0 z-[70] overflow-hidden" aria-hidden>
      {shower.pieces.map((piece) => (
        <motion.span
          key={piece.id}
          className="absolute top-0 select-none"
          style={{ left: `${piece.left}%`, fontSize: `${piece.size}rem` }}
          initial={{ y: "-10vh", x: 0, rotate: 0, opacity: 1 }}
          animate={{ y: "110vh", x: piece.drift, rotate: piece.spin, opacity: [1, 1, 0.9, 0] }}
          transition={{ duration: shower.seconds, delay: piece.delay, ease: "easeIn" }}
        >
          {piece.glyph}
        </motion.span>
      ))}
    </div>
  );
}
