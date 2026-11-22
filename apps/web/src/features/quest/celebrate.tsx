import { useNavigate } from "@tanstack/react-router";
import type { quest } from "@tokuly/api";
import { X } from "lucide-react";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { create } from "zustand";
import { Coins } from "#/components/ui/coins";
import { EmojiText } from "#/features/emoji";
import { celebrate as rainGlyphs } from "#/features/footprints";
import { dopamineFactor } from "#/lib/appearance";
import { cn } from "#/lib/cn";
import { useLocalized } from "./api";
import { type Loudness, loudness } from "./model";
import { BadgeMedal, useBadgeName } from "./parts";

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

interface Shown {
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
    queue: [...queue, { id: ++nextId, reveal: item, loud }].slice(-QUEUE_LIMIT),
  }));
  return loud;
}

/** Drops everything waiting (the account signed out). */
export function clearReveals(): void {
  useReveals.setState({ queue: [] });
}

const dismiss = (id: number) =>
  useReveals.setState(({ queue }) => ({ queue: queue.filter((item) => item.id !== id) }));

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
      <RevealCard key={current.id} shown={current} waiting={waiting} />
    </div>
  );
}

function RevealCard({ shown, waiting }: { shown: Shown; waiting: number }) {
  const { t } = useTranslation("quest");
  const navigate = useNavigate();
  const text = useLocalized();
  const badgeName = useBadgeName();
  const { reveal: item, loud } = shown;
  const full = loud === "full";

  let eyebrow: string;
  let title: string;
  let body: string | undefined;
  let open: (() => void) | undefined;
  let art: React.ReactNode;
  let extra: React.ReactNode;

  switch (item.kind) {
    case "badge": {
      const badge = item.badge;
      eyebrow = t("celebrate.badge.eyebrow");
      title = badgeName(badge);
      body = text(badge.title)
        ? t("celebrate.badge.title", { title: text(badge.title) })
        : text(badge.description);
      open = () => void navigate({ to: "/quest/badges/$key", params: { key: badge.key } });
      art = <BadgeMedal badge={badge} owned size={full ? "xl" : "lg"} />;
      break;
    }
    case "level":
      eyebrow = t("celebrate.level.eyebrow");
      title = t("celebrate.level.title", { level: item.level });
      open = () => void navigate({ to: "/quest" });
      art = (
        <span
          className={cn(
            "grid place-items-center rounded-full border-2 border-ink bg-sun font-black text-[#17122b] shadow-[0_3px_0_0_var(--ink)] tabular-nums",
            full ? "size-36 text-6xl" : "size-24 text-4xl",
          )}
        >
          {item.level}
        </span>
      );
      extra = item.coins > 0 && <Coins amount={item.coins} signed />;
      break;
    case "streak":
      eyebrow = t("celebrate.streak.eyebrow");
      title = t("celebrate.streak.title", { count: item.days });
      body = t("celebrate.streak.body", { name: item.friendName });
      open = () => void navigate({ to: "/quest/streaks" });
      art = (
        <span className={cn("leading-none", full ? "text-9xl" : "text-7xl")} aria-hidden>
          🔥
        </span>
      );
      break;
    case "claim": {
      eyebrow = t("celebrate.claim.eyebrow");
      title = text(item.mission.title);
      const coins = (item.reward?.coins ?? 0) + (item.bonus?.coins ?? 0);
      const xp = (item.reward?.xp ?? 0) + (item.bonus?.xp ?? 0);
      body = item.bonus ? t("celebrate.claim.bonus") : undefined;
      art = (
        <span className={cn("leading-none", full ? "text-9xl" : "text-7xl")} aria-hidden>
          {item.mission.emoji || "🎯"}
        </span>
      );
      extra = (
        <span className="inline-flex items-center gap-3">
          {coins > 0 && <Coins amount={coins} signed />}
          {xp > 0 && <span className="font-black text-brand">{t("xp.plus", { xp })}</span>}
        </span>
      );
      break;
    }
  }

  return (
    <div
      role="status"
      data-testid="quest-reveal"
      className={cn(
        // Only its buttons take taps: what lies under the card stays reachable.
        "sticker-lg relative flex animate-pop-in flex-col items-center rounded-[2rem] bg-surface text-center",
        full
          ? "w-[min(24rem,100%)] gap-3 px-6 pt-9 pb-7"
          : "w-[min(20rem,100%)] gap-2 px-5 pt-6 pb-5",
      )}
    >
      <button
        type="button"
        aria-label={t("celebrate.close")}
        onClick={() => dismiss(shown.id)}
        className="pointer-events-auto absolute top-2.5 right-2.5 grid size-9 place-items-center rounded-full text-ink-soft hover:bg-surface-2"
      >
        <X className="size-5" />
      </button>
      <span className={cn(full && "animate-float")}>{art}</span>
      <p className="mt-2 text-xs font-black tracking-wide text-brand">{eyebrow}</p>
      <p className={cn("font-black leading-tight", full ? "text-3xl" : "text-xl")}>
        <EmojiText text={title} />
      </p>
      {body && (
        <p className="text-sm font-bold text-ink-soft">
          <EmojiText text={body} />
        </p>
      )}
      {extra}
      {open && (
        <button
          type="button"
          onClick={() => {
            dismiss(shown.id);
            open();
          }}
          className="pressable sticker-sm pointer-events-auto mt-2 inline-flex h-9 items-center rounded-full bg-lime px-4 text-sm font-black text-[#17122b]"
        >
          {t("celebrate.open")}
        </button>
      )}
      {waiting > 0 && (
        <p className="text-xs font-bold text-ink-faint">
          {t("celebrate.more", { count: waiting })}
        </p>
      )}
    </div>
  );
}
