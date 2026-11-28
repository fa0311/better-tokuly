import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { plans } from "@tokuly/api";
import { canInvite, rsvpKey } from "@tokuly/core/plans";
import { Crown, MoreHorizontal, ShieldCheck, ShieldMinus, UserMinus, UserPlus } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { EmojiText } from "#/features/emoji";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { useMe } from "#/stores/session";
import { nameOf, planClient, rememberPlan } from "../api";
import { FriendPicker } from "../friend-picker";
import { SectionTitle } from "../parts";

const ORDER = [
  plans.Rsvp.GOING,
  plans.Rsvp.MAYBE,
  plans.Rsvp.WAITLISTED,
  plans.Rsvp.INVITED,
  plans.Rsvp.DECLINED,
];

/** Everyone in the plan by answer, with what a host can do about each of them. */
export function Participants({ plan }: { plan: plans.Plan }) {
  const { t } = useTranslation("plans");
  const me = useMe();
  const queryClient = useQueryClient();
  const [inviting, setInviting] = useState(false);
  const [confirming, setConfirming] = useState<{
    kind: "remove" | "handOver";
    participant: plans.PlanParticipant;
  }>();
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: "plans" }));

  const invite = useMutation({
    mutationFn: (userIds: string[]) => planClient().inviteToPlan({ planId: plan.id, userIds }),
    onSuccess: (response) => {
      rememberPlan(queryClient, response.plan);
      setInviting(false);
      // People who left by their own choice are skipped without a word.
      toast.success(
        response.invitedCount > 0
          ? t("people.invited", { count: response.invitedCount })
          : t("people.invitedNone"),
      );
    },
    onError,
  });
  const setRole = useMutation({
    mutationFn: ({ userId, role }: { userId: string; role: plans.PlanRole }) =>
      planClient().setPlanRole({ planId: plan.id, userId, role }),
    onSuccess: (response) => {
      rememberPlan(queryClient, response.plan);
      setConfirming(undefined);
    },
    onError,
  });
  const remove = useMutation({
    mutationFn: (userId: string) => planClient().removePlanParticipant({ planId: plan.id, userId }),
    onSuccess: (response) => {
      rememberPlan(queryClient, response.plan);
      setConfirming(undefined);
      toast(t("people.removed"));
    },
    onError,
  });

  const active = plan.status === plans.PlanStatus.ACTIVE;
  const iAmHost = plan.myRole === plans.PlanRole.HOST;
  const iAmCoHost = plan.myRole === plans.PlanRole.CO_HOST;
  const actionsFor = (participant: plans.PlanParticipant): MenuAction[] => {
    const userId = participant.user?.id ?? "";
    if (!active || userId === me?.user?.id || participant.role === plans.PlanRole.HOST) return [];
    const actions: MenuAction[] = [];
    if (iAmHost) {
      actions.push(
        participant.role === plans.PlanRole.CO_HOST
          ? {
              label: t("people.makeGuest"),
              icon: <ShieldMinus />,
              onSelect: () => setRole.mutate({ userId, role: plans.PlanRole.GUEST }),
            }
          : {
              label: t("people.makeCoHost"),
              icon: <ShieldCheck />,
              onSelect: () => setRole.mutate({ userId, role: plans.PlanRole.CO_HOST }),
            },
        {
          label: t("people.handOver"),
          icon: <Crown />,
          onSelect: () => setConfirming({ kind: "handOver", participant }),
        },
      );
    }
    // Only the host takes a co-host out.
    if (iAmHost || (iAmCoHost && participant.role === plans.PlanRole.GUEST)) {
      actions.push({
        label: t("people.remove"),
        icon: <UserMinus />,
        danger: true,
        onSelect: () => setConfirming({ kind: "remove", participant }),
      });
    }
    return actions;
  };

  const groups = ORDER.map((rsvp) => ({
    rsvp,
    people: plan.participants.filter((participant) => participant.rsvp === rsvp),
  })).filter((group) => group.people.length > 0);
  // The waitlist in the order places will free up.
  for (const group of groups)
    if (group.rsvp === plans.Rsvp.WAITLISTED)
      group.people.sort((a, b) => a.waitlistPosition - b.waitlistPosition);

  return (
    <section>
      <SectionTitle
        action={
          canInvite(plan) && (
            <Button
              variant="secondary"
              size="sm"
              icon={<UserPlus className="size-4" />}
              onClick={() => setInviting(true)}
            >
              {t("people.invite")}
            </Button>
          )
        }
      >
        {t("people.title", { count: plan.participants.length })}
      </SectionTitle>
      <div className="flex flex-col gap-3 rounded-card bg-surface p-3">
        {groups.map((group) => (
          <div key={group.rsvp}>
            <p className="px-1 pb-1 text-xs font-black text-ink-faint">
              {t(`rsvp.group.${rsvpKey(group.rsvp)}`)} · {group.people.length}
            </p>
            <ul>
              {group.people.map((participant) => {
                const actions = actionsFor(participant);
                return (
                  <li
                    key={participant.user?.id}
                    className="flex items-center gap-3 rounded-2xl px-1 py-1.5"
                  >
                    <Link
                      to="/u/$handle"
                      params={{ handle: participant.user?.handle ?? "" }}
                      className="shrink-0"
                    >
                      <Avatar user={participant.user} size="sm" />
                    </Link>
                    <div className="min-w-0 flex-1">
                      <p className="flex items-center gap-1.5">
                        <span className="truncate font-bold">
                          <EmojiText text={nameOf(participant.user)} />
                        </span>
                        {participant.role === plans.PlanRole.HOST && (
                          <Chip className="h-5 shrink-0 bg-sun px-2 text-[11px] text-[#17122b]">
                            {t("people.host")}
                          </Chip>
                        )}
                        {participant.role === plans.PlanRole.CO_HOST && (
                          <Chip className="h-5 shrink-0 px-2 text-[11px]">
                            {t("people.coHost")}
                          </Chip>
                        )}
                        {participant.rsvp === plans.Rsvp.WAITLISTED && (
                          <Chip className="h-5 shrink-0 px-2 text-[11px]">
                            #{participant.waitlistPosition}
                          </Chip>
                        )}
                      </p>
                      {participant.comment && (
                        <p className="text-sm break-words text-ink-soft">
                          「{participant.comment}」
                        </p>
                      )}
                    </div>
                    {actions.length > 0 && (
                      <Menu
                        actions={actions}
                        trigger={
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            aria-label={t("people.manage", { name: nameOf(participant.user) })}
                          >
                            <MoreHorizontal className="size-5" />
                          </Button>
                        }
                      />
                    )}
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
      </div>

      <FriendPicker
        open={inviting}
        onOpenChange={setInviting}
        title={t("people.inviteTitle")}
        description={t("people.inviteLead")}
        exclude={plan.participants.map((participant) => participant.user?.id ?? "")}
        confirmLabel={t("people.inviteConfirm")}
        loading={invite.isPending}
        onConfirm={(users) =>
          users.length ? invite.mutate(users.map((user) => user.id)) : setInviting(false)
        }
      />
      <ConfirmDialog
        open={Boolean(confirming)}
        onOpenChange={(open) => !open && setConfirming(undefined)}
        title={
          confirming?.kind === "handOver"
            ? t("people.handOverTitle", { name: nameOf(confirming.participant.user) })
            : t("people.removeTitle", { name: nameOf(confirming?.participant.user) })
        }
        description={
          confirming?.kind === "handOver" ? t("people.handOverBody") : t("people.removeBody")
        }
        confirmLabel={confirming?.kind === "handOver" ? t("people.handOver") : t("people.remove")}
        danger={confirming?.kind === "remove"}
        loading={setRole.isPending || remove.isPending}
        onConfirm={() => {
          const userId = confirming?.participant.user?.id ?? "";
          if (confirming?.kind === "handOver")
            setRole.mutate({ userId, role: plans.PlanRole.HOST });
          else remove.mutate(userId);
        }}
      />
    </section>
  );
}
