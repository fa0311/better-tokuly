import { type moments, toDate } from "@tokuly/api";
import { monthOfDay } from "@tokuly/core/history";
import { Image } from "expo-image";
import { router } from "expo-router";
import { ChevronLeft, ChevronRight } from "lucide-react-native";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ActivityIndicator, Pressable, Text, View } from "react-native";
import { Button } from "#/components/button";
import { IconButton } from "#/components/icon-button";
import { EmptyState, ErrorNote, Page, Spinner } from "#/components/page";
import { useMe } from "#/features/session/store";
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
  const me = useMe();
  const today = useMomentToday();
  const currentMonth = today.data?.day?.date ? monthOfDay(today.data.day.date) : undefined;
  const currentYear = currentMonth?.year;
  const accountCreatedYear = toDate(me?.createdAt)?.getUTCFullYear();
  const minimumYear =
    currentYear === undefined
      ? FIRST_SUPPORTED_YEAR
      : Math.min(accountCreatedYear ?? currentYear, currentYear);
  const [year, setYear] = useState<number | null>(null);
  const selectedYear = year ?? currentYear ?? accountCreatedYear ?? FIRST_SUPPORTED_YEAR;

  useEffect(() => {
    if (currentYear === undefined) return;
    setYear((value) =>
      value === null ? currentYear : Math.max(minimumYear, Math.min(value, currentYear)),
    );
  }, [currentYear, minimumYear]);

  const monthCount =
    !currentMonth || !today.data || today.isError
      ? 0
      : selectedYear < currentMonth.year
        ? 12
        : selectedYear === currentMonth.year
          ? currentMonth.month
          : 0;
  const months = useMomentYear(selectedYear, monthCount);
  const monthsReady =
    months.length === monthCount &&
    months.every((month) => month.data !== undefined && !month.isPending && !month.isError);
  const monthError = months.some((month) => month.isError);
  const responses = months.flatMap((month) => (month.data ? [month.data] : []));
  const posted = monthsReady
    ? responses.reduce((total, month) => total + month.postedCount, 0)
    : null;
  const onTime = monthsReady
    ? responses.reduce((total, month) => total + month.onTimeCount, 0)
    : null;
  const activeMonths = monthsReady
    ? responses.filter((month) => month.postedCount > 0).length
    : null;
  const title = new Intl.DateTimeFormat(i18n.language, {
    year: "numeric",
    timeZone: "UTC",
  }).format(utcDate(selectedYear));

  if (today.isPending) {
    return (
      <Page testID="moment-year-recap">
        <Spinner />
      </Page>
    );
  }

  if (today.isError) {
    return (
      <Page testID="moment-year-recap">
        <ErrorNote>{t("recap.loadError")}</ErrorNote>
        <Button variant="secondary" onPress={() => void today.refetch()}>
          {t("composer.retry")}
        </Button>
      </Page>
    );
  }

  if (!currentMonth) {
    return (
      <Page testID="moment-year-recap">
        <EmptyState emoji="⏳" title={t("recap.notReady.title")} body={t("recap.notReady.body")} />
      </Page>
    );
  }

  return (
    <Page testID="moment-year-recap">
      {monthError && (
        <>
          <ErrorNote>{t("recap.loadError")}</ErrorNote>
          <Button
            variant="secondary"
            onPress={() => {
              for (const query of months) {
                if (query.isError) void query.refetch();
              }
            }}
          >
            {t("composer.retry")}
          </Button>
        </>
      )}
      <View className="flex-row items-center justify-between rounded-card bg-surface p-3">
        <IconButton
          testID="moment-recap-previous-year"
          label={t("recap.previousYear")}
          icon={<ChevronLeft size={20} color={colors.ink} />}
          disabled={selectedYear <= minimumYear}
          onPress={() => setYear((value) => Math.max((value ?? selectedYear) - 1, minimumYear))}
        />
        <Text accessibilityLiveRegion="polite" className="text-lg font-black text-ink">
          {title}
        </Text>
        <View className={selectedYear >= currentMonth.year ? "opacity-30" : ""}>
          <IconButton
            testID="moment-recap-next-year"
            label={t("recap.nextYear")}
            icon={<ChevronRight size={20} color={colors.ink} />}
            disabled={selectedYear >= currentMonth.year}
            onPress={() =>
              setYear((value) => Math.min((value ?? selectedYear) + 1, currentMonth.year))
            }
          />
        </View>
      </View>

      <View className="flex-row gap-2">
        <SummaryStat label={t("recap.posted")} value={posted} />
        <SummaryStat label={t("recap.onTime")} value={onTime} />
        <SummaryStat label={t("recap.activeMonths")} value={activeMonths} />
      </View>

      {monthsReady && posted === 0 && (
        <EmptyState emoji="📷" title={t("recap.empty.title")} body={t("recap.empty.body")} />
      )}

      <View className="flex-row flex-wrap justify-between gap-y-3">
        {Array.from({ length: 12 }, (_, index) => {
          const month = index + 1;
          const query = months[month - 1];
          const response = query?.data;
          const available = month <= monthCount;
          const loading = available && query?.isPending === true;
          const failed = available && query?.isError === true;
          const label = new Intl.DateTimeFormat(i18n.language, {
            month: "long",
            timeZone: "UTC",
          }).format(utcDate(selectedYear, month - 1));
          const accessibilityLabel = loading
            ? t("recap.loadingMonth", { month: label })
            : failed
              ? t("recap.monthLoadError", { month: label })
              : available
                ? t("recap.monthSummary", {
                    month: label,
                    posted: response?.postedCount ?? 0,
                    onTime: response?.onTimeCount ?? 0,
                  })
                : t("recap.upcomingMonth", { month: label });

          return (
            <MonthTile
              key={month}
              year={selectedYear}
              month={month}
              label={label}
              days={response?.days ?? []}
              count={response?.postedCount ?? 0}
              available={available}
              loading={loading}
              failed={failed}
              accessibilityLabel={accessibilityLabel}
            />
          );
        })}
      </View>
    </Page>
  );
}

function SummaryStat({ label, value }: { label: string; value: number | null }) {
  return (
    <View className="min-w-0 flex-1 items-center gap-1 rounded-card bg-surface p-3">
      <Text className="text-xl font-black text-ink">{value ?? "—"}</Text>
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
  loading,
  failed,
  accessibilityLabel,
}: {
  year: number;
  month: number;
  label: string;
  days: moments.CalendarDay[];
  count: number;
  available: boolean;
  loading: boolean;
  failed: boolean;
  accessibilityLabel: string;
}) {
  const { t } = useTranslation("moments");
  const { colors } = useTheme();
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
          {loading || failed ? "—" : available ? count : t("recap.soon")}
        </Text>
      </View>
      <View className="flex-1 flex-row flex-wrap gap-1">
        {loading ? (
          <View className="flex-1 items-center justify-center">
            <ActivityIndicator color={colors.brand} />
          </View>
        ) : (
          representatives.map((day) => <DayThumbnail key={day.date} day={day} />)
        )}
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
