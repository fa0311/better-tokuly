import { useQuery } from "@tanstack/react-query";
import { fromDate, geo } from "@tokuly/api";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { PlaceRow, placeClient, placeKeys } from "#/features/places";
import { compactDuration, formatDistance, kmh } from "#/lib/format";
import { activityMeta } from "./activity";
import { historyClient, historyKeys } from "./api";

type Period = "week" | "month" | "year" | "all";

/** The flag of a country code as an emoji. */
const flag = (code: string) => (/^[A-Za-z]{2}$/.test(code) ? String.fromCodePoint(...[...code.toUpperCase()].map((letter) => 0x1f1e6 + letter.charCodeAt(0) - 65)) : "🌏");

/** The numbers of a stretch of time: how far, how many places, how I got around, and where I keep going. */
export function StatsPanel() {
  const { t } = useTranslation("history");
  const [period, setPeriod] = useState<Period>("month");
  const now = new Date();
  const midnight = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const from: Date | null =
    period === "week"
      ? new Date(midnight.getFullYear(), midnight.getMonth(), midnight.getDate() - 6)
      : period === "month"
        ? new Date(midnight.getFullYear(), midnight.getMonth(), midnight.getDate() - 29)
        : period === "year"
          ? new Date(now.getFullYear(), 0, 1)
          : null;

  const stats = useQuery({
    queryKey: historyKeys.stats(from?.getTime() ?? null, null),
    queryFn: () => historyClient().getStats({ from: from ? fromDate(from) : undefined }),
  });
  // "Where I keep going" comes from my places: each carries how often I have been.
  const places = useQuery({ queryKey: placeKeys.mine, queryFn: () => placeClient().listMyPlaces({}) });
  const top = [...(places.data?.places ?? [])]
    .filter((place) => place.myVisitCount > 0)
    .sort((a, b) => b.myVisitCount - a.myVisitCount)
    .slice(0, 5);

  const data = stats.data;
  const moving = (data?.byActivity ?? []).filter((entry) => entry.activity !== geo.Activity.STILL && entry.activity !== geo.Activity.UNSPECIFIED && entry.distanceMeters > 0);
  const longest = Math.max(1, ...moving.map((entry) => entry.distanceMeters));
  const nothing = data && data.distanceMeters < 1 && data.visitCount === 0 && Number(data.cellCount) === 0;

  return (
    <div className="mx-auto max-w-2xl">
      <Tabs<Period>
        items={[
          { value: "week", label: t("stats.week") },
          { value: "month", label: t("stats.month") },
          { value: "year", label: t("stats.year") },
          { value: "all", label: t("stats.all") },
        ]}
        value={period}
        onChange={setPeriod}
      />

      {stats.isPending ? (
        <Skeleton className="mt-4 h-64" />
      ) : !data || nothing ? (
        <EmptyState emoji="📊" title={t("stats.emptyTitle")} body={t("stats.emptyBody")} />
      ) : (
        <div className="mt-4 flex flex-col gap-5">
          <section className="sticker rounded-card bg-brand p-5 text-on-brand" aria-label={t("stats.distance")}>
            <p className="text-sm font-black opacity-80">{t("stats.distance")}</p>
            <p className="text-4xl font-black tabular-nums" data-testid="stats-distance">
              {formatDistance(data.distanceMeters)}
            </p>
            {data.maxSpeedMps > 0.5 && <p className="mt-1 text-sm font-bold opacity-80">{t("stats.maxSpeed", { speed: kmh(data.maxSpeedMps) })}</p>}
          </section>

          <dl className="grid grid-cols-3 gap-3">
            <Tile emoji="📍" value={data.visitCount} label={t("stats.visits")} />
            <Tile emoji="🏷️" value={data.placeCount} label={t("stats.places")} />
            <Tile emoji="✨" value={Number(data.cellCount)} label={t("stats.cells")} />
            <Tile emoji="🗾" value={data.regionCount} label={t("stats.regions")} />
            <Tile emoji="🌙" value={data.nightsAway} label={t("stats.nightsAway")} />
            <Tile emoji="🤜" value={data.bumpCount} label={t("stats.bumps")} />
          </dl>

          {data.countryCodes.length > 0 && (
            <section className="sticker rounded-card bg-surface p-5" aria-label={t("stats.countries")}>
              <h2 className="font-black">{t("stats.countriesCount", { count: data.countryCount })}</h2>
              <p className="mt-2 flex flex-wrap gap-2 text-3xl">
                {data.countryCodes.map((code) => (
                  <span key={code} title={code} role="img" aria-label={code}>
                    {flag(code)}
                  </span>
                ))}
              </p>
            </section>
          )}

          {moving.length > 0 && (
            <section className="sticker rounded-card bg-surface p-5" aria-label={t("stats.byActivity")}>
              <h2 className="font-black">{t("stats.byActivity")}</h2>
              <ul className="mt-3 flex flex-col gap-3">
                {moving
                  .sort((a, b) => b.distanceMeters - a.distanceMeters)
                  .map((entry) => {
                    const meta = activityMeta(entry.activity);
                    return (
                      <li key={entry.activity}>
                        <div className="flex items-baseline justify-between gap-2 text-sm font-bold">
                          <span>
                            {meta.emoji} {t(`activity.${meta.key}`)}
                          </span>
                          <span className="text-ink-soft tabular-nums">
                            {formatDistance(entry.distanceMeters)}・{compactDuration(Number(entry.durationSeconds) * 1000)}
                          </span>
                        </div>
                        <div className="mt-1 h-3 overflow-hidden rounded-full bg-surface-3" aria-hidden>
                          <div className="h-full rounded-full bg-lime" style={{ width: `${Math.max(3, (entry.distanceMeters / longest) * 100)}%` }} />
                        </div>
                      </li>
                    );
                  })}
              </ul>
            </section>
          )}

          {top.length > 0 && (
            <section aria-label={t("stats.topPlaces")}>
              <h2 className="text-lg font-black">{t("stats.topPlaces")}</h2>
              <ol className="mt-3 flex flex-col gap-3">
                {top.map((place) => (
                  <li key={place.id}>
                    <PlaceRow place={place} />
                  </li>
                ))}
              </ol>
            </section>
          )}
        </div>
      )}
    </div>
  );
}

function Tile({ emoji, value, label }: { emoji: string; value: number; label: string }) {
  return (
    <div className="sticker-sm rounded-2xl bg-surface p-3 text-center">
      <div className="text-xl" aria-hidden>
        {emoji}
      </div>
      <dd className="text-xl font-black tabular-nums">{value.toLocaleString()}</dd>
      <dt className="text-[11px] font-bold text-ink-faint">{label}</dt>
    </div>
  );
}
