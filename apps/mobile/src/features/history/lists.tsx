import { toDate, toMillis } from "@tokuly/api";
import { shiftDate, zonedDate, zonedMoment } from "@tokuly/core/format";
import { router } from "expo-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Button } from "#/components/button";
import { SwitchRow } from "#/components/form-rows";
import { EmptyState, ErrorNote, Section, Spinner } from "#/components/page";
import { Tabs } from "#/components/tabs";
import { errorMessage } from "#/lib/errors";
import { sticker, useTheme } from "#/lib/theme";
import { NS, useAccountTimeZone, useAccountToday, useNights, useVisits } from "./data";
import { Faces, GazetteerCredit, VisitCard } from "./parts";

type Period = "all" | "week" | "month";

/** "もっと見る" under a list that has more pages. */
function More({
  query,
}: {
  query: { hasNextPage: boolean; isFetchingNextPage: boolean; fetchNextPage(): unknown };
}) {
  const { t } = useTranslation();
  if (!query.hasNextPage) return null;
  return (
    <View className="items-center">
      <Button
        variant="ghost"
        loading={query.isFetchingNextPage}
        onPress={() => void query.fetchNextPage()}
      >
        {t("actions.more")}
      </Button>
    </View>
  );
}

/** Every visit, newest first, of the last week, this month or all time; each leads to its day. */
export function VisitsList() {
  const { t, i18n } = useTranslation(NS);
  const zone = useAccountTimeZone();
  const today = useAccountToday();
  const [period, setPeriod] = useState<Period>("all");
  // Days begin at the account's midnight.
  const from =
    period === "week"
      ? new Date(zonedMoment(shiftDate(today, -6), 0, zone))
      : period === "month"
        ? new Date(zonedMoment(`${today.slice(0, 7)}-01`, 0, zone))
        : null;
  const query = useVisits(from);
  const visits = query.data?.pages.flatMap((page) => page.visits) ?? [];
  return (
    <View className="gap-3">
      <Tabs<Period>
        testID="history-visits-period"
        items={[
          { value: "all", label: t("visits.all") },
          { value: "week", label: t("visits.week") },
          { value: "month", label: t("visits.month") },
        ]}
        value={period}
        onChange={setPeriod}
      />
      {query.isPending ? (
        <Spinner />
      ) : query.isError ? (
        <ErrorNote>{errorMessage(query.error, { namespace: NS })}</ErrorNote>
      ) : visits.length === 0 ? (
        period === "all" ? (
          <EmptyState emoji="👣" title={t("visits.emptyTitle")} body={t("visits.emptyBody")} />
        ) : (
          <EmptyState emoji="🔍" title={t("visits.emptyFiltered")} />
        )
      ) : (
        <>
          {visits.map((visit) => {
            const arrived = toMillis(visit.arrivedAt);
            const date = arrived > 0 ? zonedDate(arrived, zone) : "";
            return (
              <VisitCard
                key={visit.id}
                visit={visit}
                dateLabel={toDate(visit.arrivedAt)?.toLocaleDateString(i18n.language, {
                  timeZone: zone,
                  month: "short",
                  day: "numeric",
                  weekday: "short",
                })}
                footer={
                  date !== "" && (
                    <Pressable
                      testID={`visit-open-day-${visit.id}`}
                      onPress={() => router.push(`/history/${date}`)}
                    >
                      <Text className="mt-1 text-xs font-black text-brand">
                        {t("visits.openDay")}
                      </Text>
                    </Pressable>
                  )
                }
              />
            );
          })}
          <More query={query} />
          <GazetteerCredit />
        </>
      )}
    </View>
  );
}

/** Where the person slept, night by night; the switch keeps only the nights away from home. */
export function NightsList() {
  const { t, i18n } = useTranslation(NS);
  const { colors } = useTheme();
  const zone = useAccountTimeZone();
  const [awayOnly, setAwayOnly] = useState(false);
  const query = useNights();
  const all = query.data?.pages.flatMap((page) => page.nights) ?? [];
  const nights = awayOnly ? all.filter((night) => !night.atHome) : all;
  const clock = (date: Date | undefined) =>
    date?.toLocaleTimeString(i18n.language, {
      timeZone: zone,
      hour: "2-digit",
      minute: "2-digit",
    }) ?? "";
  return (
    <View className="gap-3">
      <Section>
        <SwitchRow
          testID="history-nights-away"
          label={t("nights.awayOnly")}
          hint={t("nights.awayOnlyHint")}
          value={awayOnly}
          onChange={setAwayOnly}
        />
      </Section>
      {query.isPending ? (
        <Spinner />
      ) : query.isError ? (
        <ErrorNote>{errorMessage(query.error, { namespace: NS })}</ErrorNote>
      ) : nights.length === 0 ? (
        <EmptyState
          emoji="🌙"
          title={awayOnly && all.length > 0 ? t("nights.emptyAway") : t("nights.emptyTitle")}
          body={awayOnly && all.length > 0 ? undefined : t("nights.emptyBody")}
        />
      ) : (
        <>
          {nights.map((night) => (
            <Pressable
              key={`${night.date}-${toMillis(night.from)}`}
              testID={`night-${night.date}`}
              accessibilityRole="button"
              onPress={() => router.push(`/history/${night.date}`)}
              className={`flex-row items-center gap-3 rounded-card p-4 ${night.atHome ? "bg-surface" : "bg-sky"}`}
              style={sticker(colors)}
            >
              <Text className="text-2xl">{night.atHome ? "🏠" : "🌙"}</Text>
              <View className="min-w-0 flex-1 gap-0.5">
                <Text
                  numberOfLines={1}
                  className={`font-black ${night.atHome ? "text-ink" : "text-[#17122b]"}`}
                >
                  {night.place?.name ||
                    night.name ||
                    (night.atHome ? t("nights.home") : t("nights.somewhere"))}
                </Text>
                <Text
                  className={`text-sm font-bold ${night.atHome ? "text-ink-soft" : "text-[#17122b]"}`}
                >
                  {new Date(`${night.date}T12:00:00`).toLocaleDateString(i18n.language, {
                    month: "short",
                    day: "numeric",
                    weekday: "short",
                  })}{" "}
                  {clock(toDate(night.from))}〜{clock(toDate(night.to))}
                </Text>
              </View>
              {night.withFriends.length > 0 && (
                <Faces users={night.withFriends.slice(0, 3)} size="xs" />
              )}
            </Pressable>
          ))}
          <More query={query} />
          <GazetteerCredit />
        </>
      )}
    </View>
  );
}
