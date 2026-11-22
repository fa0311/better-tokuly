import { RotateCcw } from "lucide-react";
import { motion } from "motion/react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { dopamineFactor } from "#/lib/appearance";

const GLYPHS = ["✨", "💜", "⭐", "🎉", "💫"];

interface Particle {
  id: number;
  glyph: string;
  x: number;
  y: number;
  rotate: number;
  delay: number;
}

/** How many sparkles a celebration throws at a liveliness factor (none when motion is off). */
function sparkleCount(factor: number): number {
  return Math.round(8 * factor);
}

function makeParticles(factor: number): Particle[] {
  const reach = 50 + 60 * factor;
  return Array.from({ length: sparkleCount(factor) }, (_, id) => {
    const angle = (id / sparkleCount(factor)) * Math.PI * 2 + Math.random() * 0.6;
    const distance = reach * (0.6 + Math.random() * 0.4);
    return {
      id,
      glyph: GLYPHS[id % GLYPHS.length] ?? "✨",
      x: Math.cos(angle) * distance * 1.5,
      y: Math.sin(angle) * distance * 0.75,
      rotate: (Math.random() - 0.5) * 240,
      delay: Math.random() * 0.1,
    };
  });
}

/** One celebration: sparkles flying out from the middle of its (relative) parent. */
function Sparkles() {
  // Read once per run, after the level was applied to the page.
  const [factor] = useState(dopamineFactor);
  const [particles] = useState(() => makeParticles(factor));
  return (
    <span aria-hidden className="pointer-events-none absolute top-1/2 left-1/2">
      {particles.map((particle) => (
        <motion.span
          key={particle.id}
          className="absolute -mt-3 -ml-3 text-xl"
          initial={{ x: 0, y: 0, opacity: 0, scale: 0.3 }}
          animate={{
            x: particle.x,
            y: particle.y,
            opacity: [0, 1, 1, 0],
            scale: 1,
            rotate: particle.rotate,
          }}
          transition={{ duration: 0.5 + 0.5 * factor, delay: particle.delay, ease: "easeOut" }}
        >
          {particle.glyph}
        </motion.span>
      ))}
    </span>
  );
}

/**
 * Shows what the chosen ドパガキ度 does with the real thing: a card that pops
 * in with the app's own animation (its speed and bounce follow the level), a
 * celebration whose sparkles grow with it, and a button to feel the press.
 * It replays whenever the level changes.
 */
export function DopaminePreview({ level }: { level: number }) {
  const { t } = useTranslation("settings");
  const [replays, setReplays] = useState(0);
  const run = `${level}:${replays}`;
  const replay = () => setReplays((count) => count + 1);

  return (
    <div
      className="relative mt-4 overflow-hidden rounded-2xl bg-surface-2 px-4 pt-8 pb-4"
      data-testid="dopamine-preview"
    >
      <div className="relative mx-auto w-fit max-w-full">
        {/* A new key restarts the entrance animation. */}
        <div
          key={`card:${run}`}
          className="sticker flex animate-pop-in items-center gap-3 rounded-3xl bg-surface py-3 pr-5 pl-4"
        >
          <span className="animate-wiggle text-3xl">🎉</span>
          <span className="min-w-0">
            <span className="block truncate font-black">
              {t("appearance.dopamine.preview.title")}
            </span>
            <span className="block truncate text-sm text-ink-soft">
              {t("appearance.dopamine.preview.body")}
            </span>
          </span>
        </div>
        <Sparkles key={`sparkles:${run}`} />
      </div>
      <div className="mt-8 flex items-center justify-between gap-3">
        <p className="min-w-0 text-xs font-bold leading-relaxed text-ink-soft">
          {t(`appearance.dopamine.effects.${level}`)}
        </p>
        <Button
          size="sm"
          variant="secondary"
          className="shrink-0"
          icon={<RotateCcw className="size-4" />}
          onClick={replay}
        >
          {t("appearance.dopamine.preview.replay")}
        </Button>
      </div>
    </div>
  );
}
