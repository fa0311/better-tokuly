import type { moments } from "@tokuly/api";
import { monthOf, monthOfDay } from "@tokuly/core/history";
import { Image } from "expo-image";
import { router } from "expo-router";
import { ChevronLeft, ChevronRight } from "lucide-react-native";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Button } from "#/components/button";
import { IconButton } from "#/components/icon-button";
import { EmptyState, ErrorNote, Page, Spinner } from "#/components/page";
import { mediaUrl } from "#/lib/config";
import { useTheme } from "#/lib/theme";
import { useMomentToday } from "./data";
import { useMomentYear } from "./year-data";

const FIRST_SUPPORTED_YEAR = 1970;

function utcDate(year: number, month = 0): Date {
  const date = new Date(0);
  date.setUTCFullYear(year, month, 1);
  date.setUTCHours(0, 0, 0, 0);
  return date;
}

export function MomentYearRecapScreen() {
  const { t, i18n } = useTranslation("moments");
  const { colors } = useTheme();
  const today = useMomentToday();
  const currentMonth = today.data?.day?.date
    ? monthOfDay(today.data.day.date)
    : monthOf(new Date());
  const [year, setYear] = useState(currentMonth.year);

  useEffect(() => {
    setYear(currentMonth.year);
  }, [currentMonth.year]);

  const monthCount =
    !today.data || today.isError
      ? 0
      : year < currentMonth.year
        ? 12
        : year === currentMonth.year
          ? currentMonth.month
          : 0;
  const months = useMomentYear(year, monthCount);
  const allPending = today.isPending || months.some((month) => month.isPending);
  const error = today.isError || months.some((month) => month.isError);
  const responses = months.flatMap((month) => (month.data ? [month.data] : []));
  const posted = responses.reduce((total, month) => total + month.postedCount, 0);
  const onTime = responses.reduce((total, month) => total + month.onTimeCount, 0);
  const activeMonths = responses.filter((month) => month.postedCount > 0).length;
  const title = new Intl.DateTimeFormat(i18n.language, {
    year: "numeric",
    timeZone: "UTC",
  }).format(utcDate(year));

  if (allPending) {
    return (
      <Page testID="moment-year-recap">
        <Spinner />
      </Page>
    );
  }

  if (error) {
    return (
      <Page testID="moment-year-recap">
        <ErrorNote>{t("recap.loadError")}</ErrorNote>
        <Button
          variant="secondary"
          onPress={() => {
            void today.refetch();
            for (const query of months) void query.refetch();
          }}
        >
          {t("composer.retry")}
        </Button>
      </Page>
    );
  }

  return (
    <Page testID="moment-year-recap">
      <View className="flex-row items-center justify-between rounded-card bg-surface p-3">
        <IconButton
          testID="moment-recap-previous-year"
          label={t("recap.previousYear")}
          icon={<ChevronLeft size={20} color={colors.ink} />}
          disabled={year <= FIRST_SUPPORTED_YEAR}
          onPress={() => setYear((value) => Math.max(value - 1, FIRST_SUPPORTED_YEAR))}
        />
        <Text accessibilityLiveRegion="polite" className="text-lg font-black text-ink">
          {title}
        </Text>
        <View className={year >= currentMonth.year ? "opacity-30" : ""}>
          <IconButton
            testID="moment-recap-next-year"
            label={t("recap.nextYear")}
            icon={<ChevronRight size={20} color={colors.ink} />}
            disabled={year >= currentMonth.year}
            onPress={() => setYear((value) => Math.min(value + 1, currentMonth.year))}
          />
        </View>
      </View>

      <View className="flex-row gap-2">
        <SummaryStat label={t("recap.posted")} value={posted} />
        <SummaryStat label={t("recap.onTime")} value={onTime} />
        <SummaryStat label={t("recap.activeMonths")} value={activeMonths} />
      </View>

      {posted === 0 && (
        <EmptyState emoji="📷" title={t("recap.empty.title")} body={t("recap.empty.body")} />
      )}

      <View className="flex-row flex-wrap justify-between gap-y-3">
        {Array.from({ length: 12 }, (_, index) => {
          const month = index + 1;
          const response = months[month - 1]?.data;
          const available = month <= monthCount;
          const label = new Intl.DateTimeFormat(i18n.language, {
            month: "long",
            timeZone: "UTC",
          }).format(utcDate(year, month - 1));
          const accessibilityLabel = available
            ? t("recap.monthSummary", {
                month: label,
                posted: response?.postedCount ?? 0,
                onTime: response?.onTimeCount ?? 0,
              })
            : t("recap.upcomingMonth", { month: label });

          return (
            <MonthTile
              key={month}
              year={year}
              month={month}
              label={label}
              days={response?.days ?? []}
              count={response?.postedCount ?? 0}
              available={available}
              accessibilityLabel={accessibilityLabel}
            />
          );
        })}
      </View>
    </Page>
  );
}

function SummaryStat({ label, value }: { label: string; value: number }) {
  return (
    <View className="min-w-0 flex-1 items-center gap-1 rounded-card bg-surface p-3">
      <Text className="text-xl font-black text-ink">{value}</Text>
      <Text numberOfLines={1} className="text-center text-xs font-bold text-ink-soft">
        {label}
      </Text>
    </View>
  );
}

function MonthTile({
  year,
  month,
  label,
  days,
  count,
  available,
  accessibilityLabel,
}: {
  year: number;
  month: number;
  label: string;
  days: moments.CalendarDay[];
  count: number;
  available: boolean;
  accessibilityLabel: string;
}) {
  const { t } = useTranslation("moments");
  const representatives = sampleDays(days, 4);
  return (
    <Pressable
      testID={`moment-recap-month-${year}-${month}`}
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel}
      accessibilityState={{ disabled: !available }}
      disabled={!available}
      onPress={() =>
        router.push({
          pathname: "/moments/calendar",
          params: { year: String(year), month: String(month) },
        })
      }
      className={`aspect-square w-[31%] gap-1.5 rounded-card bg-surface p-2 ${available ? "" : "opacity-40"}`}
    >
      <View className="flex-row items-center justify-between gap-1">
        <Text numberOfLines={1} className="min-w-0 flex-1 text-xs font-black text-ink">
          {label}
        </Text>
        <Text className="text-[10px] font-bold text-ink-soft">
          {available ? count : t("recap.soon")}
        </Text>
      </View>
      <View className="flex-1 flex-row flex-wrap gap-1">
        {representatives.map((day) => (
          <DayThumbnail key={day.date} day={day} />
        ))}
      </View>
    </Pressable>
  );
}

function DayThumbnail({ day }: { day: moments.CalendarDay }) {
  const image = day.back;
  const url = image?.url ? mediaUrl(image.url) : undefined;
  return (
    <View
      className="aspect-square w-[48%] overflow-hidden rounded-sm bg-surface-2"
      style={{ backgroundColor: image?.dominantColor || undefined }}
    >
      {url ? (
        <Image
          source={{ uri: url }}
          placeholder={image?.thumbhash ? { thumbhash: image.thumbhash } : undefined}
          contentFit="cover"
          style={{ width: "100%", height: "100%" }}
        />
      ) : null}
    </View>
  );
}

function sampleDays(days: moments.CalendarDay[], count: number): moments.CalendarDay[] {
  const ordered = [...days].sort((left, right) => left.date.localeCompare(right.date));
  if (ordered.length <= count) return ordered;
  return Array.from({ length: count }, (_, index) => {
    const selected = Math.round((index * (ordered.length - 1)) / (count - 1));
    return ordered[selected]!;
  });
}
