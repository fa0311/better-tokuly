import { formatDistance } from "@tokuly/core/format";
import { type Month, monthGrid, monthOfDay, shiftMonth } from "@tokuly/core/history";
import { router } from "expo-router";
import { ChevronLeft, ChevronRight } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { IconButton } from "#/components/icon-button";
import { ErrorNote, Spinner } from "#/components/page";
import { errorMessage } from "#/lib/errors";
import { sticker, useTheme } from "#/lib/theme";
import { NS, useAccountToday, useMonth } from "./data";
import { dayTitle } from "./parts";

/** The month at a glance on the account's calendar: active days stand out, each leads to its day. */
export function HistoryCalendar() {
  const { t, i18n } = useTranslation(NS);
  const { colors } = useTheme();
  const today = useAccountToday();
  const thisMonth = monthOfDay(today);
  const [month, setMonth] = useState<Month>(thisMonth);
  const isCurrent = month.year === thisMonth.year && month.month === thisMonth.month;
  const query = useMonth(month);
  const days = new Map((query.data?.days ?? []).map((day) => [day.date, day]));
  const title = new Date(month.year, month.month - 1, 1).toLocaleDateString(i18n.language, {
    year: "numeric",
    month: "long",
  });
  // 2023-01-01 was a Sunday: the weekday names in the person's language.
  const weekdays = Array.from({ length: 7 }, (_, index) =>
    new Date(2023, 0, 1 + index).toLocaleDateString(i18n.language, { weekday: "narrow" }),
  );
  const active = [...days.values()];
  const distance = active.reduce((sum, day) => sum + day.distanceMeters, 0);
  const visits = active.reduce((sum, day) => sum + day.visitCount, 0);

  return (
    <View className="gap-3">
      <View
        testID="history-calendar"
        className="rounded-card bg-surface p-3"
        style={sticker(colors)}
      >
        <View className="flex-row items-center justify-between">
          <IconButton
            testID="history-month-previous"
            label={t("calendar.previous")}
            icon={<ChevronLeft size={20} color={colors.ink} />}
            onPress={() => setMonth(shiftMonth(month, -1))}
          />
          <Text accessibilityLiveRegion="polite" className="text-lg font-black text-ink">
            {title}
          </Text>
          <View className={isCurrent ? "opacity-30" : ""}>
            <IconButton
              testID="history-month-next"
              label={t("calendar.next")}
              icon={<ChevronRight size={20} color={colors.ink} />}
              onPress={() => !isCurrent && setMonth(shiftMonth(month, 1))}
            />
          </View>
        </View>
        <View className="mt-1 flex-row">
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
              const day = days.get(date);
              const future = date > today;
              const fill = day ? (day.newCellCount > 0 ? "bg-lime" : "bg-brand-soft") : "";
              return (
                <View key={date} className="aspect-square flex-1 p-0.5">
                  <Pressable
                    testID={`history-day-${date}`}
                    accessibilityRole="button"
                    accessibilityState={{ disabled: future }}
                    accessibilityLabel={
                      day
                        ? t("calendar.activeDay", {
                            date: dayTitle(date, i18n.language),
                            distance: formatDistance(day.distanceMeters),
                            count: day.visitCount,
                          })
                        : dayTitle(date, i18n.language)
                    }
                    disabled={future}
                    onPress={() => router.push(`/history/${date}`)}
                    className={`flex-1 items-center justify-center rounded-xl ${fill} ${date === today ? "border-2 border-dashed border-ink-faint" : ""} ${future ? "opacity-40" : ""}`}
                    style={day ? sticker(colors, "sm") : undefined}
                  >
                    <Text
                      className={`text-sm ${day ? "font-black" : "font-bold"} ${day?.newCellCount ? "text-[#17122b]" : day ? "text-brand-strong" : "text-ink-soft"}`}
                    >
                      {Number(date.slice(8))}
                    </Text>
                    {day?.hasMemo && (
                      <Text className="absolute right-0.5 bottom-0 text-[8px]">📝</Text>
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
        <ErrorNote>{errorMessage(query.error, { namespace: NS })}</ErrorNote>
      ) : active.length === 0 ? (
        <Text className="px-1 text-center text-sm text-ink-soft">{t("calendar.emptyMonth")}</Text>
      ) : (
        <Text
          testID="history-month-totals"
          className="px-1 text-center text-sm font-bold text-ink-soft"
        >
          {t("calendar.totals", {
            days: active.length,
            distance: formatDistance(distance),
            visits,
          })}
        </Text>
      )}
    </View>
  );
}
