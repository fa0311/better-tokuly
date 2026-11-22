import { Link } from "@tanstack/react-router";
import { plans } from "@tokuly/api";
import { CalendarDays, Plus } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { useCountdowns, usePlanList, usePolls } from "./api";
import { Moments } from "./countdown/moments";
import { FreeFriendsStrip } from "./free/free-strip";
import { ChoiceChips } from "./parts";
import { PlanCard } from "./plan/plan-card";
import { PollCard } from "./poll/poll-card";

export const PLANS_TABS = ["plans", "polls", "moments"] as const;
export type PlansTab = (typeof PLANS_TABS)[number];

/** `?tab=plans|polls|moments`; anything else is the plans tab. */
export function parsePlansSearch(search: Record<string, unknown>): { tab?: PlansTab } {
  return PLANS_TABS.includes(search.tab as PlansTab) && search.tab !== "plans" ? { tab: search.tab as PlansTab } : {};
}

const linkButton = "pressable sticker inline-flex h-11 items-center justify-center gap-2 rounded-full px-5 text-[15px] font-bold";

/**
 * `/plans`: the hub of "あそぶ" — who is free right now on top, then the
 * person's plans, date polls, and countdowns.
 */
export function PlansScreen({ tab, onTabChange }: { tab: PlansTab; onTabChange(tab: PlansTab): void }) {
  const { t } = useTranslation("plans");
  const invited = usePlanList(plans.PlanListFilter.INVITED);
  const polls = usePolls();
  const countdowns = useCountdowns();
  const waiting = polls.data?.filter((poll) => poll.status === plans.PollStatus.OPEN && !poll.answeredByMe).length ?? 0;
  const todays = (countdowns.data ?? []).filter((countdown) => countdown.daysLeft === 0);

  return (
    <Page
      title={t("title")}
      wide
      actions={
        <Link to="/plans/calendar" className="pressable sticker-sm inline-flex h-10 items-center gap-1.5 rounded-full bg-surface px-4 text-sm font-bold">
          <CalendarDays className="size-4" />
          {t("calendar.title")}
        </Link>
      }
    >
      <FreeFriendsStrip />

      {todays.length > 0 && (
        <ul className="mt-4 flex flex-col gap-2">
          {todays.map((countdown) => (
            <li key={countdown.id}>
              <Link to="/plans/countdowns/$countdownId" params={{ countdownId: countdown.id }} className="pressable sticker flex animate-pop-in items-center gap-3 rounded-card bg-brand px-4 py-3 text-on-brand">
                <span className="animate-float text-3xl" aria-hidden>
                  {countdown.emoji || "🎉"}
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block text-xs font-black opacity-85">{t("countdown.momentKicker")}</span>
                  <span className="block truncate text-lg font-black">{countdown.title}</span>
                </span>
              </Link>
            </li>
          ))}
        </ul>
      )}

      <Tabs<PlansTab>
        className="mt-6 mb-4"
        value={tab}
        onChange={onTabChange}
        items={[
          { value: "plans", label: t("tabs.plans"), badge: invited.data?.length },
          { value: "polls", label: t("tabs.polls"), badge: waiting },
          { value: "moments", label: t("tabs.moments") },
        ]}
      />

      {tab === "plans" && <PlansTabBody invitedCount={invited.data?.length ?? 0} />}
      {tab === "polls" && (
        <div className="flex flex-col gap-4">
          <Link to="/plans/polls/new" className={`${linkButton} self-start bg-brand text-on-brand`}>
            <Plus className="size-4" />
            {t("poll.new")}
          </Link>
          {polls.isPending ? (
            <ListSkeleton />
          ) : polls.data?.length ? (
            <ul className="grid gap-3 lg:grid-cols-2">
              {polls.data.map((poll) => (
                <li key={poll.id}>
                  <PollCard poll={poll} />
                </li>
              ))}
            </ul>
          ) : (
            <EmptyState emoji="🗳️" title={t("poll.empty")} body={t("poll.emptyBody")} />
          )}
        </div>
      )}
      {tab === "moments" && <Moments />}
    </Page>
  );
}

type Filter = "upcoming" | "invited" | "past";
const FILTERS: Record<Filter, plans.PlanListFilter> = { upcoming: plans.PlanListFilter.UPCOMING, invited: plans.PlanListFilter.INVITED, past: plans.PlanListFilter.PAST };

function PlansTabBody({ invitedCount }: { invitedCount: number }) {
  const { t } = useTranslation("plans");
  const [filter, setFilter] = useState<Filter>("upcoming");
  const list = usePlanList(FILTERS[filter]);

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-3">
        <Link to="/plans/new" className={`${linkButton} bg-brand text-on-brand`}>
          <Plus className="size-4" />
          {t("plan.new")}
        </Link>
        <ChoiceChips<Filter>
          label={t("plan.filter.label")}
          value={filter}
          onChange={setFilter}
          options={[
            { value: "upcoming", label: t("plan.filter.upcoming") },
            {
              value: "invited",
              label: (
                <>
                  {t("plan.filter.invited")}
                  {invitedCount > 0 && <span className="rounded-full bg-pink px-1.5 text-[11px] font-black text-white">{invitedCount}</span>}
                </>
              ),
            },
            { value: "past", label: t("plan.filter.past") },
          ]}
        />
      </div>
      {list.isPending ? (
        <ListSkeleton />
      ) : list.data?.length ? (
        <ul className="grid gap-3 lg:grid-cols-2">
          {list.data.map((plan) => (
            <li key={plan.id}>
              <PlanCard plan={plan} />
            </li>
          ))}
        </ul>
      ) : (
        <EmptyState emoji={filter === "past" ? "📼" : filter === "invited" ? "💌" : "🎈"} title={t(`plan.empty.${filter}`)} body={t(`plan.emptyBody.${filter}`)} />
      )}
    </div>
  );
}

function ListSkeleton() {
  return (
    <div className="grid gap-3 lg:grid-cols-2">
      <Skeleton className="h-28 rounded-card" />
      <Skeleton className="h-28 rounded-card" />
    </div>
  );
}
