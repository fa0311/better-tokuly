import { useMutation, useQueryClient } from "@tanstack/react-query";
import { create, fromDate, geo } from "@tokuly/api";
import { DELETED_HISTORY_KEYS, dayRange, historyKeys, shiftDay } from "@tokuly/core/history";
import { router, Stack } from "expo-router";
import { ChevronLeft, ChevronRight } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Text, View } from "react-native";
import { Button } from "#/components/button";
import { SwitchRow } from "#/components/form-rows";
import { IconButton } from "#/components/icon-button";
import { ErrorNote, Page, Section } from "#/components/page";
import { Tabs } from "#/components/tabs";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import { asAccount, historyClient, NS, useAccountTimeZone, useAccountToday } from "./data";
import { dayTitle } from "./parts";

type Scope = "range" | "all";

/**
 * `/history/delete`: deleting history for good, as the API allows — one day
 * (`date`), a range of days, or everything; the footprints first uncovered
 * in it only when asked. Every cached answer about the history, the
 * footprints and the places is reset afterwards, so nothing deleted is
 * shown again from memory.
 */
export function DeleteHistoryScreen({ date }: { date?: string }) {
  const { t, i18n } = useTranslation(NS);
  const queryClient = useQueryClient();
  const zone = useAccountTimeZone();
  const today = useAccountToday();
  const [scope, setScope] = useState<Scope>("range");
  const [from, setFrom] = useState(today);
  const [to, setTo] = useState(today);
  const [footprints, setFootprints] = useState(false);
  const everything = !date && scope === "all";
  const start = dayRange(date ?? from, zone)?.from;
  const end = dayRange(date ?? to, zone)?.to;
  // What goes: everything only when that was chosen, else the days picked.
  // Days that cannot be read delete nothing — never everything instead.
  const range = everything
    ? {}
    : start && end && start < end
      ? { from: fromDate(start), to: fromDate(end) }
      : null;

  const remove = useMutation({
    mutationFn: async () =>
      range
        ? asAccount(() =>
            historyClient().deleteHistory({ ...range, includeFootprints: footprints }),
          )
        : undefined,
    onSuccess: (answer) => {
      if (!answer) return;
      for (const queryKey of DELETED_HISTORY_KEYS) void queryClient.resetQueries({ queryKey });
      if (date) {
        // The day on screen is empty now; the re-read adds back what remains (a memo stays).
        queryClient.setQueryData(historyKeys.day(date, zone), create(geo.GetDayResponseSchema, {}));
        queryClient.setQueryData(
          historyKeys.route(date, zone),
          create(geo.GetRouteResponseSchema, {}),
        );
      }
      Alert.alert(t("delete.done"));
    },
    onError: (error) => Alert.alert(errorMessage(error, { namespace: NS })),
  });

  return (
    <Page testID="history-delete-screen">
      <Stack.Screen options={{ title: date ? t("delete.dayTitle") : t("delete.title") }} />
      {date ? (
        <Text className="text-lg font-black text-ink">{dayTitle(date, i18n.language, true)}</Text>
      ) : (
        <>
          <Tabs<Scope>
            testID="history-delete-scope"
            items={[
              { value: "range", label: t("delete.range") },
              { value: "all", label: t("delete.all") },
            ]}
            value={scope}
            onChange={setScope}
          />
          {scope === "range" && (
            <View className="gap-2">
              <DayStepper
                testID="history-delete-from"
                label={t("delete.from")}
                value={from}
                max={today}
                onChange={setFrom}
              />
              <DayStepper
                testID="history-delete-to"
                label={t("delete.to")}
                value={to}
                max={today}
                onChange={setTo}
              />
              {!range && <ErrorNote>{t("delete.invalidRange")}</ErrorNote>}
            </View>
          )}
        </>
      )}
      <Text className="text-sm text-ink-soft">
        {everything ? t("delete.allBody") : t("delete.body")}
      </Text>
      <Section>
        <SwitchRow
          testID="history-delete-footprints"
          label={t("delete.footprints")}
          hint={t("delete.footprintsHint")}
          value={footprints}
          onChange={setFootprints}
        />
      </Section>
      <Text className="rounded-2xl bg-surface-2 px-4 py-3 text-sm font-bold text-ink-soft">
        ⚠️ {t("delete.warning")}
      </Text>
      <Button
        testID="history-delete-confirm"
        variant="danger"
        block
        size="lg"
        // Once it went through there is nothing left to send again.
        disabled={!range || remove.isSuccess}
        loading={remove.isPending}
        // Closing is this screen's own: when the person already left it, the
        // answer forgets the cache all the same but moves nothing under them.
        onPress={() => remove.mutate(undefined, { onSuccess: (answer) => answer && router.back() })}
      >
        {everything ? t("delete.confirmAll") : t("delete.confirm")}
      </Button>
    </Page>
  );
}

/** A day picked by stepping a day at a time, never past `max`. */
function DayStepper({
  label,
  value,
  max,
  onChange,
  testID,
}: {
  label: string;
  value: string;
  max: string;
  onChange(date: string): void;
  testID: string;
}) {
  const { i18n } = useTranslation();
  const { colors } = useTheme();
  const atMax = value >= max;
  return (
    <View className="flex-row items-center gap-2 rounded-card bg-surface px-3 py-2">
      <Text className="w-20 text-sm font-bold text-ink-soft">{label}</Text>
      <IconButton
        testID={`${testID}-previous`}
        label={`${label} −1`}
        icon={<ChevronLeft size={20} color={colors.ink} />}
        onPress={() => onChange(shiftDay(value, -1))}
      />
      <Text testID={`${testID}-value`} className="flex-1 text-center font-black text-ink">
        {dayTitle(value, i18n.language, true)}
      </Text>
      <View className={atMax ? "opacity-30" : ""}>
        <IconButton
          testID={`${testID}-next`}
          label={`${label} +1`}
          icon={<ChevronRight size={20} color={colors.ink} />}
          onPress={() => !atMax && onChange(shiftDay(value, 1))}
        />
      </View>
    </View>
  );
}
