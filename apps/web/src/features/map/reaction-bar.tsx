import { social } from "@tokuly/api";
import { Smile, Sticker } from "lucide-react";
import { type PointerEvent, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { EmojiPicker } from "#/features/engagement";
import { StampPicker } from "#/features/stamps";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { interactionClient } from "./data";
import { emitEffect } from "./effects";

const QUICK = [
  { key: "hearts", kind: social.MapReactionKind.HEARTS, glyph: "❤️" },
  { key: "eyes", kind: social.MapReactionKind.EYES, glyph: "👀" },
  { key: "drinks", kind: social.MapReactionKind.DRINKS, glyph: "🍻" },
] as const;

/** Holding a reaction charges it: one more level every this many milliseconds, up to 5. */
const CHARGE_STEP_MS = 350;

/**
 * Things to throw at a friend's pin — or at everyone in a gathering: hearts,
 * eyes, "let's hang out", any emoji, a stamp. A tap sends a small one;
 * holding charges it up to a big one.
 */
export function ReactionBar({ userIds }: { userIds: string[] }) {
  const { t } = useTranslation("map");
  const [charge, setCharge] = useState<{ key: string; level: number } | null>(null);
  const hold = useRef<{
    key: string;
    startedAt: number;
    timer: ReturnType<typeof setInterval>;
  } | null>(null);
  useEffect(() => () => clearInterval(hold.current?.timer), []);

  const send = async (
    request: { kind: social.MapReactionKind; emoji?: string; stampId?: string },
    intensity: number,
  ) => {
    try {
      const response = await interactionClient().sendMapReaction({
        userIds,
        intensity,
        ...request,
      });
      for (const reaction of response.reactions)
        emitEffect({ kind: "reaction", reaction, direction: "outgoing" });
      toast.success(t("reactions.sent"), { id: "map-reaction-sent", duration: 1500 });
    } catch (error) {
      toast.error(errorMessage(error, { namespace: "map" }));
    }
  };

  const release = (fire: boolean) => {
    const held = hold.current;
    if (!held) return;
    clearInterval(held.timer);
    hold.current = null;
    setCharge(null);
    const quick = QUICK.find((entry) => entry.key === held.key);
    if (fire && quick)
      void send(
        { kind: quick.kind },
        Math.min(5, 1 + Math.floor((Date.now() - held.startedAt) / CHARGE_STEP_MS)),
      );
  };

  const press = (key: string) => (event: PointerEvent<HTMLButtonElement>) => {
    if (event.button !== 0) return;
    release(false);
    const startedAt = Date.now();
    const timer = setInterval(
      () =>
        setCharge({
          key,
          level: Math.min(5, 1 + Math.floor((Date.now() - startedAt) / CHARGE_STEP_MS)),
        }),
      100,
    );
    hold.current = { key, startedAt, timer };
    setCharge({ key, level: 1 });
  };

  return (
    <div>
      <div className="flex items-center justify-between gap-1">
        {QUICK.map((quick) => {
          const level = charge?.key === quick.key ? charge.level : 0;
          return (
            <button
              key={quick.key}
              type="button"
              aria-label={t(`reactions.${quick.key}`)}
              title={t(`reactions.${quick.key}`)}
              onPointerDown={press(quick.key)}
              onPointerUp={() => release(true)}
              onPointerLeave={() => release(false)}
              onPointerCancel={() => release(false)}
              // Keyboard and assistive technology click without a pointer.
              onClick={(event) => event.detail === 0 && void send({ kind: quick.kind }, 1)}
              onContextMenu={(event) => event.preventDefault()}
              className={cn(
                "pressable relative grid size-12 touch-none place-items-center rounded-full bg-surface-2 text-2xl select-none",
                level > 0 && "bg-brand-soft",
              )}
              style={level > 0 ? { transform: `scale(${1 + level * 0.12})` } : undefined}
            >
              <span aria-hidden>{quick.glyph}</span>
              {level > 1 && (
                <span className="sticker-sm absolute -top-1 -right-1 grid size-5 place-items-center rounded-full bg-pink text-[10px] font-black text-white">
                  ×{level}
                </span>
              )}
            </button>
          );
        })}
        <EmojiPicker
          onPick={(emoji) => void send({ kind: social.MapReactionKind.EMOJI, emoji }, 2)}
          trigger={
            <button
              type="button"
              aria-label={t("reactions.emoji")}
              title={t("reactions.emoji")}
              className="pressable grid size-12 place-items-center rounded-full bg-surface-2"
            >
              <Smile className="size-6 text-sun" />
            </button>
          }
        />
        <StampPicker
          onPick={(stamp) =>
            void send({ kind: social.MapReactionKind.STAMP, stampId: stamp.id }, 2)
          }
          trigger={
            <button
              type="button"
              aria-label={t("reactions.stamp")}
              title={t("reactions.stamp")}
              className="pressable grid size-12 place-items-center rounded-full bg-surface-2"
            >
              <Sticker className="size-6 text-brand" />
            </button>
          }
        />
      </div>
      <p className="mt-1.5 text-center text-[11px] font-bold text-ink-faint">
        {t("reactions.holdHint")}
      </p>
    </div>
  );
}
