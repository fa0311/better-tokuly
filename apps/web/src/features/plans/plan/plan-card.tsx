import { Link } from "@tanstack/react-router";
import { plans } from "@tokuly/api";
import { CheckSquare, MapPin, MessageCircle, Receipt } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Chip } from "#/components/ui/card";
import { cn } from "#/lib/cn";
import { AvatarStack } from "../parts";
import { planTimeText, untilText } from "./model";

const RSVP_TONE: Record<number, string> = {
  [plans.Rsvp.GOING]: "bg-lime text-[#17122b]",
  [plans.Rsvp.MAYBE]: "bg-sun text-[#17122b]",
  [plans.Rsvp.DECLINED]: "bg-surface-3 text-ink-soft",
  [plans.Rsvp.INVITED]: "bg-pink text-white",
  [plans.Rsvp.WAITLISTED]: "bg-sky text-[#17122b]",
};

/** The key of the short label for an answer ("行く", "たぶん", ...). */
export function rsvpKey(rsvp: plans.Rsvp): "going" | "maybe" | "declined" | "invited" | "waitlisted" {
  switch (rsvp) {
    case plans.Rsvp.GOING:
      return "going";
    case plans.Rsvp.MAYBE:
      return "maybe";
    case plans.Rsvp.DECLINED:
      return "declined";
    case plans.Rsvp.WAITLISTED:
      return "waitlisted";
    default:
      return "invited";
  }
}

export function RsvpChip({ rsvp, className }: { rsvp: plans.Rsvp; className?: string }) {
  const { t } = useTranslation("plans");
  return <Chip className={cn(RSVP_TONE[rsvp], className)}>{t(`rsvp.short.${rsvpKey(rsvp)}`)}</Chip>;
}

/** A plan in a list: when, what, where, who, and the caller's own answer. */
export function PlanCard({ plan }: { plan: plans.Plan }) {
  const { t, i18n } = useTranslation("plans");
  const cancelled = plan.status === plans.PlanStatus.CANCELLED;
  const until = !cancelled && !plan.over ? untilText(plan.time, t) : undefined;
  const people = plan.participants.flatMap((participant) => (participant.user ? [participant.user] : []));
  return (
    <Link to="/plans/$planId" params={{ planId: plan.id }} className={cn("pressable sticker flex gap-3 rounded-card bg-surface p-4", (cancelled || plan.over) && "opacity-75")}>
      <span className="grid size-14 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-brand-soft text-3xl" aria-hidden>
        {plan.emoji || "📅"}
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-2">
          <span className={cn("min-w-0 flex-1 truncate text-lg font-black", cancelled && "line-through")}>{plan.title}</span>
          {cancelled ? <Chip className="bg-danger text-white">{t("plan.cancelled")}</Chip> : <RsvpChip rsvp={plan.myRsvp} />}
        </span>
        <span className="mt-0.5 flex flex-wrap items-center gap-x-2 text-sm font-bold text-ink-soft">
          <span>{planTimeText(plan.time, i18n.language, t)}</span>
          {until && <span className="rounded-full bg-brand-soft px-2 text-xs font-black text-brand-strong">{until}</span>}
        </span>
        {plan.place?.name && (
          <span className="mt-0.5 flex items-center gap-1 truncate text-sm text-ink-soft">
            <MapPin className="size-3.5 shrink-0" />
            <span className="truncate">{plan.place.name}</span>
          </span>
        )}
        <span className="mt-2 flex items-center gap-3 text-xs font-bold text-ink-soft">
          <AvatarStack users={people} total={plan.counts?.going ?? people.length} />
          <span>{t("plan.goingCount", { count: plan.counts?.going ?? 0 })}</span>
          {plan.checklistCount > 0 && (
            <span className="flex items-center gap-0.5">
              <CheckSquare className="size-3.5" />
              {plan.checklistDoneCount}/{plan.checklistCount}
            </span>
          )}
          {plan.expenseCount > 0 && (
            <span className="flex items-center gap-0.5">
              <Receipt className="size-3.5" />
              {plan.expenseCount}
            </span>
          )}
          {plan.commentCount > 0 && (
            <span className="flex items-center gap-0.5">
              <MessageCircle className="size-3.5" />
              {plan.commentCount}
            </span>
          )}
        </span>
      </span>
    </Link>
  );
}
