import { plans } from "@tokuly/api";
import { planTimeText, untilText } from "@tokuly/core/plans";
import { router } from "expo-router";
import { MapPin } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { sticker, useTheme } from "#/lib/theme";
import { AvatarStack, Chip, PlanEmoji, RsvpChip } from "./parts";

/** A plan in a list: when, what, where, who goes, and the viewer's own answer. */
export function PlanCard({ plan, zone }: { plan: plans.Plan; zone: string }) {
  const { t, i18n } = useTranslation("plans");
  const { colors } = useTheme();
  const cancelled = plan.status === plans.PlanStatus.CANCELLED;
  const until = !cancelled && !plan.over ? untilText(i18n, plan.time, zone) : undefined;
  const people = plan.participants.flatMap((participant) =>
    participant.user ? [participant.user] : [],
  );
  return (
    <Pressable
      testID={`plan-card-${plan.id}`}
      accessibilityRole="button"
      onPress={() => router.push(`/plans/${plan.id}`)}
      className={`flex-row gap-3 rounded-card bg-surface p-4 ${cancelled || plan.over ? "opacity-75" : ""}`}
      style={sticker(colors)}
    >
      <PlanEmoji emoji={plan.emoji} size="md" />
      <View className="flex-1 gap-1">
        <View className="flex-row items-center gap-2">
          <Text
            numberOfLines={1}
            className={`flex-1 text-lg font-black text-ink ${cancelled ? "line-through" : ""}`}
          >
            {plan.title}
          </Text>
          {cancelled ? (
            <Chip label={t("plan.cancelled")} box="bg-danger" ink="#ffffff" />
          ) : (
            <RsvpChip rsvp={plan.myRsvp} />
          )}
        </View>
        <View className="flex-row flex-wrap items-center gap-2">
          <Text className="text-sm font-bold text-ink-soft">
            {planTimeText(i18n, plan.time, zone)}
          </Text>
          {until && <Chip label={until} box="bg-brand-soft" ink={colors["brand-strong"]} />}
        </View>
        {plan.place?.name ? (
          <View className="flex-row items-center gap-1">
            <MapPin size={14} color={colors["ink-soft"]} />
            <Text numberOfLines={1} className="flex-1 text-sm text-ink-soft">
              {plan.place.name}
            </Text>
          </View>
        ) : null}
        <View className="mt-1 flex-row items-center gap-3">
          <AvatarStack users={people} total={plan.counts?.going ?? people.length} />
          <Text className="text-xs font-bold text-ink-soft">
            {t("plan.goingCount", { count: plan.counts?.going ?? 0 })}
          </Text>
        </View>
      </View>
    </Pressable>
  );
}
