import { useNavigate } from "@tanstack/react-router";
import { X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Coins } from "#/components/ui/coins";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { useLocalized } from "./api";
import { dismiss, type Shown } from "./celebrate";
import { BadgeMedal, useBadgeName } from "./parts";

/** The card of a reveal on screen: what it is about, a way to open it, and how many wait. */
export function RevealCard({ shown, waiting }: { shown: Shown; waiting: number }) {
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
