import { motion } from "motion/react";
import { useEffect, useRef, useState } from "react";
import { CoinIcon } from "#/components/ui/coins";
import { dopamineFactor } from "#/lib/appearance";

interface Particle {
  id: number;
  /** Where it lands, relative to the centre of the burst. */
  x: number;
  y: number;
  rotate: number;
  delay: number;
  /** A sparkle instead of a coin. */
  sparkle: string | null;
  size: number;
}

interface Burst {
  id: number;
  seconds: number;
  particles: Particle[];
}

const SPARKLES = ["✨", "🎉", "💜", "⭐"];

/**
 * How lively script-driven motion may be right now: the ドパガキ度 factor, and
 * 0 whenever the device asks for reduced motion. (The stylesheet silences CSS
 * animations for that preference, but its `--dopa` still reads as the chosen
 * level there, so the preference is checked here as well.)
 */
function motionFactor(): number {
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return 0;
  return dopamineFactor();
}

/** A burst sized by the ドパガキ度: none at 0 (or reduced motion), a few coins when calm, a shower at the top. */
function makeBurst(id: number, factor: number): Burst {
  const coins = Math.round(10 * factor);
  const sparkles = factor > 1 ? Math.round(14 * (factor - 1)) + 2 : 0;
  const reach = 70 + 70 * factor;
  const particles: Particle[] = [];
  for (let index = 0; index < coins + sparkles; index++) {
    const angle = Math.random() * Math.PI * 2;
    const distance = reach * (0.45 + Math.random() * 0.55);
    particles.push({
      id: index,
      x: Math.cos(angle) * distance,
      // Biased upwards: coins fly up and out, then gravity takes them.
      y: Math.sin(angle) * distance * 0.7 - reach * 0.35,
      rotate: (Math.random() - 0.5) * 540,
      delay: Math.random() * 0.12,
      sparkle: index >= coins ? (SPARKLES[index % SPARKLES.length] ?? null) : null,
      size: 18 + Math.random() * 14,
    });
  }
  return { id, seconds: 0.6 + 0.5 * factor, particles };
}

/**
 * Coins flying out of a reward. Put `<CoinBurst>` inside a `relative` box
 * and call the returned `fire()` when the reward lands. With motion switched
 * off nothing flies; the reward itself must already read as one.
 */
export function useCoinBurst(): { fire(): void; burst: Burst | null } {
  const [burst, setBurst] = useState<Burst | null>(null);
  const count = useRef(0);
  const fire = () => {
    const factor = motionFactor();
    if (factor <= 0) return;
    count.current += 1;
    setBurst(makeBurst(count.current, factor));
    // A little thump on phones that can.
    if (factor >= 1) navigator.vibrate?.(factor > 1 ? [18, 40, 28] : 18);
  };
  useEffect(() => {
    if (!burst) return;
    const timer = setTimeout(() => setBurst(null), (burst.seconds + 0.3) * 1000);
    return () => clearTimeout(timer);
  }, [burst]);
  return { fire, burst };
}

export function CoinBurst({ burst }: { burst: Burst | null }) {
  if (!burst) return null;
  return (
    <span aria-hidden className="pointer-events-none absolute top-1/2 left-1/2 z-10 size-0">
      {burst.particles.map((particle) => (
        <motion.span
          key={`${burst.id}-${particle.id}`}
          className="absolute top-0 left-0 grid place-items-center leading-none"
          style={{
            width: particle.size,
            height: particle.size,
            marginLeft: -particle.size / 2,
            marginTop: -particle.size / 2,
            fontSize: particle.size * 0.9,
          }}
          initial={{ x: 0, y: 0, scale: 0.3, opacity: 1, rotate: 0 }}
          animate={{
            x: particle.x,
            y: [0, particle.y, particle.y + 46],
            scale: [0.3, 1, 0.85],
            opacity: [1, 1, 0],
            rotate: particle.rotate,
          }}
          transition={{
            duration: burst.seconds,
            delay: particle.delay,
            ease: "easeOut",
            times: [0, 0.6, 1],
          }}
        >
          {particle.sparkle ?? <CoinIcon className="size-full text-[0.6em]" />}
        </motion.span>
      ))}
    </span>
  );
}

/**
 * A coin amount that rolls to its new value instead of jumping, so a gain is
 * felt. Follows the ドパガキ度: at 0 (or reduced motion) it simply shows the
 * new number.
 */
export function useCountUp(target: bigint | undefined): bigint | undefined {
  const [shown, setShown] = useState(target);
  const current = useRef(target);

  useEffect(() => {
    const from = current.current;
    const factor = motionFactor();
    if (target === undefined || from === undefined || from === target || factor <= 0) {
      current.current = target;
      setShown(target);
      return;
    }
    const span = Number(target - from);
    const duration = 650 * factor;
    const start = performance.now();
    let frame = requestAnimationFrame(function step(now: number) {
      const progress = Math.min(1, (now - start) / duration);
      const eased = 1 - (1 - progress) ** 3;
      const value = progress === 1 ? target : from + BigInt(Math.round(span * eased));
      current.current = value;
      setShown(value);
      if (progress < 1) frame = requestAnimationFrame(step);
    });
    return () => cancelAnimationFrame(frame);
  }, [target]);

  return shown;
}
