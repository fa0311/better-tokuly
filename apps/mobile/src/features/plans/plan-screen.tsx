import { useMutation, useQueryClient } from "@tanstack/react-query";
import { plans, toDate } from "@tokuly/api";
import {
  forgetPlan,
  formatDay,
  isHostLike,
  nameOf,
  planTimeText,
  rememberPlan,
  todayYmd,
  untilText,
  ymd,
} from "@tokuly/core/plans";
import { router, Stack } from "expo-router";
import { BellRing, CalendarClock, MapPin, MoreHorizontal, Navigation } from "lucide-react-native";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { ActionSheet, type SheetAction } from "#/components/action-sheet";
import { Button } from "#/components/button";
import { IconButton } from "#/components/icon-button";
import { EmptyState, Page, Row, Section, Spinner } from "#/components/page";
import { webScreenHref } from "#/features/web/handoff";
import { INK_ON_BRIGHT, sticker, useTheme } from "#/lib/theme";
import { asViewer, showPlanError } from "./actions";
import { usePlan, usePlanZone } from "./data";
import { Participants } from "./participants";
import { Chip, PlanEmoji } from "./parts";
import { openInMaps, PlaceMap } from "./place";
import { RsvpCard } from "./rsvp-card";

/**
 * A plan: when and where (on the map, and to the map app for the way
 * there), the reminders, the viewer's answer and everyone else's. The
 * checklist, the cost split and comments are on the web screen of the plan.
 */
export function PlanScreen({ planId }: { planId: string }) {
  const { t } = useTranslation("plans");
  const { data: plan, isPending, isError } = usePlan(planId);

  if (isPending) return <Spinner />;
  if (isError || !plan) {
    return (
      <Page testID="plan-missing">
        <EmptyState
          emoji="🫥"
          title={t("plan.notFound")}
          body={t("plan.notFoundBody")}
          action={<Button onPress={() => router.replace("/plans")}>{t("plan.backToPlans")}</Button>}
        />
      </Page>
    );
  }
  return <LoadedPlan plan={plan} />;
}

function LoadedPlan({ plan }: { plan: plans.Plan }) {
  const { t, i18n } = useTranslation(["plans", "mobile"]);
  const { colors } = useTheme();
  const zone = usePlanZone();
  const queryClient = useQueryClient();
  const [menu, setMenu] = useState(false);

  const cancelled = plan.status === plans.PlanStatus.CANCELLED;
  const iAmHost = plan.myRole === plans.PlanRole.HOST;
  const host = plan.participants.find((participant) => participant.role === plans.PlanRole.HOST);
  const until = !cancelled && !plan.over ? untilText(i18n, plan.time, zone) : undefined;
  const cancelledAt = toDate(plan.cancelledAt);
  const timed = plan.time?.start.case === "startAt";
  const reminders = [
    plan.remindDayBefore ? t("plans:plan.remindDayBefore") : "",
    plan.remindMinutesBefore > 0 && timed
      ? t("plans:plan.remindMinutes", { count: plan.remindMinutesBefore })
      : "",
  ].filter(Boolean);

  const cancel = useMutation({
    mutationFn: () => asViewer((service) => service.cancelPlan({ planId: plan.id, reason: "" })),
    onSuccess: (response) => {
      if (response) rememberPlan(queryClient, response.plan);
    },
    onError: showPlanError,
  });
  const leave = useMutation({
    mutationFn: () => asViewer((service) => service.leavePlan({ planId: plan.id })),
    onSuccess: (response) => {
      if (!response) return;
      forgetPlan(queryClient, plan.id);
      if (router.canGoBack()) router.back();
      else router.replace("/plans");
    },
    onError: showPlanError,
  });

  const confirm = (title: string, body: string, label: string, run: () => void) =>
    Alert.alert(title, body, [
      { text: t("common:actions.cancel"), style: "cancel" },
      { text: label, style: "destructive", onPress: run },
    ]);
  const actions: SheetAction[] = [
    ...(isHostLike(plan) && !cancelled
      ? [
          {
            key: "edit",
            label: t("plans:plan.menu.edit"),
            onPress: () => router.push(`/plans/${plan.id}/edit`),
          },
        ]
      : []),
    {
      key: "web",
      label: t("mobile:plans.openOnWeb"),
      onPress: () => router.push(webScreenHref(`/plans/${plan.id}`) as never),
    },
    ...(iAmHost
      ? []
      : [
          {
            key: "leave",
            label: t("plans:plan.menu.leave"),
            destructive: true,
            onPress: () =>
              confirm(
                t("plans:plan.leaveTitle"),
                t("plans:plan.leaveBody"),
                t("plans:plan.leaveConfirm"),
                () => leave.mutate(),
              ),
          },
        ]),
    ...(iAmHost && !cancelled
      ? [
          {
            key: "cancel",
            label: t("plans:plan.menu.cancel"),
            destructive: true,
            onPress: () =>
              confirm(
                t("plans:plan.cancelTitle"),
                t("plans:plan.cancelBody"),
                t("plans:plan.cancelConfirm"),
                () => cancel.mutate(),
              ),
          },
        ]
      : []),
  ];

  return (
    <Page testID="plan-screen">
      <Stack.Screen
        options={{
          title: plan.title,
          headerRight: () => (
            <IconButton
              testID="plan-menu"
              label={t("plans:plan.menu.label")}
              icon={<MoreHorizontal size={22} color={colors.ink} />}
              onPress={() => setMenu(true)}
            />
          ),
        }}
      />
      <View className="gap-4 rounded-card bg-surface p-4" style={sticker(colors)}>
        <View className="flex-row items-start gap-4">
          <PlanEmoji emoji={plan.emoji} size="lg" />
          <View className="flex-1 gap-1.5">
            <Text testID="plan-title" className="text-2xl font-black leading-8 text-ink">
              {plan.title}
            </Text>
            <View className="flex-row flex-wrap gap-1.5">
              {cancelled && (
                <Chip label={t("plans:plan.cancelled")} box="bg-danger" ink="#ffffff" />
              )}
              {!cancelled && plan.over && <Chip label={t("plans:plan.over")} box="bg-surface-3" />}
              {until && <Chip label={until} box="bg-lime" ink={INK_ON_BRIGHT} />}
            </View>
          </View>
        </View>
        {cancelled && (
          <View className="rounded-2xl bg-danger/10 px-4 py-3">
            <Text className="text-sm font-bold text-ink">
              {t("plans:plan.cancelledBanner", {
                date: cancelledAt
                  ? formatDay(ymd(cancelledAt, zone), i18n.language, todayYmd(zone))
                  : "",
              })}
            </Text>
            {plan.cancelReason ? (
              <Text className="mt-1 text-sm text-ink">「{plan.cancelReason}」</Text>
            ) : null}
          </View>
        )}
        <Fact icon={<CalendarClock size={20} color={INK_ON_BRIGHT} />} box="bg-sun">
          <Text testID="plan-time" className="font-bold text-ink">
            {planTimeText(i18n, plan.time, zone)}
          </Text>
        </Fact>
        {plan.place?.name ? (
          <Fact icon={<MapPin size={20} color={INK_ON_BRIGHT} />} box="bg-sky">
            <Pressable
              testID="plan-place"
              accessibilityRole="link"
              accessibilityHint={t("mobile:plans.openInMaps")}
              onPress={() => plan.place && openInMaps(plan.place)}
            >
              <Text className="font-bold text-ink">{plan.place.name}</Text>
              {plan.place.address ? (
                <Text className="text-sm text-ink-soft">{plan.place.address}</Text>
              ) : null}
              {plan.place.location && <PlaceMap location={plan.place.location} />}
              <View className="mt-2 flex-row items-center gap-1">
                <Navigation size={14} color={colors.brand} />
                <Text className="text-sm font-black text-brand">
                  {t("mobile:plans.openInMaps")}
                </Text>
              </View>
            </Pressable>
          </Fact>
        ) : null}
        {!cancelled && !plan.over && plan.time?.start.case && reminders.length > 0 && (
          <Fact icon={<BellRing size={20} color="#ffffff" />} box="bg-pink">
            <Text testID="plan-reminders" className="text-sm font-bold text-ink-soft">
              {reminders.join(" · ")}
            </Text>
          </Fact>
        )}
        {plan.description ? <Text className="leading-6 text-ink">{plan.description}</Text> : null}
        <Text className="text-sm font-bold text-ink-faint">
          {t("plans:plan.hostedBy", { name: nameOf(host?.user) })}
        </Text>
      </View>

      <RsvpCard plan={plan} />
      <Participants plan={plan} />

      <Section title={t("mobile:plans.onWeb")}>
        <Row
          testID="plan-web"
          label={t("mobile:plans.extrasOnWeb")}
          hint={t("mobile:plans.extrasOnWebHint")}
          onPress={() => router.push(webScreenHref(`/plans/${plan.id}`) as never)}
        />
      </Section>

      <ActionSheet
        visible={menu}
        title={plan.title}
        actions={actions}
        onClose={() => setMenu(false)}
      />
    </Page>
  );
}

/** One line of what the plan is: a coloured icon and its text. */
function Fact({ icon, box, children }: { icon: ReactNode; box: string; children: ReactNode }) {
  return (
    <View className="flex-row items-start gap-3">
      <View className={`size-9 items-center justify-center rounded-xl ${box}`}>{icon}</View>
      <View className="flex-1 self-center">{children}</View>
    </View>
  );
}
