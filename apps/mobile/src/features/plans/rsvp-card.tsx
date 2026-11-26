import { useMutation, useQueryClient } from "@tanstack/react-query";
import { plans } from "@tokuly/api";
import { planKeys, rememberPlan, showMyRsvp } from "@tokuly/core/plans";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { Button } from "#/components/button";
import { TextField } from "#/components/text-field";
import { useMe } from "#/features/session/store";
import { INK_ON_BRIGHT, sticker, useTheme } from "#/lib/theme";
import { asViewer, showPlanError } from "./actions";

const CHOICES = [
  { rsvp: plans.Rsvp.GOING, key: "going", emoji: "🙌", box: "bg-lime" },
  { rsvp: plans.Rsvp.MAYBE, key: "maybe", emoji: "🤔", box: "bg-sun" },
  { rsvp: plans.Rsvp.DECLINED, key: "declined", emoji: "🙏", box: "bg-surface-3" },
] as const;

/**
 * The viewer's answer: going, maybe or no in one tap (shown at once,
 * confirmed by the server), and a few words to go with it.
 */
export function RsvpCard({ plan }: { plan: plans.Plan }) {
  const { t } = useTranslation("plans");
  const { colors } = useTheme();
  const me = useMe();
  const queryClient = useQueryClient();
  const mine = plan.participants.find((participant) => participant.user?.id === me?.user?.id);
  const [comment, setComment] = useState<string | null>(null);
  const text = comment ?? mine?.comment ?? "";
  const isHost = plan.myRole === plans.PlanRole.HOST;

  const respond = useMutation({
    mutationFn: (rsvp: plans.Rsvp) =>
      asViewer((service) => service.respondToPlan({ planId: plan.id, rsvp, comment: text.trim() })),
    onMutate: async (rsvp) => {
      // A read on its way would put the old answer back over this one.
      await queryClient.cancelQueries({ queryKey: planKeys.plan(plan.id) });
      return { before: showMyRsvp(queryClient, plan.id, rsvp) };
    },
    onSuccess: (response, asked) => {
      if (!response) return;
      rememberPlan(queryClient, response.plan);
      setComment(null);
      if (asked === plans.Rsvp.GOING && response.rsvp === plans.Rsvp.WAITLISTED)
        Alert.alert(t("rsvp.waitlistedToast"));
    },
    onError: (error, _rsvp, context) => {
      // Only the answer goes back: whatever else arrived meanwhile stays.
      if (context?.before !== undefined) showMyRsvp(queryClient, plan.id, context.before);
      void queryClient.invalidateQueries({ queryKey: planKeys.plan(plan.id) });
      showPlanError(error);
    },
  });

  if (plan.status !== plans.PlanStatus.ACTIVE || plan.over) return null;
  const current = plan.myRsvp === plans.Rsvp.WAITLISTED ? plans.Rsvp.GOING : plan.myRsvp;
  const counts: Record<number, number> = {
    [plans.Rsvp.GOING]: plan.counts?.going ?? 0,
    [plans.Rsvp.MAYBE]: plan.counts?.maybe ?? 0,
    [plans.Rsvp.DECLINED]: plan.counts?.declined ?? 0,
  };
  const answered = plan.myRsvp !== plans.Rsvp.INVITED && plan.myRsvp !== plans.Rsvp.UNSPECIFIED;
  const commentChanged = text.trim() !== (mine?.comment ?? "");

  return (
    <View
      testID="plan-rsvp"
      className={`gap-3 rounded-card p-4 ${answered ? "bg-surface" : "bg-brand-soft"}`}
      style={sticker(colors)}
    >
      <Text className="text-lg font-black text-ink">
        {answered ? t("rsvp.title") : t("rsvp.ask")}
      </Text>
      {plan.myRsvp === plans.Rsvp.WAITLISTED && (
        <Text className="-mt-2 text-sm font-bold text-ink-soft">
          {t("rsvp.waitlisted", { position: mine?.waitlistPosition ?? 1 })}
        </Text>
      )}
      {plan.capacity > 0 && (
        <Text className="-mt-2 text-sm font-bold text-ink-soft">
          {t("rsvp.capacity", { going: plan.counts?.going ?? 0, capacity: plan.capacity })}
        </Text>
      )}
      <View accessibilityRole="radiogroup" className="flex-row gap-2">
        {CHOICES.map((choice) => {
          const active = current === choice.rsvp;
          const disabled = respond.isPending || (isHost && choice.rsvp !== plans.Rsvp.GOING);
          return (
            <Pressable
              key={choice.key}
              testID={`plan-rsvp-${choice.key}`}
              accessibilityRole="radio"
              accessibilityState={{ selected: active, disabled }}
              disabled={disabled}
              onPress={() => respond.mutate(choice.rsvp)}
              className={`min-h-20 flex-1 items-center justify-center gap-0.5 rounded-2xl px-1 py-2 ${active ? choice.box : "bg-surface-2"} ${disabled && !active ? "opacity-50" : ""}`}
              style={active ? sticker(colors, "sm") : undefined}
            >
              <Text className="text-2xl leading-8">{choice.emoji}</Text>
              <Text
                className="text-sm font-black"
                style={{
                  color: active && choice.box !== "bg-surface-3" ? INK_ON_BRIGHT : colors.ink,
                }}
              >
                {t(`rsvp.${choice.key}`)}
              </Text>
              <Text className="text-xs text-ink-soft">{counts[choice.rsvp] ?? 0}</Text>
            </Pressable>
          );
        })}
      </View>
      {isHost && <Text className="text-xs font-bold text-ink-soft">{t("rsvp.hostHint")}</Text>}
      {answered && (
        <View className="gap-2">
          <TextField
            testID="plan-rsvp-comment"
            label={t("rsvp.comment")}
            placeholder={t("rsvp.commentPlaceholder")}
            value={text}
            maxLength={140}
            onChangeText={setComment}
          />
          {commentChanged && (
            <Button
              testID="plan-rsvp-save-comment"
              variant="secondary"
              size="sm"
              loading={respond.isPending}
              onPress={() => respond.mutate(current)}
            >
              {t("rsvp.saveComment")}
            </Button>
          )}
        </View>
      )}
    </View>
  );
}
