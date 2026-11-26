import { addMonths, formatMonth, monthGrid, monthOf, weekdayLabels } from "@tokuly/core/plans";
import { ChevronLeft, ChevronRight } from "lucide-react-native";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Modal, Pressable, ScrollView, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { IconButton } from "#/components/icon-button";
import { useTheme } from "#/lib/theme";

/** A sheet rising from the bottom with a title; a tap outside closes it. */
function Sheet({
  visible,
  title,
  testID,
  onClose,
  children,
}: {
  visible: boolean;
  title: string;
  testID: string;
  onClose(): void;
  children: ReactNode;
}) {
  const insets = useSafeAreaInsets();
  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onClose}>
      <Pressable accessibilityRole="none" className="flex-1 bg-black/40" onPress={onClose} />
      <View
        testID={testID}
        className="gap-3 rounded-t-3xl bg-surface px-4 pt-4"
        style={{ paddingBottom: insets.bottom + 16 }}
      >
        <Text className="text-center font-black text-ink-soft">{title}</Text>
        {children}
      </View>
    </Modal>
  );
}

/**
 * A day on a month page, Sunday first (the web's calendar): today is
 * ringed, the chosen day filled. Days are `YYYY-MM-DD` on the account's
 * calendar.
 */
export function DaySheet({
  visible,
  title,
  value,
  today,
  onChange,
  onClose,
}: {
  visible: boolean;
  title: string;
  value: string;
  today: string;
  onChange(day: string): void;
  onClose(): void;
}) {
  const { t, i18n } = useTranslation("plans");
  const { colors } = useTheme();
  const [month, setMonth] = useState(() => monthOf(value || today));
  return (
    <Sheet visible={visible} title={title} testID="plan-day-sheet" onClose={onClose}>
      <View className="flex-row items-center">
        <IconButton
          testID="plan-day-prev"
          label={t("calendar.prev")}
          icon={<ChevronLeft size={22} color={colors.ink} />}
          onPress={() => setMonth(addMonths(month, -1))}
        />
        <Text testID="plan-day-month" className="flex-1 text-center text-lg font-black text-ink">
          {formatMonth(month, i18n.language)}
        </Text>
        <IconButton
          testID="plan-day-next"
          label={t("calendar.next")}
          icon={<ChevronRight size={22} color={colors.ink} />}
          onPress={() => setMonth(addMonths(month, 1))}
        />
      </View>
      <View className="flex-row">
        {weekdayLabels(i18n.language).map((label, index) => (
          <Text
            key={`${index}-${label}`}
            className="flex-1 text-center text-xs font-black text-ink-soft"
          >
            {label}
          </Text>
        ))}
      </View>
      {monthGrid(month).map((week) => (
        <View key={week[0]} className="flex-row">
          {week.map((day) => {
            const chosen = day === value;
            const inMonth = monthOf(day) === month;
            return (
              <Pressable
                key={day}
                testID={`plan-day-${day}`}
                accessibilityRole="button"
                accessibilityState={{ selected: chosen }}
                onPress={() => {
                  onChange(day);
                  onClose();
                }}
                className="h-11 flex-1 items-center justify-center"
              >
                <View
                  className={`size-10 items-center justify-center rounded-full ${chosen ? "bg-brand" : ""} ${day === today && !chosen ? "border-2 border-brand" : ""}`}
                >
                  <Text
                    className={`font-black ${chosen ? "text-on-brand" : inMonth ? "text-ink" : "text-ink-faint"}`}
                  >
                    {Number(day.slice(8))}
                  </Text>
                </View>
              </Pressable>
            );
          })}
        </View>
      ))}
    </Sheet>
  );
}

const HOURS = Array.from({ length: 24 }, (_, hour) => String(hour).padStart(2, "0"));
const MINUTES = Array.from({ length: 12 }, (_, step) => String(step * 5).padStart(2, "0"));

/** A time of day (`HH:MM`) as an hour and a minute in steps of five. */
export function TimeSheet({
  visible,
  title,
  value,
  onChange,
  onClose,
}: {
  visible: boolean;
  title: string;
  value: string;
  onChange(time: string): void;
  onClose(): void;
}) {
  const { t } = useTranslation("mobile");
  const [hour = "", minute = ""] = value ? value.split(":") : [];
  const set = (nextHour: string, nextMinute: string) => onChange(`${nextHour}:${nextMinute}`);
  return (
    <Sheet visible={visible} title={title} testID="plan-time-sheet" onClose={onClose}>
      <Text className="px-1 text-sm font-black text-ink-soft">{t("plans.hour")}</Text>
      <Grid
        items={HOURS}
        value={hour}
        testID="plan-hour"
        onPick={(picked) => set(picked, minute || "00")}
      />
      <Text className="px-1 text-sm font-black text-ink-soft">{t("plans.minute")}</Text>
      <Grid
        items={MINUTES}
        value={minute}
        testID="plan-minute"
        onPick={(picked) => {
          set(hour || "18", picked);
          onClose();
        }}
      />
    </Sheet>
  );
}

function Grid({
  items,
  value,
  testID,
  onPick,
}: {
  items: string[];
  value: string;
  testID: string;
  onPick(item: string): void;
}) {
  return (
    <ScrollView contentContainerClassName="flex-row flex-wrap gap-2">
      {items.map((item) => {
        const chosen = item === value;
        return (
          <Pressable
            key={item}
            testID={`${testID}-${item}`}
            accessibilityRole="button"
            accessibilityState={{ selected: chosen }}
            onPress={() => onPick(item)}
            className={`h-10 w-[14.5%] items-center justify-center rounded-xl ${chosen ? "bg-brand" : "bg-surface-2"}`}
          >
            <Text className={`font-black ${chosen ? "text-on-brand" : "text-ink"}`}>{item}</Text>
          </Pressable>
        );
      })}
    </ScrollView>
  );
}
