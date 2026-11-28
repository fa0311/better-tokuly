import { useMutation, useQueryClient } from "@tanstack/react-query";
import { plans } from "@tokuly/api";
import { type ReactNode, useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Card } from "#/components/ui/card";
import { TextField } from "#/components/ui/field";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { useMe } from "#/stores/session";
import { keys, planClient, rememberPlan } from "../api";

const CHOICES = [
  { rsvp: plans.Rsvp.GOING, key: "going", emoji: "🙌", tone: "bg-lime text-[#17122b]" },
  { rsvp: plans.Rsvp.MAYBE, key: "maybe", emoji: "🤔", tone: "bg-sun text-[#17122b]" },
  { rsvp: plans.Rsvp.DECLINED, key: "declined", emoji: "🙏", tone: "bg-surface-3 text-ink" },
] as const;

/**
 * The caller's answer to a plan: going, maybe or no in one tap (shown at
 * once, confirmed by the server), and a few words to go with it.
 */
export function RsvpCard({ plan }: { plan: plans.Plan }) {
  const { t } = useTranslation("plans");
  const me = useMe();
  const queryClient = useQueryClient();
  const mine = plan.participants.find((participant) => participant.user?.id === me?.user?.id);
  const [comment, setComment] = useState<string | null>(null);
  const text = comment ?? mine?.comment ?? "";
  const isHost = plan.myRole === plans.PlanRole.HOST;
  const closed = plan.status !== plans.PlanStatus.ACTIVE || plan.over;

  const respond = useMutation({
    mutationFn: (rsvp: plans.Rsvp) =>
      planClient().respondToPlan({ planId: plan.id, rsvp, comment: text.trim() }),
    onMutate: async (rsvp) => {
      // A read that is on its way would put the old answer back over this one.
      await queryClient.cancelQueries({ queryKey: keys.plan(plan.id) });
      const before = queryClient.getQueryData<plans.Plan | null>(keys.plan(plan.id))?.myRsvp;
      queryClient.setQueryData<plans.Plan | null>(
        keys.plan(plan.id),
        (current) => current && { ...current, myRsvp: rsvp },
      );
      return { before };
    },
    onSuccess: (response, asked) => {
      rememberPlan(queryClient, response.plan);
      void queryClient.invalidateQueries({ queryKey: keys.split(plan.id) });
      setComment(null);
      if (asked === plans.Rsvp.GOING && response.rsvp === plans.Rsvp.WAITLISTED)
        toast(t("rsvp.waitlistedToast"));
      else
        toast.success(
          t(
            `rsvp.toast.${asked === plans.Rsvp.GOING ? "going" : asked === plans.Rsvp.MAYBE ? "maybe" : "declined"}`,
          ),
        );
    },
    onError: (error, _rsvp, context) => {
      // Only the answer goes back: whatever else arrived meanwhile (a new item, someone else's answer) stays.
      const before = context?.before;
      if (before !== undefined)
        queryClient.setQueryData<plans.Plan | null>(
          keys.plan(plan.id),
          (current) => current && { ...current, myRsvp: before },
        );
      void queryClient.invalidateQueries({ queryKey: keys.plan(plan.id) });
      toast.error(errorMessage(error, { namespace: "plans" }));
    },
  });

  if (closed) return null;
  const current = plan.myRsvp === plans.Rsvp.WAITLISTED ? plans.Rsvp.GOING : plan.myRsvp;
  const counts: Record<number, number> = {
    [plans.Rsvp.GOING]: plan.counts?.going ?? 0,
    [plans.Rsvp.MAYBE]: plan.counts?.maybe ?? 0,
    [plans.Rsvp.DECLINED]: plan.counts?.declined ?? 0,
  };
  const commentChanged = text.trim() !== (mine?.comment ?? "");
  const answered = plan.myRsvp !== plans.Rsvp.INVITED && plan.myRsvp !== plans.Rsvp.UNSPECIFIED;

  return (
    <Card className={cn(!answered && "bg-brand-soft")}>
      <p className="text-lg font-black">{answered ? t("rsvp.title") : t("rsvp.ask")}</p>
      {plan.myRsvp === plans.Rsvp.WAITLISTED && (
        <p className="mt-1 text-sm font-bold text-ink-soft">
          {t("rsvp.waitlisted", { position: mine?.waitlistPosition ?? 1 })}
        </p>
      )}
      {plan.capacity > 0 && (
        <p className="mt-1 text-sm font-bold text-ink-soft">
          {t("rsvp.capacity", { going: plan.counts?.going ?? 0, capacity: plan.capacity })}
        </p>
      )}
      <div role="radiogroup" aria-label={t("rsvp.title")} className="mt-3 grid grid-cols-3 gap-2">
        {CHOICES.map((choice) => (
          <RsvpButton
            key={choice.key}
            active={current === choice.rsvp}
            tone={choice.tone}
            emoji={choice.emoji}
            count={counts[choice.rsvp] ?? 0}
            disabled={respond.isPending || (isHost && choice.rsvp !== plans.Rsvp.GOING)}
            onClick={() => respond.mutate(choice.rsvp)}
          >
            {t(`rsvp.${choice.key}`)}
          </RsvpButton>
        ))}
      </div>
      {isHost && <p className="mt-2 text-xs font-bold text-ink-soft">{t("rsvp.hostHint")}</p>}
      {answered && (
        <form
          className="mt-3 flex items-end gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            if (commentChanged)
              respond.mutate(
                plan.myRsvp === plans.Rsvp.WAITLISTED ? plans.Rsvp.GOING : plan.myRsvp,
              );
          }}
        >
          <TextField
            className="min-w-0 flex-1"
            aria-label={t("rsvp.comment")}
            placeholder={t("rsvp.commentPlaceholder")}
            value={text}
            maxLength={140}
            onChange={(event) => setComment(event.target.value)}
          />
          {commentChanged && (
            <Button
              type="submit"
              variant="secondary"
              className="h-12 shrink-0"
              loading={respond.isPending}
            >
              {t("rsvp.saveComment")}
            </Button>
          )}
        </form>
      )}
    </Card>
  );
}

function RsvpButton({
  active,
  tone,
  emoji,
  count,
  disabled,
  onClick,
  children,
}: {
  active: boolean;
  tone: string;
  emoji: string;
  count: number;
  disabled: boolean;
  onClick(): void;
  children: ReactNode;
}) {
  return (
    // biome-ignore lint/a11y/useSemanticElements: three big buttons, not a form field.
    <button
      type="button"
      role="radio"
      aria-checked={active}
      disabled={disabled}
      onClick={onClick}
      className={cn(
        "pressable flex min-h-20 flex-col items-center justify-center gap-0.5 rounded-2xl border-[2.5px] px-1 py-2 text-center font-black disabled:opacity-50",
        active ? cn("border-ink shadow-pop", tone) : "border-line bg-surface text-ink-soft",
      )}
    >
      <span className="text-2xl" aria-hidden>
        {emoji}
      </span>
      <span className="text-sm leading-tight">{children}</span>
      <span className="text-xs opacity-70">{count}</span>
    </button>
  );
}
