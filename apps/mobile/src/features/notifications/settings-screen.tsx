import { useMutation, useQueryClient } from "@tanstack/react-query";
import { identity, notify, toMillis } from "@tokuly/api";
import { minutesToTime, notifyKeys } from "@tokuly/core/notifications";
import { getCalendars } from "expo-localization";
import { router } from "expo-router";
import { BellOff, Check } from "lucide-react-native";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Switch, Text, View } from "react-native";
import { Button } from "#/components/button";
import { ChoiceRow, SwitchRow } from "#/components/form-rows";
import { Page, Row, Section, Spinner } from "#/components/page";
import { currentAccount } from "#/features/session/account";
import { webScreenHref } from "#/features/web/handoff";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import { quietHoursOf, usePreferenceActions, usePreferences } from "./preferences";
import { PushCard } from "./push-card";

/**
 * Notification settings, as on the web: push on this device, what a push
 * shows on the lock screen, the pause, quiet hours and the categories. The
 * muted people open on the web.
 */
export function NotificationSettingsScreen() {
  const { t } = useTranslation("notifications");
  const { colors } = useTheme();
  const preferences = usePreferences();
  return (
    <Page testID="notification-settings">
      <PushCard />
      {preferences.data ? (
        <>
          <PreviewSection preferences={preferences.data} />
          <PauseSection preferences={preferences.data} />
          <QuietHoursSection preferences={preferences.data} />
          <CategoriesSection preferences={preferences.data} />
        </>
      ) : (
        <Spinner />
      )}
      <Section>
        <Row
          testID="notification-settings-muted"
          icon={<BellOff size={20} color={colors.ink} />}
          label={t("muted.title")}
          onPress={() => router.push(webScreenHref("/notifications/settings") as never)}
        />
      </Section>
    </Page>
  );
}

/** The three choices, most shown first. */
const PREVIEWS = [
  { value: notify.PushPreview.FULL, key: "full" },
  { value: notify.PushPreview.SENDER_ONLY, key: "senderOnly" },
  { value: notify.PushPreview.HIDDEN, key: "hidden" },
] as const;

/**
 * 「通知に内容を表示」: what a push shows on the lock screen, each choice with
 * a sample of how a message would look. What a choice leaves out never
 * reaches the device; the inbox keeps everything.
 */
function PreviewSection({ preferences }: { preferences: notify.Preferences }) {
  const { t } = useTranslation("notifications");
  const { colors } = useTheme();
  const actions = usePreferenceActions();
  const current = preferences.pushPreview || notify.PushPreview.FULL;
  return (
    <Section title={t("preview.title")} lead={t("preview.lead")}>
      <View className="gap-2 p-3" accessibilityRole="radiogroup">
        {PREVIEWS.map((choice) => {
          const selected = current === choice.value;
          return (
            <Pressable
              key={choice.key}
              testID={`push-preview-${choice.key}`}
              accessibilityRole="radio"
              accessibilityState={{ selected }}
              onPress={() => !selected && void actions.setPushPreview(choice.value)}
              className={`flex-row gap-3 rounded-2xl border-2 p-3 ${selected ? "border-ink bg-brand-soft" : "border-line bg-surface-2"}`}
            >
              <View className="flex-1 gap-1">
                <Text className="font-black text-ink">
                  {t(`preview.choices.${choice.key}.label`)}
                </Text>
                <Text className="text-xs text-ink-soft">
                  {t(`preview.choices.${choice.key}.hint`)}
                </Text>
                <View
                  importantForAccessibility="no-hide-descendants"
                  className="mt-1 rounded-xl bg-surface px-3 py-2"
                >
                  <Text className="text-xs font-bold text-ink">
                    {t(`preview.choices.${choice.key}.sampleTitle`)}
                  </Text>
                  <Text className="text-xs text-ink-soft">
                    {t(`preview.choices.${choice.key}.sampleBody`)}
                  </Text>
                </View>
              </View>
              {selected && <Check size={20} color={colors.brand} />}
            </Pressable>
          );
        })}
        <Text className="px-1 text-xs text-ink-soft">{t("preview.inbox")}</Text>
      </View>
    </Section>
  );
}

/** How long a pause can be asked for, in seconds; 0 = until resumed. */
const DURATIONS = [
  { seconds: 30 * 60, key: "30m" },
  { seconds: 60 * 60, key: "1h" },
  { seconds: 8 * 60 * 60, key: "8h" },
  { seconds: 24 * 60 * 60, key: "24h" },
  { seconds: 0, key: "manual" },
] as const;

/** Pause every push for a while (the inbox keeps filling), or resume. */
function PauseSection({ preferences }: { preferences: notify.Preferences }) {
  const { t, i18n } = useTranslation("notifications");
  const actions = usePreferenceActions();
  const now = useNow(15_000);
  const [busy, setBusy] = useState(false);
  const until = toMillis(preferences.pushPause?.until);
  // A timed pause ends by itself; the next fetch need not say so first.
  const paused = Boolean(preferences.pushPause?.paused) && (until === 0 || until > now);
  const run = async (work: Promise<boolean>, done: string) => {
    setBusy(true);
    if (await work) Alert.alert(done);
    setBusy(false);
  };
  return (
    <Section title={t("pause.title")}>
      <View className="gap-3 p-4">
        <Text accessibilityLiveRegion="polite" className="text-sm text-ink-soft">
          {paused
            ? until
              ? t("pause.until", { when: formatUntil(until, now, i18n.language) })
              : t("pause.untilResumed")
            : t("pause.lead")}
        </Text>
        {paused ? (
          <Button
            testID="push-resume"
            size="sm"
            variant="lime"
            disabled={busy}
            onPress={() => void run(actions.resume(), t("pause.toast.resumed"))}
          >
            {t("pause.resume")}
          </Button>
        ) : (
          <View className="flex-row flex-wrap gap-2">
            {DURATIONS.map((duration) => (
              <Button
                key={duration.key}
                testID={`push-pause-${duration.key}`}
                size="sm"
                variant="secondary"
                disabled={busy}
                onPress={() => void run(actions.pause(duration.seconds), t("pause.toast.paused"))}
              >
                {t(`pause.durations.${duration.key}`)}
              </Button>
            ))}
          </View>
        )}
      </View>
    </Section>
  );
}

/** Every half hour of the day, in minutes. */
const HALF_HOURS = Array.from({ length: 48 }, (_, index) => index * 30);

/** Quiet hours: a nightly window without pushes, in the account's time zone. */
function QuietHoursSection({ preferences }: { preferences: notify.Preferences }) {
  const { t } = useTranslation("notifications");
  const actions = usePreferenceActions();
  const quiet = quietHoursOf(preferences);
  // The other edge is not offered: the window needs two different times.
  const times = (other: number, own: number) =>
    [...new Set([...HALF_HOURS, own])]
      .filter((minute) => minute !== other)
      .sort((a, b) => a - b)
      .map((minute) => ({ value: String(minute), label: minutesToTime(minute) }));
  return (
    <Section title={t("quiet.title")} lead={t("quiet.lead")}>
      <SwitchRow
        testID="quiet-hours"
        label={preferences.quietNow ? `${t("quiet.title")} · ${t("quiet.now")}` : t("quiet.title")}
        value={quiet.enabled}
        onChange={(enabled) => void actions.setQuietHours({ enabled })}
      />
      <ChoiceRow
        testID="quiet-start"
        label={t("quiet.start")}
        value={String(quiet.startMinute)}
        choices={times(quiet.endMinute, quiet.startMinute)}
        onChange={(minute) => void actions.setQuietHours({ startMinute: Number(minute) })}
      />
      <ChoiceRow
        testID="quiet-end"
        label={t("quiet.end")}
        hint={quiet.endMinute < quiet.startMinute ? t("quiet.nextDay") : undefined}
        value={String(quiet.endMinute)}
        choices={times(quiet.startMinute, quiet.endMinute)}
        onChange={(minute) => void actions.setQuietHours({ endMinute: Number(minute) })}
      />
      <TimeZone accountZone={preferences.timeZone} />
    </Section>
  );
}

/**
 * Quiet hours follow the account's time zone. When this phone is somewhere
 * else (a trip, a move), it offers to bring the account along.
 */
function TimeZone({ accountZone }: { accountZone: string }) {
  const { t } = useTranslation("notifications");
  const query = useQueryClient();
  const deviceZone = getCalendars()[0]?.timeZone ?? "";
  const adopt = useMutation({
    mutationFn: async () => {
      const account = currentAccount();
      if (!account) return;
      await account.api.service(identity.AccountService).updateProfile({ timeZone: deviceZone });
    },
    onSuccess: () => {
      Alert.alert(t("quiet.zoneChanged", { zone: deviceZone }));
      return query.invalidateQueries({ queryKey: notifyKeys.preferences });
    },
    onError: (error) => Alert.alert(errorMessage(error, { namespace: "notifications" })),
  });
  const differs = Boolean(deviceZone) && deviceZone !== accountZone;
  return (
    <View className="flex-row items-center gap-3 px-4 py-3">
      <View className="flex-1">
        <Text className="text-sm font-bold text-ink">{t("quiet.zone", { zone: accountZone })}</Text>
        {differs && (
          <Text className="text-xs text-ink-soft">
            {t("quiet.zoneDiffers", { zone: deviceZone })}
          </Text>
        )}
      </View>
      {differs && (
        <Button
          size="sm"
          variant="secondary"
          disabled={adopt.isPending}
          onPress={() => adopt.mutate()}
        >
          {t("quiet.zoneAdopt")}
        </Button>
      )}
    </View>
  );
}

/** Per category: keep it in the inbox, and/or push it. */
function CategoriesSection({ preferences }: { preferences: notify.Preferences }) {
  const { t } = useTranslation("notifications");
  const { colors } = useTheme();
  const actions = usePreferenceActions();
  const track = { true: colors.brand, false: colors["surface-3"] };
  return (
    <Section title={t("categories.title")} lead={t("categories.lead")}>
      <View className="flex-row justify-end gap-2 px-4 pt-3" importantForAccessibility="no">
        <Text className="w-14 text-center text-[11px] font-black text-ink-soft">
          {t("categories.inbox")}
        </Text>
        <Text className="w-14 text-center text-[11px] font-black text-ink-soft">
          {t("categories.push")}
        </Text>
      </View>
      {preferences.categories.map((category) => (
        <View
          key={category.category}
          className="flex-row items-center gap-2 border-b border-line px-4 py-3"
        >
          <View className="flex-1">
            <Text className="font-bold text-ink">{category.label}</Text>
            <Text className="text-xs text-ink-soft">{category.description}</Text>
            {category.inboxLocked && (
              <Text className="text-xs font-bold text-ink-faint">{t("categories.locked")}</Text>
            )}
          </View>
          <View className="w-14 items-center">
            <Switch
              testID={`category-${category.category}-inbox`}
              accessibilityLabel={t("categories.inboxOf", { category: category.label })}
              value={category.inbox}
              disabled={category.inboxLocked}
              trackColor={track}
              thumbColor={colors.surface}
              onValueChange={(inbox) => void actions.setCategory(category.category, { inbox })}
            />
          </View>
          <View className="w-14 items-center">
            <Switch
              testID={`category-${category.category}-push`}
              accessibilityLabel={t("categories.pushOf", { category: category.label })}
              value={category.push}
              trackColor={track}
              thumbColor={colors.surface}
              onValueChange={(push) => void actions.setCategory(category.category, { push })}
            />
          </View>
        </View>
      ))}
    </Section>
  );
}

/** The time now, read again every `everyMs`. */
function useNow(everyMs: number): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), everyMs);
    return () => clearInterval(timer);
  }, [everyMs]);
  return now;
}

/** "15:30" today, "11月23日 15:30" otherwise. */
function formatUntil(untilMs: number, nowMs: number, language: string): string {
  const until = new Date(untilMs);
  const sameDay = until.toDateString() === new Date(nowMs).toDateString();
  return sameDay
    ? until.toLocaleTimeString(language, { hour: "2-digit", minute: "2-digit" })
    : until.toLocaleString(language, {
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      });
}
