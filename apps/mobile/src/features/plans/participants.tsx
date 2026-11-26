import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { common } from "@tokuly/api";
import { plans } from "@tokuly/api";
import { canInvite, nameOf, rememberPlan, rsvpKey } from "@tokuly/core/plans";
import { router } from "expo-router";
import { UserPlus } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { sticker, useTheme } from "#/lib/theme";
import { asViewer, showPlanError } from "./actions";
import { FriendPicker } from "./friend-picker";

/** Who is in the plan, by answer: going first, then maybe, waiting, not answered, declined. */
const ORDER = [
  plans.Rsvp.GOING,
  plans.Rsvp.MAYBE,
  plans.Rsvp.WAITLISTED,
  plans.Rsvp.INVITED,
  plans.Rsvp.DECLINED,
];

/** The people of a plan grouped by their answer, with their few words; inviting more friends. */
export function Participants({ plan }: { plan: plans.Plan }) {
  const { t } = useTranslation("plans");
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const [picking, setPicking] = useState(false);

  const invite = useMutation({
    mutationFn: (people: common.UserSummary[]) =>
      asViewer((service) =>
        service.inviteToPlan({ planId: plan.id, userIds: people.map((user) => user.id) }),
      ),
    onSuccess: (response) => {
      if (!response) return;
      rememberPlan(queryClient, response.plan);
      setPicking(false);
      Alert.alert(
        response.invitedCount > 0
          ? t("people.invited", { count: response.invitedCount })
          : t("people.invitedNone"),
      );
    },
    onError: showPlanError,
  });

  const groups = ORDER.map((rsvp) => ({
    rsvp,
    people: plan.participants
      .filter((participant) => participant.rsvp === rsvp)
      .sort((a, b) => a.waitlistPosition - b.waitlistPosition),
  })).filter((group) => group.people.length > 0);

  return (
    <View
      testID="plan-participants"
      className="gap-3 rounded-card bg-surface p-4"
      style={sticker(colors)}
    >
      <View className="flex-row items-center">
        <Text className="flex-1 text-lg font-black text-ink">
          {t("people.title", { count: plan.participants.length })}
        </Text>
        {canInvite(plan) && !plan.over && (
          <Button
            testID="plan-invite"
            variant="secondary"
            size="sm"
            icon={<UserPlus size={16} color={colors.ink} />}
            onPress={() => setPicking(true)}
          >
            {t("people.invite")}
          </Button>
        )}
      </View>
      {groups.map((group) => (
        <View key={group.rsvp} className="gap-2">
          <Text className="text-xs font-black text-ink-soft">
            {t(`rsvp.group.${rsvpKey(group.rsvp)}`)} · {group.people.length}
          </Text>
          {group.people.map((participant) => (
            <Pressable
              key={participant.user?.id}
              accessibilityRole="button"
              onPress={() => participant.user && router.push(`/u/${participant.user.handle}`)}
              className="flex-row items-center gap-3"
            >
              <Avatar user={participant.user} size="sm" />
              <View className="flex-1">
                <View className="flex-row items-center gap-1.5">
                  <Text numberOfLines={1} className="shrink font-black text-ink">
                    {nameOf(participant.user)}
                  </Text>
                  {participant.role === plans.PlanRole.HOST && (
                    <Text className="text-xs font-black text-brand">{t("people.host")}</Text>
                  )}
                  {participant.role === plans.PlanRole.CO_HOST && (
                    <Text className="text-xs font-black text-brand">{t("people.coHost")}</Text>
                  )}
                </View>
                {participant.comment ? (
                  <Text className="text-sm text-ink-soft">「{participant.comment}」</Text>
                ) : null}
              </View>
            </Pressable>
          ))}
        </View>
      ))}
      <FriendPicker
        visible={picking}
        title={t("people.inviteTitle")}
        lead={t("people.inviteLead")}
        initial={[]}
        taken={plan.participants.flatMap((participant) =>
          participant.user ? [participant.user.id] : [],
        )}
        confirmLabel={t("people.inviteConfirm")}
        busy={invite.isPending}
        onConfirm={(people) => (people.length > 0 ? invite.mutate(people) : setPicking(false))}
        onClose={() => setPicking(false)}
      />
    </View>
  );
}
