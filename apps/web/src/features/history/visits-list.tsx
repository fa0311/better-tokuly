import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { toDate } from "@tokuly/api";
import { dayRange, formatDay, historyQueries } from "@tokuly/core/history";
import { placeEmoji, placeQueries } from "@tokuly/core/places";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { Tabs } from "#/components/ui/tabs";
import { GazetteerCredit, placeClient } from "#/features/places";
import {
  accountTimeZone,
  shiftDate,
  useAccountTimeZone,
  useAccountToday,
  zonedMoment,
} from "#/lib/account-time";
import { historyClient } from "./api";
import { VisitCard } from "./visit-card";

type Period = "all" | "week" | "month" | "custom";
const ANY_PLACE = "any";

/** Every visit, newest first, narrowed by when and by which of my places. */
export function VisitsList() {
  const { t, i18n } = useTranslation("history");
  const zone = useAccountTimeZone();
  const today = useAccountToday();
  const [period, setPeriod] = useState<Period>("all");
  const [placeId, setPlaceId] = useState(ANY_PLACE);
  const [customFrom, setCustomFrom] = useState(today);
  const [customTo, setCustomTo] = useState(today);

  const places = useQuery(placeQueries.mine(placeClient));

  // Days begin at the account's midnight.
  const startOf = (day: string) => new Date(zonedMoment(day, 0, zone));
  const from: Date | null =
    period === "week"
      ? startOf(shiftDate(today, -6))
      : period === "month"
        ? startOf(`${today.slice(0, 7)}-01`)
        : period === "custom"
          ? (dayRange(customFrom, zone)?.from ?? null)
          : null;
  const to: Date | null = period === "custom" ? (dayRange(customTo, zone)?.to ?? null) : null;
  const rangeValid = period !== "custom" || Boolean(from && to && from < to);

  const query = useInfiniteQuery({
    ...historyQueries.visits(historyClient, from, to, placeId === ANY_PLACE ? "" : placeId),
    enabled: rangeValid,
  });
  const visits = query.data?.pages.flatMap((page) => page.visits) ?? [];
  const filtered = period !== "all" || placeId !== ANY_PLACE;

  return (
    <div className="mx-auto max-w-2xl">
      <div className="flex flex-col gap-3">
        <Tabs<Period>
          items={[
            { value: "all", label: t("visits.all") },
            { value: "week", label: t("visits.week") },
            { value: "month", label: t("visits.month") },
            { value: "custom", label: t("visits.custom") },
          ]}
          value={period}
          onChange={setPeriod}
        />
        {period === "custom" && (
          <div className="grid grid-cols-2 gap-3">
            <TextField
              type="date"
              label={t("visits.from")}
              value={customFrom}
              max={today}
              onChange={(event) => setCustomFrom(event.target.value)}
            />
            <TextField
              type="date"
              label={t("visits.to")}
              value={customTo}
              max={today}
              error={rangeValid ? undefined : t("delete.invalidRange")}
              onChange={(event) => setCustomTo(event.target.value)}
            />
          </div>
        )}
        {(places.data?.places.length ?? 0) > 0 && (
          <SelectField
            label={t("visits.place")}
            value={placeId}
            onChange={setPlaceId}
            options={[
              { value: ANY_PLACE, label: t("visits.anyPlace") },
              ...places.data!.places.map((place) => ({
                value: place.id,
                label: `${placeEmoji(place)} ${place.name}`,
              })),
            ]}
          />
        )}
      </div>

      <div className="mt-4">
        {!rangeValid ? null : query.isPending ? (
          <div className="flex flex-col gap-3">
            <Skeleton className="h-20" />
            <Skeleton className="h-20" />
          </div>
        ) : visits.length === 0 ? (
          filtered ? (
            <EmptyState emoji="🔍" title={t("visits.emptyFiltered")} />
          ) : (
            <EmptyState emoji="👣" title={t("visits.emptyTitle")} body={t("visits.emptyBody")} />
          )
        ) : (
          <>
            <ul className="flex flex-col gap-3" data-testid="visit-list">
              {visits.map((visit) => {
                const arrived = toDate(visit.arrivedAt);
                return (
                  <li key={visit.id} className="sticker rounded-card bg-surface p-4">
                    <VisitCard
                      visit={visit}
                      dateLabel={arrived?.toLocaleDateString(i18n.language, {
                        timeZone: accountTimeZone(),
                        month: "short",
                        day: "numeric",
                        weekday: "short",
                      })}
                    />
                    {arrived && (
                      <Link
                        to="/history/$date"
                        params={{ date: formatDay(arrived) }}
                        className="mt-2 inline-block text-xs font-black text-brand hover:underline"
                      >
                        {t("visits.openDay")}
                      </Link>
                    )}
                  </li>
                );
              })}
            </ul>
            <LoadMore query={query} />
            <GazetteerCredit className="mt-4 text-center" />
          </>
        )}
      </div>
    </div>
  );
}
