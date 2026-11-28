import { type Month, monthGrid, monthOf, monthOfDay, shiftMonth } from "@tokuly/core/history";
import { Image } from "expo-image";
import { router, useLocalSearchParams } from "expo-router";
import { ChevronLeft, ChevronRight } from "lucide-react-native";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Button } from "#/components/button";
import { IconButton } from "#/components/icon-button";
import { EmptyState, ErrorNote, Page, Spinner } from "#/components/page";
import { mediaUrl } from "#/lib/config";
import { errorMessage } from "#/lib/errors";
import { sticker, useTheme } from "#/lib/theme";
import { useMomentCalendar, useMomentToday } from "./data";

function dateLabel(date: string, language: string): string {
  const [year, month, day] = date.split("-").map(Number);
  return new Intl.DateTimeFormat(language, { dateStyle: "long", timeZone: "UTC" }).format(
    new Date(Date.UTC(year ?? 2000, (month ?? 1) - 1, day ?? 1)),
  );
}

export function MomentCalendarScreen() {
  const { t, i18n } = useTranslation("moments");
  const { colors } = useTheme();
  const today = useMomentToday();
  const params = useLocalSearchParams<{ year?: string; month?: string }>();
  const routeYear = Number(params.year);
  const routeMonth = Number(params.month);
  const hasRouteMonth =
    Number.isInteger(routeYear) &&
    Number.isInteger(routeMonth) &&
    routeYear >= 1970 &&
    routeYear <= 9999 &&
    routeMonth >= 1 &&
    routeMonth <= 12;
  const [month, setMonth] = useState<Month>(() =>
    hasRouteMonth ? { year: routeYear, month: routeMonth } : monthOf(new Date()),
  );
  const todayDate = today.data?.day?.date;
  const currentMonth = todayDate ? monthOfDay(todayDate) : monthOf(new Date());

  useEffect(() => {
    if (hasRouteMonth) setMonth({ year: routeYear, month: routeMonth });
    else if (todayDate) setMonth(monthOfDay(todayDate));
  }, [hasRouteMonth, routeMonth, routeYear, todayDate]);

  const query = useMomentCalendar(month, !today.isPending);
  const entries = query.data?.days ?? [];
  const days = new Map(entries.map((entry) => [entry.date, entry]));
  const isCurrent = month.year === currentMonth.year && month.month === currentMonth.month;
  const title = new Intl.DateTimeFormat(i18n.language, {
    year: "numeric",
    month: "long",
    timeZone: "UTC",
  }).format(new Date(Date.UTC(month.year, month.month - 1, 1)));
  const weekdays = Array.from({ length: 7 }, (_, index) =>
    new Intl.DateTimeFormat(i18n.language, { weekday: "narrow", timeZone: "UTC" }).format(
      new Date(Date.UTC(2023, 0, index + 1)),
    ),
  );

  if (today.isPending) {
    return (
      <Page testID="moment-calendar">
        <Spinner />
      </Page>
    );
  }

  return (
    <Page testID="moment-calendar">
      {today.isError && (
        <ErrorNote>{errorMessage(today.error, { namespace: "moments" })}</ErrorNote>
      )}
      <View className="gap-2 rounded-card bg-surface p-3" style={sticker(colors)}>
        <View className="flex-row items-center justify-between">
          <IconButton
            testID="moment-calendar-previous"
            label={t("calendar.previous")}
            icon={<ChevronLeft size={20} color={colors.ink} />}
            onPress={() => setMonth(shiftMonth(month, -1))}
          />
          <Text accessibilityLiveRegion="polite" className="text-lg font-black text-ink">
            {title}
          </Text>
          <View className={isCurrent ? "opacity-30" : ""}>
            <IconButton
              testID="moment-calendar-next"
              label={t("calendar.next")}
              icon={<ChevronRight size={20} color={colors.ink} />}
              onPress={() => !isCurrent && setMonth(shiftMonth(month, 1))}
            />
          </View>
        </View>

        <View className="flex-row">
          {weekdays.map((weekday, index) => (
            <Text
              key={index}
              className={`flex-1 py-1 text-center text-xs font-black ${index === 0 ? "text-pink" : index === 6 ? "text-sky" : "text-ink-faint"}`}
            >
              {weekday}
            </Text>
          ))}
        </View>

        {monthGrid(month).map((week, row) => (
          <View key={row} className="flex-row">
            {week.map((date, column) => {
              if (!date) return <View key={column} className="aspect-square flex-1" />;
              const entry = days.get(date);
              const momentId = entry?.momentId || undefined;
              const labelDate = dateLabel(date, i18n.language);
              const late = Number(entry?.lateSeconds ?? 0) > 0;
              const imageUrl = entry?.back?.url ? mediaUrl(entry.back.url) : undefined;
              const accessibilityLabel = momentId
                ? t("calendar.day", { date: labelDate })
                : t("calendar.emptyDay", { date: labelDate });

              return (
                <View key={date} className="aspect-square flex-1 p-0.5">
                  <Pressable
                    testID={`moment-calendar-day-${date}`}
                    accessibilityRole="button"
                    accessibilityLabel={accessibilityLabel}
                    accessibilityState={{ disabled: !momentId }}
                    disabled={!momentId}
                    onPress={() =>
                      momentId &&
                      router.push({
                        pathname: "/moments/friends/[momentId]",
                        params: { momentId },
                      })
                    }
                    className="flex-1 items-center justify-center overflow-hidden rounded-xl"
                    style={{
                      backgroundColor: entry?.back?.dominantColor || colors["surface-2"],
                    }}
                  >
                    {imageUrl && (
                      <Image
                        source={{ uri: imageUrl }}
                        placeholder={
                          entry?.back?.thumbhash ? { thumbhash: entry.back.thumbhash } : undefined
                        }
                        contentFit="cover"
                        style={{
                          position: "absolute",
                          top: 0,
                          right: 0,
                          bottom: 0,
                          left: 0,
                          width: "100%",
                          height: "100%",
                        }}
                      />
                    )}
                    <Text className="rounded-full bg-surface px-1.5 text-xs font-black text-ink">
                      {Number(date.slice(8))}
                    </Text>
                    {late && (
                      <Text className="absolute right-1 bottom-1 rounded-full bg-surface px-1 text-[9px] font-black text-brand">
                        {t("calendar.lateMark")}
                      </Text>
                    )}
                  </Pressable>
                </View>
              );
            })}
          </View>
        ))}
      </View>

      {query.isPending ? (
        <Spinner />
      ) : query.isError ? (
        <View className="gap-2">
          <ErrorNote>{errorMessage(query.error, { namespace: "moments" })}</ErrorNote>
          <Button variant="secondary" onPress={() => void query.refetch()}>
            {t("composer.retry")}
          </Button>
        </View>
      ) : (
        <View className="gap-2">
          {entries.length === 0 && (
            <EmptyState
              emoji="🗓️"
              title={t("calendar.emptyState.title")}
              body={t("calendar.emptyState.body")}
            />
          )}
          <Text
            testID="moment-calendar-totals"
            className="px-1 text-center text-sm font-bold text-ink-soft"
          >
            {t("calendar.summary", {
              posted: query.data?.postedCount ?? 0,
              onTime: query.data?.onTimeCount ?? 0,
            })}
          </Text>
          <Button variant="secondary" size="sm" onPress={() => router.push("/moments/recap")}>
            {t("calendar.recap")}
          </Button>
        </View>
      )}
    </Page>
  );
}
