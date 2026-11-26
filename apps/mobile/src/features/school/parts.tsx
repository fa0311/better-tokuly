import { school, toMillis } from "@tokuly/api";
import {
  classColor,
  classStatusKey,
  classWhen,
  clock,
  currentClass,
  dayLabel,
  inkOn,
  takesPlace,
  untilText,
} from "@tokuly/core/school";
import { router } from "expo-router";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Spinner } from "#/components/page";
import { INK_ON_BRIGHT, sticker, useTheme } from "#/lib/theme";
import { NS, useAccountNow, useClassNow } from "./data";

/** A heading over a block of the school screen, with what it leads to on its right. */
export function Block({
  title,
  action,
  children,
}: {
  title: string;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <View className="gap-2">
      <View className="min-h-8 flex-row items-center justify-between px-1">
        <Text className="text-lg font-black text-ink">{title}</Text>
        {action}
      </View>
      {children}
    </View>
  );
}

/** A small coloured pill: the subject's emoji and name on its colour, struck through when off. */
export function SubjectPill({
  subject,
  color,
  off,
}: {
  subject: Pick<school.Subject, "name" | "emoji" | "color"> | undefined;
  color?: string;
  off?: boolean;
}) {
  const { colors } = useTheme();
  const fill = color || subject?.color || "#ddd6ee";
  if (!subject) return null;
  return (
    <View
      className={`h-6 max-w-full flex-row items-center gap-1 rounded-full border-2 px-2 ${off ? "opacity-50" : ""}`}
      style={{ backgroundColor: fill, borderColor: colors.ink }}
    >
      {subject.emoji ? <Text className="text-xs">{subject.emoji}</Text> : null}
      <Text
        numberOfLines={1}
        className={`shrink text-xs font-black ${off ? "line-through" : ""}`}
        style={{ color: inkOn(fill) }}
      >
        {subject.name}
      </Text>
    </View>
  );
}

/**
 * The class in progress and the next one, like a home-screen widget: the
 * subject in its colour, the room, and how long until the bell.
 */
export function ClassNowCard() {
  const { t, i18n } = useTranslation(NS);
  const { colors } = useTheme();
  const { data, isPending } = useClassNow();
  const { ms: now, date: today } = useAccountNow(15_000);
  if (isPending) return <Spinner />;
  const current = currentClass(data, now);
  const next = data?.next;
  const fill = current ? classColor(current) : colors.surface;
  const ink = current ? inkOn(fill) : colors.ink;
  const done = current
    ? Math.min(
        1,
        Math.max(
          0,
          (now - toMillis(current.startsAt)) /
            Math.max(1, toMillis(current.endsAt) - toMillis(current.startsAt)),
        ),
      )
    : 0;
  return (
    <View
      testID="school-now"
      className="overflow-hidden rounded-card bg-surface"
      style={sticker(colors)}
    >
      {current ? (
        <View className="gap-1 p-4" style={{ backgroundColor: fill }}>
          <View className="flex-row items-center gap-2">
            <Text className="overflow-hidden rounded-full bg-live px-2 py-0.5 text-xs font-black text-white">
              {t("now.current")}
            </Text>
            <Text className="text-xs font-black" style={{ color: ink }}>
              {t("day.period", { period: current.period })} · 〜{clock(current.endMinute)}
            </Text>
          </View>
          <Text numberOfLines={1} className="text-3xl font-black" style={{ color: ink }}>
            {current.subject?.emoji} {current.subject?.name}
          </Text>
          <View className="flex-row items-center gap-2">
            <Text numberOfLines={1} className="flex-1 text-sm font-bold" style={{ color: ink }}>
              {[current.room, current.teacher].filter(Boolean).join(" · ")}
            </Text>
            <Text className="text-sm font-bold" style={{ color: ink }}>
              {untilText(i18n, toMillis(current.endsAt) - now)}
            </Text>
          </View>
          <View
            className="mt-2 h-2.5 overflow-hidden rounded-full border-2"
            style={{ borderColor: ink }}
          >
            <View className="h-full" style={{ width: `${done * 100}%`, backgroundColor: ink }} />
          </View>
        </View>
      ) : (
        <View className="gap-1 p-4">
          <Text className="text-xs font-black text-ink-soft">{t("now.title")}</Text>
          <Text className="text-2xl font-black text-ink">
            {next ? (next.date === today ? t("now.break") : t("now.doneToday")) : t("now.nothing")}
          </Text>
        </View>
      )}
      {next && (
        <View
          className="flex-row items-center gap-3 border-t-2 px-4 py-3"
          style={{ borderColor: colors.ink }}
        >
          <View
            className="size-3 rounded-full border-2"
            style={{ backgroundColor: classColor(next), borderColor: colors.ink }}
          />
          <View className="flex-1">
            <Text className="text-xs font-black text-ink-soft">{t("now.next")}</Text>
            <Text numberOfLines={1} className="font-black text-ink">
              {next.subject?.emoji} {next.subject?.name}
              {next.room ? `  ${next.room}` : ""}
            </Text>
          </View>
          <View className="items-end">
            <Text className="text-sm font-bold text-ink">{classWhen(i18n, next, today)}</Text>
            {next.date === today && (
              <Text className="text-xs text-ink-soft">
                {untilText(i18n, toMillis(next.startsAt) - now)}
              </Text>
            )}
          </View>
        </View>
      )}
    </View>
  );
}

/** One resolved day, read only: every period with the class that takes place in it, and what changed. */
export function DayClasses({ day, nowMs }: { day: school.TimetableDay; nowMs?: number }) {
  const { t, i18n } = useTranslation(NS);
  const { colors } = useTheme();
  if (day.kind !== school.DayKind.SCHOOL_DAY) {
    const holiday = day.kind === school.DayKind.HOLIDAY;
    const outOfTerm = day.kind === school.DayKind.OUT_OF_TERM;
    return (
      <View
        testID="school-day-off"
        className="flex-row items-center gap-3 rounded-card bg-surface-2 px-4 py-5"
      >
        <Text className="text-3xl">{holiday ? "🎌" : outOfTerm ? "🏝️" : "🛋️"}</Text>
        <View className="flex-1">
          <Text className="font-black text-ink">
            {holiday
              ? day.label || t("day.holiday")
              : outOfTerm
                ? t("day.outOfTerm")
                : t("day.dayOff")}
          </Text>
          <Text className="text-sm text-ink-soft">{t("day.noClasses")}</Text>
        </View>
      </View>
    );
  }
  return (
    <View
      testID="school-day"
      accessibilityLabel={t("day.classesOf", { date: dayLabel(day.date, i18n.language) })}
      className="gap-1.5"
    >
      {day.periods.map((period) => {
        const classes = day.classes.filter((item) => item.period === period.number);
        const live =
          nowMs !== undefined &&
          classes.some(
            (item) =>
              takesPlace(item) && toMillis(item.startsAt) <= nowMs && nowMs < toMillis(item.endsAt),
          );
        return (
          <View
            key={period.number}
            className={`flex-row items-center gap-3 rounded-2xl px-3 py-2.5 ${live ? "bg-surface" : "bg-surface-2"}`}
            style={live ? sticker(colors, "sm") : undefined}
          >
            <View className="w-16 items-center">
              <Text numberOfLines={1} adjustsFontSizeToFit className="font-black text-ink">
                {period.label || t("day.period", { period: period.number })}
              </Text>
              <Text className="text-[11px] font-bold text-ink-faint">
                {clock(period.startMinute)}
              </Text>
            </View>
            <View className="flex-1 gap-1">
              {classes.length === 0 ? (
                <Text className="text-sm font-bold text-ink-faint">{t("day.free")}</Text>
              ) : (
                classes.map((item) => (
                  <ClassLine key={`${item.entryId}-${item.changeId}-${item.status}`} item={item} />
                ))
              )}
            </View>
            {live && (
              <Text className="overflow-hidden rounded-full bg-live px-2 py-0.5 text-[11px] font-black text-white">
                {t("day.now")}
              </Text>
            )}
          </View>
        );
      })}
    </View>
  );
}

function ClassLine({ item }: { item: school.ScheduledClass }) {
  const { t, i18n } = useTranslation(NS);
  const off = !takesPlace(item);
  const key = classStatusKey(item.status);
  const away = item.status === school.ClassStatus.MOVED_AWAY;
  const moved = away || item.status === school.ClassStatus.MOVED_HERE;
  const where = moved
    ? t(away ? "day.movedTo" : "day.movedFrom", {
        date: dayLabel(item.otherDate, i18n.language),
        period: item.otherPeriod,
      })
    : undefined;
  const detail = [where, item.room, item.teacher, item.memo].filter(Boolean).join(" · ");
  return (
    <View className="flex-row flex-wrap items-center gap-x-2 gap-y-0.5">
      <SubjectPill subject={item.subject} color={classColor(item)} off={off} />
      {key && (
        <Text
          className={`overflow-hidden rounded-full px-2 py-0.5 text-[11px] font-black ${off ? "bg-danger text-white" : "bg-sun"}`}
          style={off ? undefined : { color: INK_ON_BRIGHT }}
        >
          {t(`day.status.${key}`)}
        </Text>
      )}
      {detail ? (
        <Text numberOfLines={1} className="shrink text-xs font-bold text-ink-soft">
          {detail}
        </Text>
      ) : null}
    </View>
  );
}

/** "ぜんぶ見る ›": leads to a screen of the area. */
export function SeeAll({ href, testID }: { href: string; testID?: string }) {
  const { t } = useTranslation(NS);
  return (
    <Pressable testID={testID} accessibilityRole="link" onPress={() => router.push(href as never)}>
      <Text className="text-sm font-bold text-brand">{t("today.seeAll")} ›</Text>
    </Pressable>
  );
}
