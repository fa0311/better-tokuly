import { Link } from "@tanstack/react-router";
import { plans } from "@tokuly/api";
import { Pin, Repeat, Users } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { cn } from "#/lib/cn";
import { useMe } from "#/stores/session";
import { nameOf } from "../api";
import { formatDay } from "../dates";
import { anniversaryEmoji, anniversaryTitle, countdownParts, inDaysText, localized, tintOf } from "./model";

/** The big number of a countdown or a counter. */
export function BigCount({ lead, number, unit, className }: { lead?: string; number: string; unit?: string; className?: string }) {
  return (
    <p className={cn("flex items-baseline gap-1 font-black", className)}>
      {lead && <span className="text-sm">{lead}</span>}
      <span className="text-4xl leading-none tabular-nums">{number}</span>
      {unit && <span className="text-sm">{unit}</span>}
    </p>
  );
}

/** A countdown as a sticker: the emoji, the name, and how many days are left. */
export function CountdownCard({ countdown }: { countdown: plans.Countdown }) {
  const { t, i18n } = useTranslation("plans");
  const me = useMe();
  const mine = countdown.owner?.id === me?.user?.id;
  const passed = countdown.daysLeft < 0;
  return (
    <Link
      to="/plans/countdowns/$countdownId"
      params={{ countdownId: countdown.id }}
      className={cn("pressable sticker flex min-h-36 flex-col rounded-card p-4", passed ? "bg-surface-2 text-ink-soft" : tintOf(countdown.id), countdown.daysLeft === 0 && "animate-wiggle")}
      data-testid="countdown-card"
    >
      <span className="flex items-start justify-between gap-2">
        <span className="text-3xl" aria-hidden>
          {countdown.emoji || "⏳"}
        </span>
        <span className="flex items-center gap-1.5 opacity-80">
          {countdown.repeatsYearly && <Repeat className="size-4" aria-label={t("countdown.yearly")} />}
          {mine && countdown.sharing === plans.CountdownSharing.PROFILE && <Pin className="size-4" aria-label={t("countdown.pinned")} />}
          {mine && countdown.sharing === plans.CountdownSharing.CHOSEN && <Users className="size-4" aria-label={t("countdown.shared")} />}
          {!mine && <Avatar user={countdown.owner} size="xs" />}
        </span>
      </span>
      <span className="mt-1 line-clamp-2 font-black break-words">{countdown.title}</span>
      <span className="mt-auto pt-2">
        <BigCount {...countdownParts(countdown.daysLeft, t)} />
        <span className="text-xs font-bold opacity-75">
          {formatDay(countdown.nextDate || countdown.date, i18n.language)}
        </span>
      </span>
    </Link>
  );
}

/** A "days since" counter of two people, or a couple invitation still waiting. */
export function AnniversaryCard({ anniversary }: { anniversary: plans.Anniversary }) {
  const { t, i18n } = useTranslation("plans");
  const pending = anniversary.status === plans.AnniversaryStatus.PENDING;
  const next = anniversary.upcoming[0];
  return (
    <Link
      to="/plans/anniversaries/$anniversaryId"
      params={{ anniversaryId: anniversary.id }}
      className={cn("pressable sticker flex items-center gap-3 rounded-card p-4", pending ? "bg-surface" : anniversary.kind === plans.AnniversaryKind.COUPLE ? "bg-pink text-white" : "bg-sky text-[#17122b]")}
      data-testid="anniversary-card"
    >
      <span className="relative shrink-0">
        <Avatar user={anniversary.partner} size="lg" outlined />
        <span className="absolute -right-1.5 -bottom-1.5 text-2xl" aria-hidden>
          {anniversaryEmoji(anniversary)}
        </span>
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate font-black">{t("anniv.with", { name: nameOf(anniversary.partner) })}</span>
        <span className="block truncate text-sm font-bold opacity-80">{anniversaryTitle(anniversary, t)}</span>
        {pending ? (
          <span className="mt-1 block text-sm font-black">{anniversary.invitedByMe ? t("anniv.waiting") : t("anniv.invitedYou")}</span>
        ) : (
          next && <span className="mt-1 block truncate text-xs font-bold opacity-80">{next.daysLeft === 0 ? localized(next.label, i18n.language) : `${localized(next.label, i18n.language)} · ${inDaysText(next.daysLeft, t)}`}</span>
        )}
      </span>
      {!pending && <BigCount number={String(anniversary.days)} unit={anniversary.countStartDay ? t("anniv.dayNth") : t("anniv.daysUnit")} className="shrink-0" />}
    </Link>
  );
}
