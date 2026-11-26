import { useMutation, useQueryClient } from "@tanstack/react-query";
import { fromDate } from "@tokuly/api";
import { zonedMoment } from "@tokuly/core/format";
import { addDays, refreshSchool } from "@tokuly/core/school";
import { router } from "expo-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { Button } from "#/components/button";
import { SwitchRow } from "#/components/form-rows";
import { Page, Section } from "#/components/page";
import { TextField } from "#/components/text-field";
import { errorMessage } from "#/lib/errors";
import { sticker, useTheme } from "#/lib/theme";
import { asAccount, assignmentClient, NS, useAccountNow } from "./data";

/** How many days from the account's today an assignment is due, by the chip that says so. */
const DUE_DAYS = { today: 0, tomorrow: 1, dayAfter: 2, nextWeek: 7 } as const;
type DueDay = keyof typeof DUE_DAYS;

/**
 * `/school/new-assignment`: what to do and by which day (23:59 of it, on the
 * account's calendar), reminded unless switched off. The subject, a time of
 * day and a memo are set on the web.
 */
export function NewAssignmentScreen() {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const { date: today, zone } = useAccountNow(60_000);
  const [title, setTitle] = useState("");
  const [due, setDue] = useState<DueDay>("tomorrow");
  const [remind, setRemind] = useState(true);
  const save = useMutation({
    mutationFn: () => {
      const dueAt = new Date(zonedMoment(addDays(today, DUE_DAYS[due]), 23 * 60 + 59, zone));
      return asAccount(() =>
        assignmentClient().createAssignment({
          title: title.trim(),
          dueAt: fromDate(dueAt),
          remind,
        }),
      );
    },
    onSuccess: (answer) => {
      if (!answer) return;
      refreshSchool(queryClient, "assignments");
      router.back();
    },
    onError: (error) => Alert.alert(errorMessage(error, { namespace: NS })),
  });
  return (
    <Page testID="new-assignment">
      <TextField
        testID="assignment-title"
        label={t("assignments.titleField")}
        placeholder={t("assignments.titlePlaceholder")}
        value={title}
        maxLength={80}
        onChangeText={setTitle}
        autoFocus
      />
      <View className="gap-1.5">
        <Text className="px-1 text-sm font-bold text-ink-soft">{t("mobile:school.dueOn")}</Text>
        <View className="flex-row flex-wrap gap-2">
          {(Object.keys(DUE_DAYS) as DueDay[]).map((key) => {
            const chosen = key === due;
            return (
              <Pressable
                key={key}
                testID={`assignment-due-${key}`}
                accessibilityRole="radio"
                accessibilityState={{ selected: chosen }}
                onPress={() => setDue(key)}
                className={`h-10 justify-center rounded-full px-4 ${chosen ? "bg-ink" : "bg-surface"}`}
                style={chosen ? sticker(colors, "sm") : undefined}
              >
                <Text className={`font-black ${chosen ? "text-surface" : "text-ink"}`}>
                  {t(`mobile:school.dueDays.${key}`)}
                </Text>
              </Pressable>
            );
          })}
        </View>
        <Text className="px-1 text-xs text-ink-faint">{t("mobile:school.dueHint")}</Text>
      </View>
      <Section>
        <SwitchRow
          testID="assignment-remind"
          label={t("assignments.remind")}
          hint={t("assignments.remindHint")}
          value={remind}
          onChange={setRemind}
        />
      </Section>
      <Button
        testID="assignment-save"
        block
        size="lg"
        loading={save.isPending}
        disabled={title.trim() === ""}
        onPress={() => save.mutate()}
      >
        {t("assignments.addAction")}
      </Button>
    </Page>
  );
}
