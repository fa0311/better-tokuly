import { geo } from "@tokuly/api";
import { compactDuration, formatDistance, kmh, shiftDate, zonedMoment } from "@tokuly/core/format";
import { activityMeta } from "@tokuly/core/history";
import { placeEmoji } from "@tokuly/core/places";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";
import { EmptyState, ErrorNote, Row, Section, Spinner } from "#/components/page";
import { Tabs } from "#/components/tabs";
import { errorMessage } from "#/lib/errors";
import { sticker, useTheme } from "#/lib/theme";
import { NS, useAccountTimeZone, useAccountToday, useMyPlaces, useStats } from "./data";
import { openPath } from "./open";

type Period = "week" | "month" | "year" | "all";

/** The flag of a country code as an emoji. */
const flag = (code: string) =>
  /^[A-Za-z]{2}$/.test(code)
    ? String.fromCodePoint(
        ...[...code.toUpperCase()].map((letter) => 0x1f1e6 + letter.charCodeAt(0) - 65),
      )
    : "🌏";

/** The numbers of a stretch of time: how far, how many places, how the person got around, where they keep going. */
export function StatsPanel() {
  const { t, i18n } = useTranslation(NS);
  const { colors } = useTheme();
  const zone = useAccountTimeZone();
  const today = useAccountToday();
  const [period, setPeriod] = useState<Period>("month");
  // Days begin at the account's midnight.
  const startOf = (day: string) => new Date(zonedMoment(day, 0, zone));
  const from =
    period === "week"
      ? startOf(shiftDate(today, -6))
      : period === "month"
        ? startOf(shiftDate(today, -29))
        : period === "year"
          ? startOf(`${today.slice(0, 4)}-01-01`)
          : null;
  const stats = useStats(from);
  const places = useMyPlaces();
  const top = [...(places.data?.places ?? [])]
    .filter((place) => place.myVisitCount > 0)
    .sort((a, b) => b.myVisitCount - a.myVisitCount)
    .slice(0, 5);
  const data = stats.data;
  const nothing =
    data && data.distanceMeters < 1 && data.visitCount === 0 && Number(data.cellCount) === 0;
  const moving = (data?.byActivity ?? [])
    .filter(
      (entry) =>
        entry.activity !== geo.Activity.STILL &&
        entry.activity !== geo.Activity.UNSPECIFIED &&
        entry.distanceMeters > 0,
    )
    .sort((a, b) => b.distanceMeters - a.distanceMeters);
  const longest = moving[0]?.distanceMeters ?? 1;

  return (
    <View className="gap-4">
      <Tabs<Period>
        testID="history-stats-period"
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
        <Spinner />
      ) : stats.isError ? (
        <ErrorNote>{errorMessage(stats.error, { namespace: NS })}</ErrorNote>
      ) : !data || nothing ? (
        <EmptyState emoji="📊" title={t("stats.emptyTitle")} body={t("stats.emptyBody")} />
      ) : (
        <>
          <View className="rounded-card bg-brand p-5" style={sticker(colors)}>
            <Text className="text-sm font-black text-on-brand opacity-80">
              {t("stats.distance")}
            </Text>
            <Text testID="history-stats-distance" className="text-4xl font-black text-on-brand">
              {formatDistance(data.distanceMeters)}
            </Text>
            {data.maxSpeedMps > 0.5 && (
              <Text className="mt-1 text-sm font-bold text-on-brand opacity-80">
                {t("stats.maxSpeed", { speed: kmh(data.maxSpeedMps) })}
              </Text>
            )}
          </View>
          <View className="flex-row flex-wrap gap-3">
            <Tile emoji="📍" value={data.visitCount} label={t("stats.visits")} />
            <Tile emoji="🏷️" value={data.placeCount} label={t("stats.places")} />
            <Tile emoji="✨" value={Number(data.cellCount)} label={t("stats.cells")} />
            <Tile emoji="🗾" value={data.regionCount} label={t("stats.regions")} />
            <Tile emoji="🌙" value={data.nightsAway} label={t("stats.nightsAway")} />
            <Tile emoji="🤜" value={data.bumpCount} label={t("stats.bumps")} />
          </View>
          {data.countryCodes.length > 0 && (
            <View className="gap-2 rounded-card bg-surface p-5" style={sticker(colors)}>
              <Text className="font-black text-ink">
                {t("stats.countriesCount", { count: data.countryCount })}
              </Text>
              <Text className="text-3xl">{data.countryCodes.map(flag).join(" ")}</Text>
            </View>
          )}
          {moving.length > 0 && (
            <View className="gap-3 rounded-card bg-surface p-5" style={sticker(colors)}>
              <Text className="font-black text-ink">{t("stats.byActivity")}</Text>
              {moving.map((entry) => {
                const meta = activityMeta(entry.activity);
                return (
                  <View key={entry.activity} className="gap-1">
                    <View className="flex-row justify-between gap-2">
                      <Text className="text-sm font-bold text-ink">
                        {meta.emoji} {t(`activity.${meta.key}`)}
                      </Text>
                      <Text className="text-sm font-bold text-ink-soft">
                        {formatDistance(entry.distanceMeters)}・
                        {compactDuration(i18n.language, Number(entry.durationSeconds) * 1000)}
                      </Text>
                    </View>
                    <View className="h-3 overflow-hidden rounded-full bg-surface-3">
                      <View
                        className="h-full rounded-full bg-lime"
                        style={{ width: `${Math.max(3, (entry.distanceMeters / longest) * 100)}%` }}
                      />
                    </View>
                  </View>
                );
              })}
            </View>
          )}
          {top.length > 0 && (
            <Section title={t("stats.topPlaces")}>
              {top.map((place) => (
                <Row
                  key={place.id}
                  testID={`history-top-place-${place.id}`}
                  icon={<Text className="text-2xl">{placeEmoji(place)}</Text>}
                  label={place.name}
                  hint={t("places:row.visits", { count: place.myVisitCount })}
                  onPress={() => openPath(`/places/${place.id}`)}
                />
              ))}
            </Section>
          )}
        </>
      )}
    </View>
  );
}

function Tile({ emoji, value, label }: { emoji: string; value: number; label: string }) {
  const { colors } = useTheme();
  return (
    <View
      className="min-w-[29%] flex-1 items-center rounded-2xl bg-surface p-3"
      style={sticker(colors, "sm")}
    >
      <Text className="text-xl">{emoji}</Text>
      <Text className="text-xl font-black text-ink">{value.toLocaleString()}</Text>
      <Text className="text-[11px] font-bold text-ink-faint">{label}</Text>
    </View>
  );
}
