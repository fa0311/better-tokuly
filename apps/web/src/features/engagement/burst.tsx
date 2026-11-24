import { motion } from "motion/react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { dopamineFactor } from "#/lib/appearance";

interface Particle {
  id: number;
  x: number;
  y: number;
  rotate: number;
  scale: number;
  glyph: string;
}

interface Shot {
  id: number;
  seconds: number;
  particles: Particle[];
}

/** How lively script-driven motion may be: the ドパガキ度 factor, 0 when the device asks for reduced motion. */
export function motionFactor(): number {
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return 0;
  return dopamineFactor();
}

/**
 * A little burst of glyphs flying out of a control when a tap lands: hearts
 * out of a like, the emoji out of a reaction. Put `burst` inside a `relative`
 * box around the control and call `fire()` when the tap counts. How much
 * flies follows the ドパガキ度; with motion off nothing does.
 */
export function useBurst(glyphs: string[]): {
  /** `only` replaces the glyphs for this one burst (the emoji just picked). */ fire(
    only?: string[],
  ): void;
  burst: ReactNode;
} {
  const [shot, setShot] = useState<Shot | null>(null);
  const count = useRef(0);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);

  const fire = (only?: string[]) => {
    const flying = only ?? glyphs;
    const factor = motionFactor();
    if (factor <= 0 || flying.length === 0) return;
    const amount = Math.round(5 + 5 * factor);
    const reach = 20 + 18 * factor;
    const particles: Particle[] = Array.from({ length: amount }, (_, index) => {
      const angle = (index / amount) * Math.PI * 2 + Math.random() * 0.6;
      const distance = reach * (0.6 + Math.random() * 0.5);
      return {
        id: index,
        x: Math.cos(angle) * distance,
        // Biased upwards, like something popping off the button.
        y: Math.sin(angle) * distance - reach * 0.35,
        rotate: (Math.random() - 0.5) * 120,
        scale: 0.6 + Math.random() * 0.6,
        glyph: flying[index % flying.length]!,
      };
    });
    const seconds = 0.45 + 0.25 * factor;
    setShot({ id: ++count.current, seconds, particles });
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setShot(null), seconds * 1000 + 80);
  };

  const burst = shot && (
    <span aria-hidden className="pointer-events-none absolute top-1/2 left-1/2 z-10 size-0">
      {shot.particles.map((particle) => (
        <motion.span
          key={`${shot.id}:${particle.id}`}
          className="absolute -top-2 -left-2 block size-4 text-center text-xs leading-4"
          initial={{ x: 0, y: 0, opacity: 1, scale: 0.2, rotate: 0 }}
          animate={{
            x: particle.x,
            y: particle.y,
            opacity: 0,
            scale: particle.scale,
            rotate: particle.rotate,
          }}
          transition={{
            duration: shot.seconds,
            ease: [0.22, 1, 0.36, 1],
            opacity: { duration: shot.seconds, ease: "easeIn" },
          }}
        >
          {particle.glyph}
        </motion.span>
      ))}
    </span>
  );
  return { fire, burst };
}
