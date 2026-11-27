import { motion } from "motion/react";
import type { Shower } from "./celebrate";

/** One shower of glyphs falling down the screen. */
export function Falling({ shower }: { shower: Shower }) {
  return (
    <div className="pointer-events-none fixed inset-0 z-[70] overflow-hidden" aria-hidden>
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
