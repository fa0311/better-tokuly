import { useQueryClient } from "@tanstack/react-query";
import { school } from "@tokuly/api";
import { addDays, dayLabel, mondayOf, parseYmd, takesPlace, weekdayOf } from "@tokuly/core/school";
import { router } from "expo-router";
import { ChevronLeft, ChevronRight } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, RefreshControl, ScrollView, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { IconButton } from "#/components/icon-button";
import { ErrorNote, Row, Section, Spinner } from "#/components/page";
import { webScreenHref } from "#/features/web/handoff";
import { errorMessage } from "#/lib/errors";
import { sticker, useTheme } from "#/lib/theme";
import { NS, useAccountNow, useDay, useWeek } from "./data";
import { DueList } from "./due";
import { Block, ClassNowCard, DayClasses, SeeAll } from "./parts";
import { StudyingFriends } from "./studying-friends";

/** What stays on the web for now: the timer, rooms, exams, making the timetable, friends' timetables, settings. */
const ON_WEB = [
  { key: "timer", path: "/school/study" },
  { key: "rooms", path: "/school/rooms" },
  { key: "exams", path: "/school/exams" },
  { key: "timetable", path: "/school/timetable" },
  { key: "friends", path: "/school/friends" },
  { key: "settings", path: "/school/settings" },
] as const;

/**
 * `/school`: the class that is on and the next one, the classes of any day
 * of any week (with that day's changes), what is due, and the friends who
 * are studying now.
 */
export function SchoolScreen() {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const [refreshing, setRefreshing] = useState(false);
  const refresh = async () => {
    setRefreshing(true);
    await queryClient.invalidateQueries({ queryKey: ["school"] });
    setRefreshing(false);
  };
  return (
    <SafeAreaView edges={["bottom"]} className="flex-1 bg-bg">
      <ScrollView
        testID="school"
        contentContainerClassName="gap-6 px-4 pt-4 pb-12"
        refreshControl={<RefreshControl refreshing={refreshing} onRefresh={refresh} />}
      >
        <ClassNowCard />
        <Classes />
        <Block
          title={t("due.title")}
          action={<SeeAll testID="school-assignments" href="/school/assignments" />}
        >
          <DueList limit={5} />
        </Block>
        <Block title={t("study.friendsTitle")}>
          <StudyingFriends />
        </Block>
        <Section lead={t("mobile:school.onWeb")}>
          {ON_WEB.map((item) => (
            <Row
              key={item.key}
              testID={`school-web-${item.key}`}
              label={t(`mobile:school.${item.key}`)}
              onPress={() => router.push(webScreenHref(item.path) as never)}
            />
          ))}
        </Section>
      </ScrollView>
    </SafeAreaView>
  );
}

/** The classes of the chosen day, under a strip of its week. */
function Classes() {
  const { t, i18n } = useTranslation(NS);
  const { colors } = useTheme();
  const { ms: now, date: today } = useAccountNow(30_000);
  // No day picked means "today", so the screen moves on with the date.
  const [picked, setPicked] = useState<string | undefined>(undefined);
  const date = picked ?? today;
  const setDate = (next: string) => setPicked(next === today ? undefined : next);
  const monday = mondayOf(date);
  const week = useWeek(monday, addDays(monday, 6));
  const day = useDay(date);
  return (
    <Block
      title={
        date === today
          ? t("today.classesToday")
          : t("today.classesOn", { date: dayLabel(date, i18n.language) })
      }
      action={
        date !== today && (
          <Pressable testID="school-back-today" onPress={() => setDate(today)}>
            <Text className="text-sm font-bold text-brand">{t("today.backToToday")}</Text>
          </Pressable>
        )
      }
    >
      <View className="flex-row items-center gap-1">
        <IconButton
          testID="school-week-previous"
          label={t("today.previousWeek")}
          icon={<ChevronLeft size={20} color={colors.ink} />}
          onPress={() => setDate(addDays(date, -7))}
        />
        <View className="flex-1 flex-row gap-1" accessibilityLabel={t("today.week")}>
          {Array.from({ length: 7 }, (_, offset) => {
            const value = addDays(monday, offset);
            const info = week.data?.find((item) => item.date === value);
            const count = info?.classes.filter(takesPlace).length ?? 0;
            const off = info !== undefined && info.kind !== school.DayKind.SCHOOL_DAY;
            const chosen = value === date;
            return (
              <Pressable
                key={value}
                testID={`school-date-${value}`}
                accessibilityRole="button"
                accessibilityState={{ selected: chosen }}
                accessibilityLabel={`${dayLabel(value, i18n.language)} ${info ? t("today.classCount", { count }) : ""}`}
                onPress={() => setDate(value)}
                className={`flex-1 items-center rounded-2xl py-1.5 ${chosen ? "bg-ink" : value === today ? "bg-brand-soft" : ""}`}
                style={chosen ? sticker(colors, "sm") : undefined}
              >
                <Text
                  className={`text-[11px] font-bold ${chosen ? "text-surface" : "text-ink-soft"}`}
                >
                  {t(`weekdays.short.${weekdayOf(value)}`)}
                </Text>
                <Text className={`font-black ${chosen ? "text-surface" : "text-ink"}`}>
                  {parseYmd(value).getDate()}
                </Text>
                <Text
                  className={`text-[10px] font-bold ${off ? "text-coral" : chosen ? "text-surface" : "text-ink-soft"}`}
                >
                  {info ? (off ? t("today.off") : count) : " "}
                </Text>
              </Pressable>
            );
          })}
        </View>
        <IconButton
          testID="school-week-next"
          label={t("today.nextWeek")}
          icon={<ChevronRight size={20} color={colors.ink} />}
          onPress={() => setDate(addDays(date, 7))}
        />
      </View>
      {day.isPending ? (
        <Spinner />
      ) : day.data ? (
        <DayClasses day={day.data} nowMs={date === today ? now : undefined} />
      ) : (
        <ErrorNote>{errorMessage(day.error, { namespace: NS })}</ErrorNote>
      )}
      {day.data?.kind === school.DayKind.OUT_OF_TERM && (
        <Pressable
          testID="school-setup-timetable"
          onPress={() => router.push(webScreenHref("/school/timetable") as never)}
        >
          <Text className="py-2 text-center text-sm font-bold text-brand">
            {t("today.setUpTimetable")}
          </Text>
        </Pressable>
      )}
      {day.data?.kind === school.DayKind.SCHOOL_DAY && (
        <Text className="px-1 text-xs text-ink-faint">{t("mobile:school.changesOnWeb")}</Text>
      )}
    </Block>
  );
}
