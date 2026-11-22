import { Link, useRouterState } from "@tanstack/react-router";
import type { common, quest } from "@tokuly/api";
import { Lock } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Chip } from "#/components/ui/card";
import { Coins } from "#/components/ui/coins";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { useLocalized } from "./api";
import {
  isHiddenSecret,
  type RarityKey,
  type Remaining,
  rarityKey,
  remaining,
  tierKey,
} from "./model";

/** The sections of the quest area, as a row of links under every page's header. */
export function QuestNav({ className }: { className?: string }) {
  const { t } = useTranslation("quest");
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const items = [
    { to: "/quest", label: t("nav.home"), active: pathname === "/quest" || pathname === "/quest/" },
    {
      to: "/quest/badges",
      label: t("nav.badges"),
      active: pathname.startsWith("/quest/badges") || pathname.startsWith("/quest/showcase"),
    },
    {
      to: "/quest/streaks",
      label: t("nav.streaks"),
      active: pathname.startsWith("/quest/streaks"),
    },
    {
      to: "/quest/ranking",
      label: t("nav.ranking"),
      active: pathname.startsWith("/quest/ranking"),
    },
  ] as const;
  return (
    <nav
      aria-label={t("nav.label")}
      className={cn(
        "scrollbar-none mb-4 flex max-w-full gap-1 overflow-x-auto rounded-full bg-surface-2 p-1",
        className,
      )}
    >
      {items.map((item) => (
        <Link
          key={item.to}
          to={item.to}
          aria-current={item.active ? "page" : undefined}
          className={cn(
            "flex h-9 flex-auto shrink-0 items-center justify-center rounded-full px-2 text-sm font-bold whitespace-nowrap transition-colors",
            item.active ? "sticker-sm bg-surface text-ink" : "text-ink-soft hover:text-ink",
          )}
        >
          {item.label}
        </Link>
      ))}
    </nav>
  );
}

/** A filled bar for progress towards something; the fill glides to its new width. */
export function ProgressBar({
  value,
  label,
  valueText,
  tone = "brand",
  className,
}: {
  /** 0–1. */
  value: number;
  /** What the bar measures, for screen readers. */
  label: string;
  /** The progress in words ("30 / 100 XP"). */
  valueText?: string;
  tone?: "brand" | "lime" | "sun";
  className?: string;
}) {
  const percent = Math.round(Math.min(1, Math.max(0, value)) * 100);
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent}
      aria-valuetext={valueText}
      className={cn(
        "h-3.5 overflow-hidden rounded-full border-2 border-ink bg-surface-2",
        className,
      )}
    >
      <div
        className={cn(
          "h-full rounded-full transition-[width] ease-out-soft",
          tone === "brand" ? "bg-brand" : tone === "lime" ? "bg-lime" : "bg-sun",
        )}
        style={{ width: `${percent}%`, transitionDuration: "calc(700ms * var(--dopa))" }}
      />
    </div>
  );
}

const MEDAL_SIZES = {
  sm: { box: "size-10 text-xl", ribbon: "text-[9px] px-1", lock: "" },
  md: { box: "size-16 text-3xl", ribbon: "text-[10px] px-1.5", lock: "size-4" },
  lg: { box: "size-24 text-5xl", ribbon: "text-xs px-2", lock: "size-5" },
  xl: { box: "size-36 text-7xl", ribbon: "text-sm px-3", lock: "size-6" },
} as const;

const TIER_TINT = {
  bronze: "bg-coral",
  silver: "bg-surface-3",
  gold: "bg-sun",
} as const;

/**
 * A badge as a round sticker: its emoji on the colour of its tier, with the
 * tier written on a ribbon. One the viewer has not earned is greyed with a
 * lock; a secret one they have not earned is a question mark.
 */
export function BadgeMedal({
  badge,
  size = "md",
  owned = badge.owned,
  className,
}: {
  badge: quest.Badge;
  size?: keyof typeof MEDAL_SIZES;
  /** Draw it as earned whatever `badge.owned` says (a showcase, a reward). */
  owned?: boolean;
  className?: string;
}) {
  const { t } = useTranslation("quest");
  const sizes = MEDAL_SIZES[size];
  const tier = tierKey(badge.tier);
  const hidden = isHiddenSecret(badge) && !owned;
  return (
    <span className={cn("relative inline-grid shrink-0 place-items-center", className)} aria-hidden>
      <span
        className={cn(
          "grid place-items-center rounded-full border-2 border-ink leading-none select-none",
          sizes.box,
          hidden
            ? "bg-ink font-black text-surface"
            : owned
              ? cn(tier ? TIER_TINT[tier] : "bg-mint", "shadow-[0_3px_0_0_var(--ink)]")
              : "bg-surface-2",
        )}
      >
        <span className={cn(!owned && !hidden && "opacity-35 grayscale")}>
          {hidden ? "?" : badge.emoji || "🏅"}
        </span>
      </span>
      {tier && !hidden && (
        <span
          className={cn(
            "absolute -bottom-1.5 rounded-full border-2 border-ink font-black leading-tight text-[#17122b]",
            sizes.ribbon,
            owned ? TIER_TINT[tier] : "bg-surface-2 text-ink-faint",
          )}
        >
          {t(size === "sm" ? `tiersShort.${tier}` : `tiers.${tier}`)}
        </span>
      )}
      {!owned && !hidden && size !== "sm" && (
        <span className="absolute -top-0.5 -right-0.5 grid place-items-center rounded-full border-2 border-ink bg-surface p-0.5 text-ink-soft">
          <Lock className={sizes.lock} />
        </span>
      )}
    </span>
  );
}

/** A badge's name: its own, or "シークレット" while it is still a secret. */
export function useBadgeName(): (badge: quest.Badge | undefined) => string {
  const { t } = useTranslation("quest");
  const text = useLocalized();
  return (badge) => {
    if (!badge) return "";
    const name = text(badge.name);
    if (!name) return t("badges.secretName");
    const tier = tierKey(badge.tier);
    return tier ? t("badges.nameWithTier", { name, tier: t(`tiers.${tier}`) }) : name;
  };
}

const RARITY_TINT: Record<RarityKey, string> = {
  common: "bg-surface-2 text-ink-soft",
  uncommon: "bg-mint text-[#17122b]",
  rare: "bg-sky text-[#17122b]",
  epic: "bg-pink text-white",
  legendary: "bg-sun text-[#17122b]",
};

/** How rare a badge is, in words; nothing while the server does not say. */
export function RarityChip({
  rarity,
  className,
}: {
  rarity: quest.BadgeRarity;
  className?: string;
}) {
  const { t } = useTranslation("quest");
  const key = rarityKey(rarity);
  if (!key) return null;
  return <Chip className={cn(RARITY_TINT[key], className)}>{t(`rarity.${key}`)}</Chip>;
}

/** What claiming something gives: coins, XP and sometimes a badge. */
export function RewardLine({
  reward,
  className,
}: {
  reward: quest.Reward | undefined;
  className?: string;
}) {
  const { t } = useTranslation("quest");
  const name = useBadgeName();
  if (!reward) return null;
  return (
    <span className={cn("inline-flex flex-wrap items-center gap-x-2.5 gap-y-1", className)}>
      {reward.coins > 0 && <Coins amount={reward.coins} size="sm" signed />}
      {reward.xp > 0 && (
        <span className="text-sm font-black text-brand tabular-nums">
          {t("xp.plus", { xp: reward.xp })}
        </span>
      )}
      {reward.badge && (
        <span className="inline-flex items-center gap-1 text-sm font-black">
          <span aria-hidden>{reward.badge.emoji || "🏅"}</span>
          {name(reward.badge)}
        </span>
      )}
    </span>
  );
}

/** "あと5時間" until a moment. */
export function useRemainingText(): (untilMs: number, nowMs: number) => string {
  const { t } = useTranslation("quest");
  return (untilMs, nowMs) => remainingText(remaining(untilMs, nowMs), t);
}

function remainingText(
  left: Remaining,
  t: (key: string, options: { count: number }) => string,
): string {
  return t(`remaining.${left.unit}`, { count: left.count });
}

/** A level as a small filled pill: "Lv.12". */
export function LevelPill({ level, className }: { level: number; className?: string }) {
  const { t } = useTranslation("quest");
  return (
    <span
      className={cn(
        "inline-flex h-5 shrink-0 items-center rounded-full border-2 border-ink bg-sun px-1.5 text-[11px] leading-none font-black text-[#17122b] tabular-nums",
        className,
      )}
      role="img"
      aria-label={t("level.label", { level })}
    >
      {t("level.short", { level })}
    </span>
  );
}

/** A title (称号) as a small outlined label. */
export function TitlePill({ title, className }: { title: string; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex h-5 max-w-full shrink-0 items-center rounded-full border-2 border-ink bg-brand-soft px-1.5 text-[11px] leading-none font-black text-ink",
        className,
      )}
    >
      <span className="truncate">{title}</span>
    </span>
  );
}

/** A person as a row that opens their quest page: avatar, name, something on the right. */
export function PersonRow({
  user,
  subtitle,
  trailing,
  leading,
  highlighted,
  label,
}: {
  user: common.UserSummary | undefined;
  subtitle?: ReactNode;
  trailing?: ReactNode;
  leading?: ReactNode;
  highlighted?: boolean;
  /** The link's name for screen readers. */
  label?: string;
}) {
  if (!user) return null;
  const body = (
    <>
      {leading}
      <Avatar user={user} size="md" />
      <span className="min-w-0 flex-1 text-left">
        <span className="block truncate font-bold">
          <EmojiText text={user.displayName} />
        </span>
        {subtitle && (
          <span className="mt-0.5 flex min-w-0 items-center gap-1.5 text-xs font-bold text-ink-faint">
            {subtitle}
          </span>
        )}
      </span>
      {trailing}
    </>
  );
  const classes = cn(
    "flex w-full items-center gap-3 rounded-2xl px-3 py-2.5",
    highlighted ? "bg-brand-soft" : "transition-colors hover:bg-surface-2",
  );
  return user.deleted ? (
    <div className={classes}>{body}</div>
  ) : (
    <Link
      to="/quest/u/$handle"
      params={{ handle: user.handle }}
      className={classes}
      aria-label={label}
    >
      {body}
    </Link>
  );
}

/** The heading of a block inside a quest page, with an optional link or note on the right. */
export function SectionTitle({ children, aside }: { children: ReactNode; aside?: ReactNode }) {
  return (
    <div className="flex items-end justify-between gap-3">
      <h2 className="text-lg font-black">{children}</h2>
      {aside && <div className="shrink-0 text-sm font-bold text-ink-soft">{aside}</div>}
    </div>
  );
}
