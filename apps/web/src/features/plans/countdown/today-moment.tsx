import { Dialog as BaseDialog } from "@base-ui/react/dialog";
import { motion } from "motion/react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { dopamineFactor } from "#/lib/appearance";

const CONFETTI = ["🎉", "✨", "🎊", "⭐", "💜", "🌈"];

interface Piece {
  id: number;
  left: number;
  delay: number;
  seconds: number;
  rotate: number;
  size: number;
  glyph: string;
}

/** How much falls: follows the ドパガキ度, nothing when the device asks for reduced motion. */
function makeConfetti(emoji: string): Piece[] {
  if (
    typeof window === "undefined" ||
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  )
    return [];
  const factor = dopamineFactor();
  if (factor <= 0) return [];
  const glyphs = emoji ? [emoji, ...CONFETTI] : CONFETTI;
  return Array.from({ length: Math.round(18 * factor) }, (_, id) => ({
    id,
    left: Math.random() * 100,
    delay: Math.random() * 1.8,
    seconds: 2.6 + Math.random() * 2.2,
    rotate: (Math.random() - 0.5) * 540,
    size: 1.4 + Math.random() * 1.6,
    glyph: glyphs[id % glyphs.length]!,
  }));
}

/**
 * "Today is the day": the whole screen for the moment a countdown reaches
 * zero (or a counter reaches a milestone) — the emoji, the name, confetti.
 */
export function TodayMoment({
  open,
  onOpenChange,
  emoji,
  title,
  kicker,
  detail,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  emoji: string;
  title: string /** The line above the title. */;
  kicker: string;
  detail?: string;
}) {
  const { t } = useTranslation("plans");
  const [pieces] = useState(() => makeConfetti(emoji));
  return (
    <BaseDialog.Root open={open} onOpenChange={onOpenChange}>
      <BaseDialog.Portal>
        <BaseDialog.Popup
          data-testid="today-moment"
          className="fixed inset-0 z-[60] flex flex-col items-center justify-center overflow-hidden bg-brand px-6 text-center text-on-brand outline-none transition-opacity duration-300 data-[ending-style]:opacity-0 data-[starting-style]:opacity-0"
        >
          <div className="pointer-events-none absolute inset-0" aria-hidden>
            {pieces.map((piece) => (
              <motion.span
                key={piece.id}
                className="absolute top-0 select-none"
                style={{ left: `${piece.left}%`, fontSize: `${piece.size}rem` }}
                initial={{ y: "-12vh", rotate: 0, opacity: 0 }}
                animate={{ y: "112vh", rotate: piece.rotate, opacity: [0, 1, 1, 0.8] }}
                transition={{
                  duration: piece.seconds,
                  delay: piece.delay,
                  repeat: Number.POSITIVE_INFINITY,
                  ease: "linear",
                }}
              >
                {piece.glyph}
              </motion.span>
            ))}
          </div>
          <div className="relative flex flex-col items-center">
            <span
              className="animate-float text-[7rem] leading-none drop-shadow-[0_6px_0_rgb(0_0_0/0.25)]"
              aria-hidden
            >
              {emoji || "🎉"}
            </span>
            <BaseDialog.Description className="mt-6 animate-pop-in rounded-full border-[2.5px] border-[#17122b] bg-lime px-5 py-1.5 text-lg font-black text-[#17122b]">
              {kicker}
            </BaseDialog.Description>
            <BaseDialog.Title className="mt-4 max-w-lg animate-slide-up text-4xl leading-tight font-black break-words sm:text-5xl">
              {title}
            </BaseDialog.Title>
            {detail && <p className="mt-3 text-lg font-bold opacity-90">{detail}</p>}
            <Button
              variant="secondary"
              size="lg"
              className="mt-10"
              onClick={() => onOpenChange(false)}
            >
              {t("countdown.momentClose")}
            </Button>
          </div>
        </BaseDialog.Popup>
      </BaseDialog.Portal>
    </BaseDialog.Root>
  );
}

/** Per account: what one person celebrated is not seen by the next one on this browser. */
const MOMENT_SEEN_PREFIX = "tokuly.plans.momentSeen.";
const seenKey = (userId: string) => `${MOMENT_SEEN_PREFIX}${userId}`;

/** Whether the moment for `id` on `day` was shown to this account in this browser already (it opens by itself once). */
export function momentSeen(userId: string, id: string, day: string): boolean {
  try {
    return (
      window.localStorage.getItem(seenKey(userId))?.split(",").includes(`${id}:${day}`) ?? false
    );
  } catch {
    return false;
  }
}

export function markMomentSeen(userId: string, id: string, day: string): void {
  try {
    const seen = (window.localStorage.getItem(seenKey(userId)) ?? "").split(",").filter(Boolean);
    if (!seen.includes(`${id}:${day}`))
      window.localStorage.setItem(seenKey(userId), [...seen.slice(-30), `${id}:${day}`].join(","));
  } catch {
    // Shown again next time, then.
  }
}
