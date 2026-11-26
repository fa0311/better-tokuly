import { plans } from "@tokuly/api";
import { router } from "expo-router";
import { CalendarDays, CalendarHeart, Hourglass, Plus, Vote } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState, Page, Row, Section, Spinner } from "#/components/page";
import { Tabs } from "#/components/tabs";
import { webScreenHref } from "#/features/web/handoff";
import { useTheme } from "#/lib/theme";
import { usePlanList, usePlanZone } from "./data";
import { PlanCard } from "./plan-card";

type Filter = "upcoming" | "invited" | "past";
const FILTERS: Record<Filter, plans.PlanListFilter> = {
  upcoming: plans.PlanListFilter.UPCOMING,
  invited: plans.PlanListFilter.INVITED,
  past: plans.PlanListFilter.PAST,
};
const EMPTY_EMOJI: Record<Filter, string> = { upcoming: "🎈", invited: "💌", past: "📼" };

/** The rest of "あそぶ", on the web for now. */
const WEB_TOOLS = [
  { key: "free", label: "free.title", path: "/plans/free", Icon: CalendarHeart },
  { key: "polls", label: "tabs.polls", path: "/plans?tab=polls", Icon: Vote },
  { key: "moments", label: "tabs.moments", path: "/plans?tab=moments", Icon: Hourglass },
  { key: "calendar", label: "calendar.title", path: "/plans/calendar", Icon: CalendarDays },
] as const;

/**
 * The viewer's plans — what is coming (their own and the ones friends asked
 * them to), invitations waiting for an answer, what is over — and the way
 * to a new one.
 */
export function PlansScreen() {
  const { t } = useTranslation(["plans", "mobile"]);
  const { colors } = useTheme();
  const zone = usePlanZone();
  const [filter, setFilter] = useState<Filter>("upcoming");
  const invited = usePlanList(plans.PlanListFilter.INVITED);
  const list = usePlanList(FILTERS[filter]);

  return (
    <Page testID="plans-screen">
      <Button
        testID="plans-new"
        icon={<Plus size={18} color={colors["on-brand"]} />}
        onPress={() => router.push(webScreenHref("/plans/new") as never)}
      >
        {t("plans:plan.new")}
      </Button>
      <Tabs<Filter>
        testID="plans-filter"
        value={filter}
        onChange={setFilter}
        items={[
          { value: "upcoming", label: t("plans:plan.filter.upcoming") },
          {
            value: "invited",
            label: t("plans:plan.filter.invited"),
            badge: invited.plans?.length,
          },
          { value: "past", label: t("plans:plan.filter.past") },
        ]}
      />
      {list.paging.isPending ? (
        <Spinner />
      ) : list.plans?.length ? (
        <View className="gap-3">
          {list.plans.map((plan) => (
            <PlanCard key={plan.id} plan={plan} zone={zone} />
          ))}
          {list.paging.hasNextPage && (
            <Button
              variant="secondary"
              block
              loading={list.paging.isFetchingNextPage}
              onPress={() => void list.paging.fetchNextPage()}
            >
              {t("mobile:plans.more")}
            </Button>
          )}
        </View>
      ) : (
        <EmptyState
          emoji={EMPTY_EMOJI[filter]}
          title={t(`plans:plan.empty.${filter}`)}
          body={t(`plans:plan.emptyBody.${filter}`)}
        />
      )}
      <Section title={t("mobile:plans.onWeb")}>
        {WEB_TOOLS.map(({ key, label, path, Icon }) => (
          <Row
            key={key}
            testID={`plans-web-${key}`}
            icon={<Icon size={20} color={colors.ink} />}
            label={t(`plans:${label}`)}
            onPress={() => router.push(webScreenHref(path) as never)}
          />
        ))}
      </Section>
    </Page>
  );
}
