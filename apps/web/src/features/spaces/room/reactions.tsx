import { errorReason } from "@tokuly/api";
import { SmilePlus } from "lucide-react";
import { motion } from "motion/react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { EmojiPicker, EmojiText } from "#/features/emoji";
import { dopamineFactor } from "#/lib/appearance";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { spaceClient } from "../api";
import { emitReaction, onReaction } from "../live";

/** The emoji one tap away; the picker has every other one. */
const QUICK = ["👏", "😂", "❤️", "🔥", "😮", "🎉"];
/** More than this at once is more than anyone could watch. */
const MAX_FLYING = 36;

interface Flying {
  id: number;
  emoji: string;
  /** Where it starts, in percent of the room's width. */
  left: number;
  drift: number;
  seconds: number;
}

/** How lively the bursts may be: the ドパガキ度 factor, 0 when the device asks for reduced motion. */
function motionFactor(): number {
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return 0;
  return dopamineFactor();
}

/**
 * The emoji people send, floating up over the room. With motion off they
 * appear in place for a moment instead, so nothing is lost but the movement.
 */
export function ReactionLayer({ spaceId }: { spaceId: string }) {
  const [flying, setFlying] = useState<Flying[]>([]);
  const count = useRef(0);

  useEffect(() => {
    const timers = new Set<ReturnType<typeof setTimeout>>();
    const off = onReaction((reaction) => {
      if (reaction.spaceId !== spaceId) return;
      const factor = motionFactor();
      const item: Flying = {
        id: ++count.current,
        emoji: reaction.emoji,
        left: 8 + Math.random() * 78,
        drift: (Math.random() - 0.5) * 60 * factor,
        seconds: factor > 0 ? 2 + Math.random() * 0.8 : 1.6,
      };
      setFlying((current) => [...current.slice(-(MAX_FLYING - 1)), item]);
      const timer = setTimeout(() => {
        timers.delete(timer);
        setFlying((current) => current.filter((other) => other.id !== item.id));
      }, item.seconds * 1000);
      timers.add(timer);
    });
    return () => {
      off();
      for (const timer of timers) clearTimeout(timer);
    };
  }, [spaceId]);

  const moving = typeof window !== "undefined" && motionFactor() > 0;
  return (
    <div aria-hidden className="pointer-events-none absolute inset-0 z-10 overflow-hidden">
      {flying.map((item) => (
        <motion.span
          key={item.id}
          data-testid="space-reaction"
          className="absolute bottom-40 block text-4xl leading-none drop-shadow-sm md:bottom-32"
          style={{ left: `${item.left}%` }}
          initial={{ y: 0, x: 0, opacity: 0, scale: 0.4 }}
          animate={
            moving
              ? { y: "-55vh", x: item.drift, opacity: [0, 1, 1, 0], scale: [0.4, 1.15, 1, 0.9] }
              : { opacity: [0, 1, 1, 0], scale: 1 }
          }
          transition={{ duration: item.seconds, ease: "easeOut" }}
        >
          <EmojiText text={item.emoji} emojiSize={36} />
        </motion.span>
      ))}
    </div>
  );
}

/** The row of emoji to send into the room. */
export function ReactionBar({ spaceId }: { spaceId: string }) {
  const { t } = useTranslation("spaces");
  const myId = useMe()?.user?.id ?? "";
  const send = (emoji: string) => {
    // The sender sees their own burst at once; the others get it over realtime.
    emitReaction({ spaceId, userId: myId, emoji });
    spaceClient()
      .sendReaction({ spaceId, emoji })
      .catch((error: unknown) => {
        // Sent faster than the room takes them: the burst simply does not travel.
        if (errorReason(error) !== "RATE_LIMITED")
          toast.error(errorMessage(error, { namespace: "spaces" }));
      });
  };
  const button =
    "pressable grid size-10 shrink-0 place-items-center rounded-full bg-surface-2 text-xl leading-none";
  return (
    <fieldset className="scrollbar-none flex min-w-0 items-center gap-1.5 overflow-x-auto">
      <legend className="sr-only">{t("room.reactions")}</legend>
      {QUICK.map((emoji) => (
        <button
          key={emoji}
          type="button"
          aria-label={t("room.react", { emoji })}
          onClick={() => send(emoji)}
          className={button}
        >
          {emoji}
        </button>
      ))}
      <EmojiPicker
        onPick={send}
        trigger={
          <button type="button" aria-label={t("room.moreEmoji")} className={button}>
            <SmilePlus className="size-5 text-ink-soft" />
          </button>
        }
      />
    </fieldset>
  );
}
